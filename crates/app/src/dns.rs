//! Roblox DNS Manager integrated into Integra.
//!
//! * DNS resolver benchmark: TCP RTT + real UDP DNS queries per resolver,
//!   scored by latency and resolution quality (including tr.rbxcdn.com).
//! * Apply / reset DNS on all active adapters (PowerShell, DoH-capable).
//! * Roblox image fix: find the fastest IP for the Roblox CDN and pin it
//!   into the hosts file (backup + managed block).

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};

use crate::CommandError;

pub const HOSTS_PATH: &str = r"C:\Windows\System32\drivers\etc\hosts";
const HOSTS_MARK_BEGIN: &str = "# >>> Integra Roblox DNS Manager";
const HOSTS_MARK_END: &str = "# <<< Integra Roblox DNS Manager";

/// Resolver catalog used by the benchmark and by the apply shortcuts.
#[derive(Clone, Serialize)]
pub struct DnsServer {
    pub id: &'static str,
    pub name: &'static str,
    pub primary: &'static str,
    pub secondary: &'static str,
    pub doh_template: &'static str,
}

pub const DNS_SERVERS: &[DnsServer] = &[
    DnsServer {
        id: "quad9",
        name: "Quad9",
        primary: "9.9.9.9",
        secondary: "149.112.112.112",
        doh_template: "https://dns.quad9.net/dns-query",
    },
    DnsServer {
        id: "cloudflare",
        name: "Cloudflare",
        primary: "1.1.1.1",
        secondary: "1.0.0.1",
        doh_template: "https://cloudflare-dns.com/dns-query",
    },
    DnsServer {
        id: "google",
        name: "Google Public DNS",
        primary: "8.8.8.8",
        secondary: "8.8.4.4",
        doh_template: "https://dns.google/dns-query",
    },
    DnsServer {
        id: "adguard",
        name: "AdGuard DNS",
        primary: "94.140.14.14",
        secondary: "94.140.15.15",
        doh_template: "https://dns.adguard-dns.com/dns-query",
    },
    DnsServer {
        id: "yandex",
        name: "Yandex DNS",
        primary: "77.88.8.8",
        secondary: "77.88.8.1",
        doh_template: "https://common.dot.yandex.net/dns-query",
    },
    DnsServer {
        id: "comss",
        name: "Comss.one (RU)",
        primary: "83.220.223.55",
        secondary: "83.220.222.55",
        doh_template: "",
    },
];

/// Upper bound for one PowerShell invocation: the scripts here are tiny, so
/// a hung child must never wedge the caller forever.
const PS_TIMEOUT: Duration = Duration::from_secs(20);

fn ps(script: &str) -> Result<String, CommandError> {
    let mut child = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| CommandError::raw(e.to_string()))?;
    // Bounded wait: poll for exit, kill when the deadline passes.
    let deadline = Instant::now() + PS_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(CommandError::raw("powershell timed out"));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(CommandError::raw(e.to_string())),
        }
    };
    // The child is gone, so the pipes are at EOF and drain without blocking.
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    // Output is parsed only after a verified successful exit.
    if status.success() {
        return Ok(stdout);
    }
    let low = stderr.to_lowercase();
    if low.contains("denied") || low.contains("доступ") || low.contains("(5)") {
        return Err(CommandError::key("dns.not_admin"));
    }
    Err(CommandError::raw(format!(
        "powershell failed: {}",
        stderr.trim()
    )))
}

