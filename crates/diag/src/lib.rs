//! integra-diag — connectivity checks and strategy benchmark scoring input.
//!
//! * DNS — resolver latency;
//! * TCP — handshake latency;
//! * TLS — a **real TLS handshake** through rustls with webpki-roots
//!   (no system tools involved), so DPI interference with the encrypted
//!   handshake is detected;
//! * HTTP — a HTTPS HEAD request on top of the same TLS session.

use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use rustls::pki_types::ServerName;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Site {
    pub id: String,
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_port() -> u16 {
    443
}

impl Site {
    pub fn builtin(id: &str, name: &str, host: &str) -> Site {
        Site { id: id.into(), name: name.into(), host: host.into(), port: 443 }
    }
}

pub fn default_sites() -> Vec<Site> {
    vec![
        Site::builtin("default-discord", "Discord", "discord.com"),
        Site::builtin("default-youtube", "YouTube", "youtube.com"),
        Site::builtin("default-github", "GitHub", "github.com"),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    Dns,
    Tcp,
    Tls,
    Http,
}

#[derive(Debug, Clone, Serialize)]
pub struct LatencyStats {
    pub min_ms: u64,
    pub avg_ms: u64,
    pub max_ms: u64,
    pub rounds: u32,
    pub successes: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckOutcome {
    pub kind: CheckKind,
    pub ok: bool,
    pub latency: Option<LatencyStats>,
    pub addrs: Vec<String>,
    pub error: Option<String>,
}

/// Resolve a hostname with a hard timeout (std resolver has none).
pub fn resolve_host(host: &str, timeout: Duration) -> Result<Vec<SocketAddr>, String> {
    let (tx, rx) = channel();
    let owned = host.to_string();
    std::thread::spawn(move || {
        let result = (owned.as_str(), 0u16)
            .to_socket_addrs()
            .map(|iter| iter.collect::<Vec<_>>())
            .map_err(|e| e.to_string());
        let _ = tx.send(result);
    });
    match rx.recv_timeout(timeout) {
        Ok(Ok(addrs)) if !addrs.is_empty() => Ok(addrs),
        Ok(Ok(_)) => Err(integra_core::i18n::tr("network.dns_failed")),
        Ok(Err(err)) => Err(err),
        Err(RecvTimeoutError::Timeout) => Err(integra_core::i18n::tr("network.timeout")),
        Err(RecvTimeoutError::Disconnected) => Err(integra_core::i18n::tr("network.dns_failed")),
    }
}

/// One TCP handshake measurement against a resolved address.
pub fn tcp_once(addr: SocketAddr, timeout: Duration) -> Result<u64, String> {
    let started = Instant::now();
    match TcpStream::connect_timeout(&addr, timeout) {
        Ok(_) => Ok(started.elapsed().as_millis().max(1) as u64),
        Err(err) if err.kind() == std::io::ErrorKind::TimedOut => {
            Err(integra_core::i18n::tr("network.timeout"))
        }
        Err(_) => Err(integra_core::i18n::tr("network.unreachable")),
    }
}

fn stats(samples: &[u64]) -> Option<LatencyStats> {
    if samples.is_empty() {
        return None;
    }
    let sum: u64 = samples.iter().sum();
    Some(LatencyStats {
        min_ms: *samples.iter().min().unwrap(),
        avg_ms: sum / samples.len() as u64,
        max_ms: *samples.iter().max().unwrap(),
        rounds: samples.len() as u32,
        successes: samples.len() as u32,
    })
}

/// Measure TCP latency (DNS + handshake), mirroring the legacy Integra ping.
pub fn measure_tcp_latency(host: &str, port: u16, timeout: Duration) -> (bool, Option<u64>, Option<String>) {
    let started = Instant::now();
    let addrs = match resolve_host(host, timeout) {
        Ok(addrs) => addrs,
        Err(err) => return (false, None, Some(err)),
    };
    let mut last_error = None;
    for addr in addrs {
        let addr = SocketAddr::new(addr.ip(), port);
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(_) => {
                let ms = started.elapsed().as_millis().max(1) as u64;
                return (true, Some(ms), None);
            }
            Err(err) => last_error = Some(err.to_string()),
        }
    }
    (false, None, Some(last_error.unwrap_or_else(|| integra_core::i18n::tr("network.unreachable"))))
}

// ── TLS / HTTP (real handshake via rustls) ─────────────────

fn io_error_message(err: std::io::Error) -> String {
    match err.kind() {
        ErrorKind::TimedOut | ErrorKind::WouldBlock => integra_core::i18n::tr("network.timeout"),
        ErrorKind::ConnectionRefused => integra_core::i18n::tr("network.unreachable"),
        _ => err.to_string(),
    }
}

/// Shared TLS client config (webpki-roots, ring provider).
fn tls_client_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            Arc::new(
                rustls::ClientConfig::builder_with_provider(provider)
                    .with_safe_default_protocol_versions()
                    .expect("default protocol versions are supported")
                    .with_root_certificates(roots)
                    .with_no_client_auth(),
            )
        })
        .clone()
}

