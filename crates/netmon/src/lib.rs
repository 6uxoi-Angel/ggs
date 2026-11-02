//! integra-netmon — "where does this app connect to?"
//!
//! A polling tracker that snapshots the OS connection tables, filters them by
//! process (name substring or explicit PIDs, auto-including processes started
//! *after* the session began — e.g. launch the tracker, then start Apex), and
//! accumulates a log of unique connections with first/last-seen timestamps,
//! reverse-DNS hostnames and well-known port labels.
//!
//! On Windows the tables come from `GetExtendedTcpTable` / `GetExtendedUdpTable`
//! (iphlpapi, same source as TCPView). On Linux `/proc/net/{tcp,udp}{,6}` is
//! parsed — that keeps the crate testable in any dev environment.

pub mod collector;
pub mod services;
pub mod tracker;

#[cfg(windows)]
pub mod windows_tables;
#[cfg(target_os = "linux")]
pub mod procfs;

use std::net::{IpAddr, SocketAddr};

use chrono::{DateTime, Local};
use serde::Serialize;
use thiserror::Error;

pub use collector::Collector;
pub use tracker::{Netmon, ProcessFilter, SessionSummary, TrackerConfig};

#[derive(Debug, Error)]
pub enum NetmonError {
    #[error("netmon.already_running")]
    AlreadyRunning,
    #[error("netmon.not_running")]
    NotRunning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Tcp4,
    Tcp6,
    Udp4,
    Udp6,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TcpState {
    Closed,
    Listen,
    SynSent,
    SynRecv,
    Estab,
    FinWait1,
    FinWait2,
    CloseWait,
    Closing,
    LastAck,
    TimeWait,
    DeleteTcb,
}

impl TcpState {
    /// MIB_TCP_STATE numeric values (Windows) / TCP states (Linux, same order).
    pub fn from_raw(raw: u32) -> Option<TcpState> {
        Some(match raw {
            1 => TcpState::Closed,
            2 => TcpState::Listen,
            3 => TcpState::SynSent,
            4 => TcpState::SynRecv,
            5 => TcpState::Estab,
            6 => TcpState::FinWait1,
            7 => TcpState::FinWait2,
            8 => TcpState::CloseWait,
            9 => TcpState::Closing,
            10 => TcpState::LastAck,
            11 => TcpState::TimeWait,
            12 => TcpState::DeleteTcb,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Connection {
    pub proto: Protocol,
    pub local: SocketAddr,
    pub remote: Option<SocketAddr>,
    pub state: Option<TcpState>,
    pub pid: u32,
}

impl Connection {
    /// Key identifying a unique logical connection for dedup.
    pub fn key(&self) -> ConnectionKey {
        ConnectionKey {
            proto: self.proto,
            pid: self.pid,
            local: self.local,
            remote: self.remote,
        }
    }

    pub fn remote_ip(&self) -> Option<IpAddr> {
        self.remote.map(|addr| addr.ip())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionKey {
    pub proto: Protocol,
    pub pid: u32,
    pub local: SocketAddr,
    pub remote: Option<SocketAddr>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub exe: Option<String>,
}

/// One unique observed connection, enriched with process/host metadata.
#[derive(Debug, Clone, Serialize)]
pub struct TrackedEntry {
    pub proto: Protocol,
    pub pid: u32,
    pub process: String,
    pub exe: Option<String>,
    pub local: String,
    pub remote: Option<String>,
    pub remote_ip: Option<String>,
    pub remote_port: Option<u16>,
    pub host: Option<String>,
    pub service: Option<String>,
    pub state: Option<TcpState>,
    pub first_seen: DateTime<Local>,
    pub last_seen: DateTime<Local>,
    pub hits: u32,
}