fn is_admin() -> bool {
    std::process::Command::new("net")
        .args(["session"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ── DNS wire protocol (A query over UDP) ────────────────────────────

/// Per-query pseudo-random transaction ID: xorshift64* seeded from the
/// system clock nanos ^ process id, mixed with an incrementing counter. The
/// old `Instant::now() & 0xFFFF` derivation was predictable and colliding.
fn next_dns_id() -> u16 {
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
    static STATE: AtomicU64 = AtomicU64::new(0);
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let mut state = STATE.load(Ordering::Relaxed);
    if state == 0 {
        state = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
            ^ ((std::process::id() as u64) << 32)
            ^ 0x9E37_79B9_7F4A_7C15;
        if state == 0 {
            state = 0x9E37_79B9_7F4A_7C15;
        }
    }
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    STATE.store(state, Ordering::Relaxed);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    ((state >> 32) as u16) ^ (state as u16) ^ count as u16
}

/// Validate a datagram as the answer to OUR query: same transaction id, QR
/// bit set (it is a response, not a request), at least one question, and the
/// question section's qname bytes equal to ours (DNS names are
/// case-insensitive, so compare ASCII-case-insensitively). Everything else —
/// spoofed interceptor replies, late answers, garbage — is discarded.
fn response_matches(query: &[u8], resp: &[u8]) -> bool {
    if resp.len() < 12 || query.len() < 17 {
        return false;
    }
    if resp[0] != query[0] || resp[1] != query[1] {
        return false;
    }
    if resp[2] & 0x80 == 0 {
        return false;
    }
    if u16::from_be_bytes([resp[4], resp[5]]) == 0 {
        return false;
    }
    // Query layout: 12-byte header + qname (labels + root) + qtype/qclass,
    // so the qname occupies query[12..len-16]. A matching response echoes it
    // uncompressed at the same offset.
    let qlen = query.len() - 16;
    if resp.len() < 12 + qlen {
        return false;
    }
    let (want, got) = (&query[12..12 + qlen], &resp[12..12 + qlen]);
    want.iter()
        .zip(got.iter())
        .all(|(a, b)| a.to_ascii_lowercase() == b.to_ascii_lowercase())
}

fn build_a_query(id: u16, domain: &str) -> Vec<u8> {
    let mut p = Vec::with_capacity(64);
    p.extend_from_slice(&id.to_be_bytes());
    p.extend_from_slice(&[0x01, 0x00]); // RD
    p.extend_from_slice(&[0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    for label in domain.split('.') {
        p.push(label.len() as u8);
        p.extend_from_slice(label.as_bytes());
    }
    p.push(0);
    p.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]); // A IN
    p
}

fn skip_name(buf: &[u8], mut off: usize) -> Option<usize> {
    loop {
        let len = *buf.get(off)?;
        match len {
            0 => return Some(off + 1),
            n if n & 0xC0 == 0xC0 => return Some(off + 2),
            n => off += 1 + n as usize,
        }
    }
}

struct DnsAnswer {
    addresses: Vec<Ipv4Addr>,
    cname: Option<String>,
}

/// Decode a possibly-compressed name inside `buf`, starting at `off`.
fn decode_name(buf: &[u8], mut off: usize) -> Option<(String, usize)> {
    let mut out = String::new();
    let mut jumps = 0;
    loop {
        let len = *buf.get(off)?;
        if len & 0xC0 == 0xC0 {
            let ptr = (len as usize & 0x3F) << 8 | *buf.get(off + 1)? as usize;
            jumps += 1;
            if jumps > 10 {
                return None;
            }
            off = ptr;
            continue;
        }
        if len == 0 {
            break;
        }
        off += 1;
        let l = len as usize;
        if off + l > buf.len() {
            return None;
        }
        if !out.is_empty() {
            out.push('.');
        }
        out.push_str(&String::from_utf8_lossy(&buf[off..off + l]));
        off += l;
    }
    Some((out, off))
}

fn parse_a_response(buf: &[u8], qname: &str) -> Option<DnsAnswer> {
    if buf.len() < 12 {
        return None;
    }
    let ancount = u16::from_be_bytes([buf[6], buf[7]]) as usize;
    let mut off = 12usize;
    for label in qname.split('.') {
        let len = *buf.get(off)? as usize;
        if len == 0 || len != label.len() {
            return None;
        }
        off += 1 + len;
    }
    off += 5; // root + qtype + qclass
    let mut addresses = Vec::new();
    let mut cname = None;
    for _ in 0..ancount {
        off = skip_name(buf, off)?;
        if off + 10 > buf.len() {
            return None;
        }
        let qtype = u16::from_be_bytes([buf[off], buf[off + 1]]);
        let rdlen = u16::from_be_bytes([buf[off + 8], buf[off + 9]]) as usize;
        off += 10;
        if off + rdlen > buf.len() {
            return None;
        }
        if qtype == 1 && rdlen == 4 {
            addresses.push(Ipv4Addr::new(
                buf[off],
                buf[off + 1],
                buf[off + 2],
                buf[off + 3],
            ));
        } else if qtype == 5 && cname.is_none() {
            if let Some((name, _)) = decode_name(buf, off) {
                cname = Some(name);
            }
        }
        off += rdlen;
    }
    Some(DnsAnswer { addresses, cname })
}

/// One query exchange: UDP first, TCP fallback (plain UDP :53 is often
/// filtered by ISPs, while TCP DNS usually passes).
fn raw_query(server: &str, q: &[u8], timeout: Duration) -> Option<Vec<u8>> {
    if let Ok(sock) = UdpSocket::bind("0.0.0.0:0") {
        if sock.set_read_timeout(Some(timeout)).is_ok() {
            if let Ok(addr) = format!("{server}:53").parse::<SocketAddr>() {
                if sock.connect(addr).is_ok() && sock.send(q).is_ok() {
                    let deadline = Instant::now() + timeout;
                    let mut buf = [0u8; 512];
                    loop {
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        if remaining.is_zero() || sock.set_read_timeout(Some(remaining)).is_err() {
                            break;
                        }
                        match sock.recv(&mut buf) {
                            Ok(n) => {
                                // Spoofed/late/garbage datagrams are thrown
                                // away and we keep waiting for OUR answer.
                                if response_matches(q, &buf[..n]) {
                                    return Some(buf[..n].to_vec());
                                }
                            }
                            Err(_) => break,
                        }
                    }
                }
            }
        }
    }
    if let Ok(addr) = format!("{server}:53").parse::<SocketAddr>() {
        if let Ok(mut stream) = TcpStream::connect_timeout(&addr, timeout) {
            let _ = stream.set_read_timeout(Some(timeout));
            let len = (q.len() as u16).to_be_bytes();
            let sent = stream.write_all(&len).and_then(|_| stream.write_all(q));
            if sent.is_ok() {
                let mut lb = [0u8; 2];
                if stream.read_exact(&mut lb).is_ok() {
                    let n = u16::from_be_bytes(lb) as usize;
                    if (1..=4096).contains(&n) {
                        let mut buf = vec![0u8; n];
                        if stream.read_exact(&mut buf).is_ok() && response_matches(q, &buf) {
                            return Some(buf);
                        }
                    }
                }
            }
        }
    }
    None
}

fn dns_resolve(server: &str, domain: &str, timeout: Duration) -> Option<(Duration, DnsAnswer)> {
    let id = next_dns_id();
    let q = build_a_query(id, domain);
    let start = Instant::now();
    let resp = raw_query(server, &q, timeout)?;
    let elapsed = start.elapsed();
    let answer = parse_a_response(&resp, domain)?;
    if answer.addresses.is_empty() && answer.cname.is_none() {
        return None;
    }
    Some((elapsed, answer))
}

/// Resolve a domain following the CNAME chain (up to 3 hops) on the same
/// resolver, returning total time and gathered IPv4 addresses.
fn resolve_with_cname(
    server: &str,
    domain: &str,
    timeout: Duration,
) -> Option<(Duration, Vec<Ipv4Addr>)> {
    let mut cur = domain.to_string();
    let mut ips = Vec::new();
    let mut total = Duration::ZERO;
    for _ in 0..3 {
        let (elapsed, answer) = dns_resolve(server, &cur, timeout)?;
        total += elapsed;
        ips.extend(answer.addresses);
        match answer.cname {
            Some(c) if !c.is_empty() && c != cur => cur = c,
            _ => break,
        }
    }
    if ips.is_empty() {
        None
    } else {
        Some((total, ips))
    }
}

/// TCP connect RTT to the resolver's :53.
fn tcp_rtt(server: &str, timeout: Duration) -> Option<Duration> {
    let addr: SocketAddr = format!("{server}:53").parse().ok()?;
    let start = Instant::now();
    TcpStream::connect_timeout(&addr, timeout).ok()?;
    Some(start.elapsed())
}

// ── benchmark ───────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct BenchResult {
    pub id: String,
    pub name: String,
    pub primary: String,
    pub secondary: String,
    pub rtt_ms: Option<f64>,
    pub resolve_ms: Option<f64>,
    pub success_rate: f64,
    pub roblox_ok: bool,
    pub score: f64,
}

const BENCH_DOMAINS: &[&str] = &["tr.rbxcdn.com", "discord.com", "www.youtube.com", "mail.ru"];

pub fn bench_dns() -> Vec<BenchResult> {
    let mut results = Vec::with_capacity(DNS_SERVERS.len());
    for s in DNS_SERVERS {
        // TCP RTT: up to 3 attempts, 700 ms each.
        let mut rtts = Vec::new();
        for _ in 0..3 {
            if let Some(d) = tcp_rtt(s.primary, Duration::from_millis(700)) {
                rtts.push(d.as_secs_f64() * 1000.0);
            }
        }
        let rtt_ms = if rtts.is_empty() {
            None
        } else {
            Some(rtts.iter().sum::<f64>() / rtts.len() as f64)
        };

        // Resolution: UDP with TCP fallback, 1.5 s per domain (CNAME-aware).
        let mut ok = 0usize;
        let mut times = Vec::new();
        let mut roblox_ok = false;
        for d in BENCH_DOMAINS {
            if let Some((el, ips)) = resolve_with_cname(s.primary, d, Duration::from_millis(1500)) {
                ok += 1;
                times.push(el.as_secs_f64() * 1000.0);
                if *d == "tr.rbxcdn.com" && !ips.is_empty() {
                    roblox_ok = true;
                }
            }
        }
        let total = BENCH_DOMAINS.len();
        let success_rate = ok as f64 / total as f64;
        let resolve_ms = if times.is_empty() {
            None
        } else {
            Some(times.iter().sum::<f64>() / times.len() as f64)
        };

        // Score: 0..100. Latency 45%, resolution success 40%, Roblox CDN 15%.
        let latency_part = match rtt_ms {
            Some(r) => (1.0 - (r / 800.0).min(1.0)) * 45.0,
            None => 0.0,
        };
        let success_part = success_rate * 40.0;
        let roblox_part = if roblox_ok { 15.0 } else { 0.0 };
        let score = latency_part + success_part + roblox_part;

        results.push(BenchResult {
            id: s.id.to_string(),
            name: s.name.to_string(),
            primary: s.primary.to_string(),
            secondary: s.secondary.to_string(),
            rtt_ms,
            resolve_ms,
            success_rate,
            roblox_ok,
            score,
        });
    }
    results.sort_by(|a, b| b.score.total_cmp(&a.score));
    results
}

// ── apply / reset ──────────────────────────────────────────────────

fn active_adapter_script() -> &'static str {
    "Get-NetAdapter | Where-Object {$_.Status -eq 'Up'} | Select-Object -ExpandProperty ifIndex"
}

pub fn dns_status() -> Result<Value, CommandError> {
    let adapters = ps(&format!(
        "Get-DnsClientServerAddress -AddressFamily IPv4 | Where-Object {{$_.ServerAddresses.Count -gt 0}} | Select-Object InterfaceAlias,InterfaceIndex,@{{n='ServerAddresses';e={{$_.ServerAddresses -join ','}}}} | ConvertTo-Json -Compress"
    ))?;
    let doh = ps(
        "Get-DnsClientDohServerAddress -ErrorAction SilentlyContinue | Select-Object ServerAddress,DohTemplate,@{n='AutoUpgrade';e={$_.AutoUpgrade}},@{n='Fallback';e={$_.AllowFallbackToUdp}} | ConvertTo-Json -Compress",
    )
    .unwrap_or_else(|_| "[]".into());
    let adapters_json: Value = serde_json::from_str(adapters.trim()).unwrap_or_else(|_| json!([]));
    let doh_json: Value = serde_json::from_str(doh.trim()).unwrap_or_else(|_| json!([]));
    Ok(json!({
        "admin": is_admin(),
        "adapters": adapters_json,
        "doh": doh_json,
    }))
}

pub fn dns_apply(primary: &str, secondary: &str, use_doh: bool) -> Result<(), CommandError> {
    primary
        .parse::<Ipv4Addr>()
        .map_err(|_| CommandError::raw("bad ip"))?;
    secondary
        .parse::<Ipv4Addr>()
        .map_err(|_| CommandError::raw("bad ip"))?;

    let defaults = ps(active_adapter_script())?;
    let if_indices: Vec<&str> = defaults.split_whitespace().collect();
    if if_indices.is_empty() {
        return Err(CommandError::key("dns.no_adapter"));
    }
    ps(&format!(
        "Get-NetAdapter | Where-Object {{$_.Status -eq 'Up'}} | ForEach-Object {{Set-DnsClientServerAddress -InterfaceIndex $_.ifIndex -ServerAddresses ('{primary}','{secondary}')}}"
    ))?;

    if use_doh {
        let template = DNS_SERVERS
            .iter()
            .find(|s| s.primary == primary)
            .map(|s| s.doh_template)
            .filter(|t| !t.is_empty());
        if let Some(tpl) = template {
            // Apply DoH on both IPs. Failures are surfaced (with the
            // trimmed PowerShell stderr) instead of being swallowed.
            ps(&format!(
                "Set-DnsClientDohServerAddress -ServerAddress '{primary}' -DohTemplate '{tpl}' -AutoUpgrade $true -AllowFallbackToUdp $false"
            ))?;
            ps(&format!(
                "Set-DnsClientDohServerAddress -ServerAddress '{secondary}' -DohTemplate '{tpl}' -AutoUpgrade $true -AllowFallbackToUdp $false"
            ))?;
        }
    }

    let _ = std::process::Command::new("ipconfig")
        .args(["/flushdns"])
        .output();
    Ok(())
}

pub fn dns_reset() -> Result<(), CommandError> {
    ps(active_adapter_script())?;
    let _ = ps(
        "Get-NetAdapter | Where-Object {$_.Status -eq 'Up'} | ForEach-Object {Set-DnsClientServerAddress -InterfaceIndex $_.ifIndex -ResetServerAddresses}",
    )?;

    // Best-effort removal of DoH records for our known servers.
    let known: Vec<&str> = DNS_SERVERS
        .iter()
        .flat_map(|s| [s.primary, s.secondary])
        .collect();
    let known_list = known.join("','");
    let _ = ps(&format!(
        "$ips=@('{known_list}'); if(Get-Command Set-DnsClientDohServerAddress -ErrorAction SilentlyContinue){{ foreach($ip in $ips){{ Set-DnsClientDohServerAddress -ServerAddress $ip -Remove -ErrorAction SilentlyContinue }} }}"
    ));
    let _ = std::process::Command::new("ipconfig")
        .args(["/flushdns"])
        .output();
    Ok(())
}

// ── Roblox CDN hosts fix ────────────────────────────────────────────

#[derive(Serialize)]
pub struct HostsCandidate {
    pub ip: String,
    pub rtt_ms: f64,
}

pub fn roblox_find() -> Vec<HostsCandidate> {
    // Collect candidate IPs from several resolvers (UDP → TCP fallback,
    // CNAME-aware), then measure real TCP-443 RTT to each unique IP.
    let mut ips: Vec<Ipv4Addr> = Vec::new();
    for s in DNS_SERVERS.iter().take(4) {
        if let Some((_, found)) =
            resolve_with_cname(s.primary, "tr.rbxcdn.com", Duration::from_millis(1500))
        {
            ips.extend(found);
        }
    }
    // Deduplicate.
    let mut seen = std::collections::HashSet::new();
    ips.retain(|ip| seen.insert(*ip));

    let mut out = Vec::new();
    for ip in ips {
        let addr: SocketAddr = SocketAddr::new(ip.into(), 443);
        let mut best = None;
        for _ in 0..2 {
            let start = Instant::now();
            if TcpStream::connect_timeout(&addr, Duration::from_millis(800)).is_ok() {
                let ms = start.elapsed().as_secs_f64() * 1000.0;
                best = Some(best.map_or(ms, |b: f64| b.min(ms)));
            }
        }
        if let Some(ms) = best {
            out.push(HostsCandidate {
                ip: ip.to_string(),
                rtt_ms: ms,
            });
        }
    }
    out.sort_by(|a, b| a.rtt_ms.total_cmp(&b.rtt_ms));
    out
}

fn hosts_path() -> &'static str {
    HOSTS_PATH
}

