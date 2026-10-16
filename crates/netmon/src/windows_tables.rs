//! Windows collector: iphlpapi extended TCP/UDP tables (TCP_TABLE_OWNER_PID_ALL)
//! and Toolhelp32 process enumeration. Same data source as TCPView.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, GetExtendedUdpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID,
    MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID, MIB_UDP6ROW_OWNER_PID, MIB_UDP6TABLE_OWNER_PID,
    MIB_UDPROW_OWNER_PID, MIB_UDPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_ALL, UDP_TABLE_OWNER_PID,
};
use windows::Win32::Networking::WinSock::{ADDRESS_FAMILY, AF_INET, AF_INET6};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

use crate::collector::Collector;
use crate::{Connection, ProcessInfo, Protocol, TcpState};

#[derive(Default)]
pub struct WindowsCollector;

/// Port fields are stored in network byte order in the low 16 bits.
#[inline]
fn port_from_raw(raw: u32) -> u16 {
    u16::from_be(raw as u16)
}

/// IPv4 address fields are raw network-order bytes read as a little-endian u32.
#[inline]
fn ipv4_from_raw(raw: u32) -> Ipv4Addr {
    Ipv4Addr::from(raw.to_le_bytes())
}

impl Collector for WindowsCollector {
    fn snapshot(&self) -> Vec<Connection> {
        let mut out = Vec::new();
        unsafe {
            out.extend(tcp_table(AF_INET, Protocol::Tcp4));
            out.extend(tcp_table(AF_INET6, Protocol::Tcp6));
            out.extend(udp_table(AF_INET, Protocol::Udp4));
            out.extend(udp_table(AF_INET6, Protocol::Udp6));
        }
        out
    }

    fn processes(&self) -> Vec<ProcessInfo> {
        let mut out = Vec::new();
        unsafe {
            let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
                return out;
            };
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            if Process32FirstW(snapshot, &mut entry).is_ok() {
                loop {
                    let len = entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    out.push(ProcessInfo {
                        pid: entry.th32ProcessID,
                        name: String::from_utf16_lossy(&entry.szExeFile[..len]),
                        exe: None,
                    });
                    if Process32NextW(snapshot, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snapshot);
        }
        out
    }
}

unsafe fn grow_buffer<F: FnMut(*mut u8, &mut u32) -> bool>(fill: &mut F) -> Vec<u8> {
    let mut size: u32 = 0;
    fill(std::ptr::null_mut(), &mut size);
    if size == 0 {
        size = 1024;
    }
    for _ in 0..8 {
        let mut buf = vec![0u8; size as usize];
        let mut needed = size;
        if fill(buf.as_mut_ptr(), &mut needed) {
            return buf;
        }
        if needed > size {
            size = needed;
        } else {
            size *= 2;
        }
    }
    Vec::new()
}

unsafe fn tcp_table(af: ADDRESS_FAMILY, proto: Protocol) -> Vec<Connection> {
    let family = af.0 as u32;
    let buf = grow_buffer(&mut |ptr, size| {
        GetExtendedTcpTable(
            Some(ptr.cast()), // ИСПРАВЛЕНИЕ 1: Оборачиваем в Some()
            size,
            false,
            family,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        ) == windows::Win32::Foundation::NO_ERROR.0
    });
    if buf.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    if af == AF_INET {
        let table = &*(buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID);
        let rows = std::slice::from_raw_parts(table.table.as_ptr(), table.dwNumEntries as usize);
        for row in rows {
            let row: &MIB_TCPROW_OWNER_PID = row;
            out.push(Connection {
                proto,
                local: SocketAddr::V4(SocketAddrV4::new(
                    ipv4_from_raw(row.dwLocalAddr),
                    port_from_raw(row.dwLocalPort),
                )),
                remote: Some(SocketAddr::V4(SocketAddrV4::new(
                    ipv4_from_raw(row.dwRemoteAddr),
                    port_from_raw(row.dwRemotePort),
                ))),
                state: TcpState::from_raw(row.dwState as u32), // ИСПРАВЛЕНИЕ 2: Убрано .0
                pid: row.dwOwningPid,
            });
        }
    } else {
        let table = &*(buf.as_ptr() as *const MIB_TCP6TABLE_OWNER_PID);
        let rows = std::slice::from_raw_parts(table.table.as_ptr(), table.dwNumEntries as usize);
        for row in rows {
            let row: &MIB_TCP6ROW_OWNER_PID = row;
            out.push(Connection {
                proto,
                local: SocketAddr::V6(SocketAddrV6::new(
                    Ipv6Addr::from(row.ucLocalAddr),
                    port_from_raw(row.dwLocalPort),
                    0, // ИСПРАВЛЕНИЕ 3: Добавлен flowinfo
                    row.dwLocalScopeId,
                )),
                remote: Some(SocketAddr::V6(SocketAddrV6::new(
                    Ipv6Addr::from(row.ucRemoteAddr),
                    port_from_raw(row.dwRemotePort),
                    0, // ИСПРАВЛЕНИЕ 3: Добавлен flowinfo
                    row.dwRemoteScopeId,
                ))),
                state: TcpState::from_raw(row.dwState as u32), // ИСПРАВЛЕНИЕ 2: Убрано .0
                pid: row.dwOwningPid,
            });
        }
    }
    out
}

unsafe fn udp_table(af: ADDRESS_FAMILY, proto: Protocol) -> Vec<Connection> {
    let family = af.0 as u32;
    let buf = grow_buffer(&mut |ptr, size| {
        GetExtendedUdpTable(
            Some(ptr.cast()), // ИСПРАВЛЕНИЕ 1: Оборачиваем в Some()
            size,
            false,
            family,
            UDP_TABLE_OWNER_PID,
            0,
        ) == windows::Win32::Foundation::NO_ERROR.0
    });
    if buf.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    if af == AF_INET {
        let table = &*(buf.as_ptr() as *const MIB_UDPTABLE_OWNER_PID);
        let rows = std::slice::from_raw_parts(table.table.as_ptr(), table.dwNumEntries as usize);
        for row in rows {
            let row: &MIB_UDPROW_OWNER_PID = row;
            out.push(Connection {
                proto,
                local: SocketAddr::V4(SocketAddrV4::new(
                    ipv4_from_raw(row.dwLocalAddr),
                    port_from_raw(row.dwLocalPort),
                )),
                remote: None,
                state: None,
                pid: row.dwOwningPid,
            });
        }
    } else {
        let table = &*(buf.as_ptr() as *const MIB_UDP6TABLE_OWNER_PID);
        let rows = std::slice::from_raw_parts(table.table.as_ptr(), table.dwNumEntries as usize);
        for row in rows {
            let row: &MIB_UDP6ROW_OWNER_PID = row;
            out.push(Connection {
                proto,
                local: SocketAddr::V6(SocketAddrV6::new(
                    Ipv6Addr::from(row.ucLocalAddr),
                    port_from_raw(row.dwLocalPort),
                    0, // ИСПРАВЛЕНИЕ 3: Добавлен flowinfo
                    row.dwLocalScopeId,
                )),
                remote: None,
                state: None,
                pid: row.dwOwningPid,
            });
        }
    }
    out
}