/// Parse `HTTP/1.1 200 OK` → 200. Returns None when no status line found.
pub fn parse_http_status(head: &str) -> Option<u16> {
    let line = head.lines().next()?;
    let mut parts = line.split_whitespace();
    let version = parts.next()?;
    if !version.to_ascii_uppercase().starts_with("HTTP/") {
        return None;
    }
    parts.next()?.parse::<u16>().ok()
}

/// One HTTPS probe: TCP connect + real TLS handshake (rustls + webpki-roots)
/// + HTTPS HEAD request. Returns `(status, total_ms)`; `status` is None when
/// the handshake and the write succeeded but no parseable response arrived
/// (which still proves TLS works).
pub fn https_probe(host: &str, addr: SocketAddr, timeout: Duration) -> Result<(Option<u16>, u64), String> {
    let server_name = ServerName::try_from(host.to_string())
        .map_err(|_| integra_core::i18n::tr("core.invalid_domain"))?;
    let conn = rustls::ClientConnection::new(tls_client_config(), server_name)
        .map_err(|e| format!("TLS: {e}"))?;

    let started = Instant::now();
    let tcp = TcpStream::connect_timeout(&addr, timeout).map_err(io_error_message)?;
    tcp.set_read_timeout(Some(timeout)).map_err(io_error_message)?;
    tcp.set_write_timeout(Some(timeout)).map_err(io_error_message)?;

    let mut tls = rustls::StreamOwned::new(conn, tcp);
    let request = format!(
        "HEAD / HTTP/1.1\r\nHost: {host}\r\nUser-Agent: Integra-Diag\r\nAccept: */*\r\nConnection: close\r\n\r\n"
    );
    tls.write_all(request.as_bytes())
        .map_err(|e| io_error_message(e))?;

    let mut buf = [0u8; 1024];
    let mut received = 0usize;
    loop {
        match tls.read(&mut buf[received..]) {
            Ok(0) => break,
            Ok(n) => {
                received += n;
                // Response headers usually arrive in the first chunk.
                if received >= 16 || buf[..received].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Err(err) => return Err(io_error_message(err)),
        }
    }
    let ms = started.elapsed().as_millis().max(1) as u64;
    let head = String::from_utf8_lossy(&buf[..received]).to_string();
    Ok((parse_http_status(&head), ms))
}

/// First successful probe across all resolved addresses.
fn probe_all_addrs(
    host: &str,
    port: u16,
    timeout: Duration,
    is_ok: impl Fn(Option<u16>) -> bool,
) -> Result<u64, String> {
    let resolved = resolve_host(host, timeout)?;
    let mut last = Err(integra_core::i18n::tr("network.unreachable"));
    for addr in resolved {
        match https_probe(host, SocketAddr::new(addr.ip(), port), timeout) {
            Ok((status, ms)) if is_ok(status) => return Ok(ms),
            Ok((status, _)) => {
                let detail = status
                    .map(|code| format!("HTTP {code}"))
                    .unwrap_or_else(|| integra_core::i18n::tr("network.unreachable"));
                last = Err(detail);
            }
            Err(err) => last = Err(err),
        }
    }
    last
}

/// Run the requested checks against one site, `rounds` times each.
pub fn check_site(site: &Site, kinds: &[CheckKind], rounds: u32, timeout: Duration) -> Vec<CheckOutcome> {
    let rounds = rounds.clamp(1, 10);
    let mut outcomes = Vec::new();
    for kind in kinds {
        outcomes.push(match kind {
            CheckKind::Dns => {
                let mut samples = Vec::new();
                let mut addrs = Vec::new();
                let mut error = None;
                for _ in 0..rounds {
                    let started = Instant::now();
                    match resolve_host(&site.host, timeout) {
                        Ok(resolved) => {
                            samples.push(started.elapsed().as_millis().max(1) as u64);
                            addrs = resolved.iter().map(|a| a.ip().to_string()).collect();
                        }
                        Err(err) => {
                            error = Some(err);
                            break;
                        }
                    }
                }
                CheckOutcome {
                    kind: *kind,
                    ok: error.is_none(),
                    latency: stats(&samples),
                    addrs,
                    error,
                }
            }
            CheckKind::Tcp => {
                let mut samples = Vec::new();
                let mut error = None;
                for _ in 0..rounds {
                    match resolve_host(&site.host, timeout).and_then(|resolved| {
                        let mut last = Err(integra_core::i18n::tr("network.unreachable"));
                        for addr in resolved {
                            match tcp_once(SocketAddr::new(addr.ip(), site.port), timeout) {
                                Ok(ms) => return Ok(ms),
                                Err(err) => last = Err(err),
                            }
                        }
                        last
                    }) {
                        Ok(ms) => samples.push(ms),
                        Err(err) => {
                            error = Some(err);
                            break;
                        }
                    }
                }
                CheckOutcome { kind: *kind, ok: error.is_none(), latency: stats(&samples), addrs: vec![], error }
            }
            CheckKind::Tls => {
                let mut samples = Vec::new();
                let mut error = None;
                for _ in 0..rounds {
                    match probe_all_addrs(&site.host, site.port, timeout, |_| true) {
                        Ok(ms) => samples.push(ms),
                        Err(err) => {
                            error = Some(err);
                            break;
                        }
                    }
                }
                CheckOutcome { kind: *kind, ok: error.is_none(), latency: stats(&samples), addrs: vec![], error }
            }
            CheckKind::Http => {
                let mut samples = Vec::new();
                let mut error = None;
                for _ in 0..rounds {
                    match probe_all_addrs(&site.host, site.port, timeout, |status| {
                        status.is_some_and(|code| (100..500).contains(&code))
                    }) {
                        Ok(ms) => samples.push(ms),
                        Err(err) => {
                            error = Some(err);
                            break;
                        }
                    }
                }
                CheckOutcome { kind: *kind, ok: error.is_none(), latency: stats(&samples), addrs: vec![], error }
            }
        });
    }
    outcomes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_localhost() {
        let addrs = resolve_host("localhost", Duration::from_secs(2)).unwrap();
        assert!(!addrs.is_empty());
    }

    #[test]
    fn dns_failure_is_reported() {
        assert!(resolve_host("definitely-not-a-real-host.invalid", Duration::from_secs(2)).is_err());
    }

    #[test]
    fn tcp_to_closed_port_fails_fast() {
        // Port 1 on loopback: refused immediately, no hang.
        let (ok, _, error) = measure_tcp_latency("localhost", 1, Duration::from_secs(2));
        assert!(!ok);
        assert!(error.is_some());
    }

    #[test]
    fn stats_math() {
        let s = stats(&[10, 20, 30]).unwrap();
        assert_eq!((s.min_ms, s.avg_ms, s.max_ms, s.rounds), (10, 20, 30, 3));
        assert!(stats(&[]).is_none());
    }

    #[test]
    fn tls_http_are_fully_implemented() {
        let site = Site::builtin("x", "X", "localhost");
        // Both stages must be attempted against a real (closed) port and
        // report a proper error, not a "coming later" stub.
        let outcomes = check_site(&site, &[CheckKind::Tls, CheckKind::Http], 1, Duration::from_secs(2));
        for outcome in outcomes {
            assert!(!outcome.ok);
            let error = outcome.error.unwrap_or_default();
            assert!(!error.contains("iteration"), "no stubs left: {error}");
        }
    }

    #[test]
    fn parses_http_status_lines() {
        assert_eq!(parse_http_status("HTTP/1.1 200 OK\r\nServer: x"), Some(200));
        assert_eq!(parse_http_status("HTTP/2 403"), Some(403));
        assert_eq!(parse_http_status("garbage"), None);
        assert_eq!(parse_http_status(""), None);
    }

    #[test]
    fn tls_probe_to_closed_port_fails_fast() {
        let addr: SocketAddr = "127.0.0.1:1".parse().unwrap();
        assert!(https_probe("localhost", addr, Duration::from_secs(1)).is_err());
    }
}