fn read_hosts() -> Result<String, CommandError> {
    std::fs::read_to_string(hosts_path())
        .map_err(|e| CommandError::raw(format!("cannot read hosts: {e}")))
}

fn managed_block(ip: &str) -> String {
    format!("{HOSTS_MARK_BEGIN}\r\n{ip}    tr.rbxcdn.com\r\n{HOSTS_MARK_END}\r\n")
}

/// Result of a hosts-file modification: the backup path plus an explicit
/// warning when an unpaired managed block had to be repaired (the old code
/// silently returned Ok in that case).
#[derive(Serialize)]
pub struct HostsApplyReport {
    pub backup: String,
    pub repaired: bool,
    pub warning: Option<String>,
}

/// Byte bounds of the line containing `pos`.
fn line_bounds(text: &str, pos: usize) -> (usize, usize) {
    let start = text[..pos].rfind('\n').map(|r| r + 1).unwrap_or(0);
    let end = text[pos..]
        .find('\n')
        .map(|r| pos + r + 1)
        .unwrap_or(text.len());
    (start, end)
}

/// A stray managed entry line (`<ip>    tr.rbxcdn.com`).
fn is_stray_entry(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty() && !t.starts_with('#') && t.ends_with("tr.rbxcdn.com")
}

/// Remove an orphan marker line at `orphan`, together with any stray managed
/// block content directly attached to it (entry lines after an orphan BEGIN
/// marker, entry lines before an orphan END marker).
fn repair_orphan(text: &str, orphan: usize, marker: &str) -> String {
    let (line_start, line_end) = line_bounds(text, orphan);
    let mut from = line_start;
    let mut to = line_end;
    if marker == HOSTS_MARK_BEGIN {
        loop {
            if to >= text.len() {
                break;
            }
            let (s, e) = line_bounds(text, to);
            if s != to || !is_stray_entry(&text[s..e]) {
                break;
            }
            to = e;
        }
    } else {
        loop {
            if from == 0 {
                break;
            }
            let prev_start = text[..from].rfind('\n').map(|r| r + 1).unwrap_or(0);
            if prev_start == from || !is_stray_entry(&text[prev_start..from]) {
                break;
            }
            from = prev_start;
        }
    }
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..from]);
    out.push_str(&text[to..]);
    out
}

