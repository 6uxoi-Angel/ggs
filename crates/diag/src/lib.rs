//! integra-diag — connectivity checks and (later) strategy benchmarks.
//!
//! Iteration 1 implements DNS and TCP checks with multi-round latency stats.
//! TLS/HTTP checks and the strategy benchmark engine land in iteration 5;
//! their types are already part of the public API so the UI can render them.

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::time::{Duration, Instant};

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
            CheckKind::Tls | CheckKind::Http => CheckOutcome {
                kind: *kind,
                ok: false,
                latency: None,
                addrs: vec![],
                error: Some("coming in iteration 5".into()),
            },
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
    fn tls_http_are_declared_but_pending() {
        let site = Site::builtin("x", "X", "localhost");
        let outcomes = check_site(&site, &[CheckKind::Tls], 1, Duration::from_secs(1));
        assert!(!outcomes[0].ok);
        assert!(outcomes[0].error.as_deref().unwrap().contains("iteration 5"));
    }
}