/// Write the hosts file safely: a fresh best-effort backup of the previous
/// content first, then an atomic replace (via `integra_core::fsutil`) so a
/// crash mid-write can never truncate the system hosts file.
fn write_hosts(path: &str, new_text: &str) -> Result<(), CommandError> {
    let bak = format!("{path}.bak");
    if let Ok(prev) = std::fs::read(path) {
        let _ = std::fs::write(&bak, prev);
    }
    integra_core::fsutil::atomic_write(std::path::Path::new(path), new_text.as_bytes())
        .map_err(|e| CommandError::raw(format!("write hosts failed: {e}")))
}

pub fn roblox_hosts_apply(ip: &str) -> Result<HostsApplyReport, CommandError> {
    ip.parse::<Ipv4Addr>()
        .map_err(|_| CommandError::raw("bad ip"))?;
    let path = hosts_path();
    // One-time pristine backup (kept from previous versions).
    let bak = format!("{path}.integra.bak");
    if !std::path::Path::new(&bak).exists() {
        if let Ok(hosts) = std::fs::read(path) {
            let _ = std::fs::write(&bak, hosts);
        }
    }
    let text = read_hosts()?;
    let block = managed_block(ip);
    let begin = text.find(HOSTS_MARK_BEGIN);
    let end = text.find(HOSTS_MARK_END);
    let (new_text, repaired, warning) = match (begin, end) {
        (Some(start), Some(end_rel)) => {
            // Both markers: replace everything between them.
            let end_abs = start + end_rel + HOSTS_MARK_END.len();
            let mut t = text[..start].to_string();
            t.push_str(&block);
            t.push_str(&text[end_abs..]);
            (t, false, None)
        }
        (Some(start), None) => {
            // Orphan BEGIN marker: strip the marker line and any stray
            // managed entries, then append a fresh managed block.
            let mut t = repair_orphan(&text, start, HOSTS_MARK_BEGIN);
            if !t.ends_with('\n') {
                t.push('\n');
            }
            t.push_str(&block);
            (
                t,
                true,
                Some(
                    "unpaired begin marker found in hosts; the orphaned block was repaired"
                        .to_string(),
                ),
            )
        }
        (None, Some(orphan)) => {
            // Orphan END marker: strip it (plus stray entries before it)
            // and append a fresh managed block.
            let mut t = repair_orphan(&text, orphan, HOSTS_MARK_END);
            if !t.ends_with('\n') {
                t.push('\n');
            }
            t.push_str(&block);
            (
                t,
                true,
                Some(
                    "unpaired end marker found in hosts; the orphaned block was repaired"
                        .to_string(),
                ),
            )
        }
        (None, None) => {
            let mut t = text.clone();
            if !t.ends_with('\n') {
                t.push('\n');
            }
            t.push_str(&block);
            (t, false, None)
        }
    };
    write_hosts(path, &new_text)?;
    Ok(HostsApplyReport {
        backup: bak,
        repaired,
        warning,
    })
}

pub fn roblox_hosts_revert() -> Result<bool, CommandError> {
    let text = read_hosts()?;
    let begin = text.find(HOSTS_MARK_BEGIN);
    let end = text.find(HOSTS_MARK_END);
    let (start, end_abs) = match (begin, end) {
        (Some(start), Some(end_rel)) => (start, start + end_rel + HOSTS_MARK_END.len()),
        (Some(start), None) => {
            // Orphan BEGIN marker: repair instead of silently passing.
            let repaired = repair_orphan(&text, start, HOSTS_MARK_BEGIN);
            write_hosts(hosts_path(), &repaired)?;
            return Ok(true);
        }
        (None, Some(orphan)) => {
            // Orphan END marker: repair instead of silently passing.
            let repaired = repair_orphan(&text, orphan, HOSTS_MARK_END);
            write_hosts(hosts_path(), &repaired)?;
            return Ok(true);
        }
        (None, None) => return Ok(false),
    };
    let new_text = format!("{}{}", &text[..start], &text[end_abs..]);
    write_hosts(hosts_path(), &new_text)?;
    Ok(true)
}

pub fn roblox_hosts_state() -> Result<bool, CommandError> {
    Ok(read_hosts()?.contains(HOSTS_MARK_BEGIN))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live smoke test: resolver TCP/UDP fallback must actually resolve a
    /// real domain via a public resolver. VPN/offline environments will
    /// fail — this is informational, not a gate.
    #[test]
    fn live_resolve_via_public_resolvers() {
        let servers = ["1.1.1.1", "9.9.9.9", "8.8.8.8", "77.88.8.8"];
        let mut any = false;
        for s in servers {
            if let Some((elapsed, ips)) =
                resolve_with_cname(s, "tr.rbxcdn.com", Duration::from_millis(2000))
            {
                let ms = elapsed.as_secs_f64() * 1000.0;
                println!("{s}: {ms:.0}ms ips={ips:?}");
                any = true;
            } else {
                println!("{s}: no answer (blocked or offline)");
            }
        }
        if !any {
            eprintln!("NOTE: all public resolvers unreachable — network filter or offline");
        }
    }
}
