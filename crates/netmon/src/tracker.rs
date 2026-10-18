//! Tracking sessions: polling loop, filtering, dedup, reverse DNS.

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use chrono::Local;
use parking_lot::Mutex;
use serde::Serialize;

use crate::collector::{default_collector, Collector};
use crate::services::label_for_port;
use crate::{ConnectionKey, NetmonError, ProcessInfo, TrackedEntry};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "mode", content = "value", rename_all = "snake_case")]
pub enum ProcessFilter {
    All,
    /// Case-insensitive substring of the process name; matching processes are
    /// (re)discovered every few polls, so apps launched *after* the session
    /// start (the "launch Apex after pressing record" flow) are picked up.
    Name(String),
    Pids(Vec<u32>),
}

#[derive(Debug, Clone)]
pub struct TrackerConfig {
    pub filter: ProcessFilter,
    pub poll_interval: Duration,
    pub resolve_hosts: bool,
    pub max_entries: usize,
    /// How often the process list is re-scanned (auto-inclusion latency).
    pub process_refresh: Duration,
}

impl Default for TrackerConfig {
    fn default() -> Self {
        Self {
            filter: ProcessFilter::All,
            poll_interval: Duration::from_millis(1000),
            resolve_hosts: true,
            max_entries: 5000,
            process_refresh: Duration::from_secs(3),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary {
    pub entries: usize,
    pub processes_seen: usize,
    pub duration_secs: u64,
}

type OnEntry = Arc<dyn Fn(&TrackedEntry) + Send + Sync>;
type EntryStore = Arc<Mutex<Vec<(ConnectionKey, TrackedEntry)>>>;

struct Session {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<()>,
    entries: EntryStore,
    started: Instant,
}

struct Finished {
    entries: EntryStore,
    summary: SessionSummary,
}

/// Shared handle controlling at most one active tracking session.
#[derive(Default)]
pub struct Netmon {
    session: Mutex<Option<Session>>,
    finished: Mutex<Option<Finished>>,
    on_entry: Mutex<Option<OnEntry>>,
    collector: Mutex<Option<Arc<Box<dyn Collector>>>>,
}

impl Netmon {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Collector override (used by tests).
    pub fn with_collector(collector: Box<dyn Collector>) -> Arc<Self> {
        let netmon = Self::default();
        *netmon.collector.lock() = Some(Arc::new(collector));
        Arc::new(netmon)
    }

    fn collector(&self) -> Arc<Box<dyn Collector>> {
        let mut guard = self.collector.lock();
        guard.get_or_insert_with(|| Arc::new(default_collector())).clone()
    }

    pub fn set_on_entry(&self, callback: OnEntry) {
        *self.on_entry.lock() = Some(callback);
    }

    pub fn is_running(&self) -> bool {
        self.session.lock().as_ref().is_some_and(|s| !s.stop.load(Ordering::Relaxed))
    }

    pub fn last_summary(&self) -> Option<SessionSummary> {
        self.finished.lock().as_ref().map(|f| f.summary.clone())
    }

    pub fn start(&self, config: TrackerConfig) -> Result<(), NetmonError> {
        let mut guard = self.session.lock();
        if guard.as_ref().is_some_and(|s| !s.stop.load(Ordering::Relaxed)) {
            return Err(NetmonError::AlreadyRunning);
        }

        let collector = self.collector();
        let stop = Arc::new(AtomicBool::new(false));
        let entries: EntryStore = Arc::new(Mutex::new(Vec::new()));
        let on_entry = self.on_entry.lock().clone();

        let loop_stop = stop.clone();
        let loop_entries = entries.clone();
        let handle = std::thread::Builder::new()
            .name("integra-netmon".into())
            .spawn(move || run_session(collector, config, loop_stop, loop_entries, on_entry))
            .map_err(|_| NetmonError::NotRunning)?;

        *self.finished.lock() = None;
        *guard = Some(Session { stop, handle, entries, started: Instant::now() });
        Ok(())
    }

    pub fn stop(&self) -> Result<SessionSummary, NetmonError> {
        let session = self.session.lock().take().ok_or(NetmonError::NotRunning)?;
        session.stop.store(true, Ordering::Relaxed);
        let _ = session.handle.join();
        let store = session.entries.lock();
        let summary = SessionSummary {
            entries: store.len(),
            processes_seen: store.iter().map(|(_, e)| e.pid).collect::<HashSet<_>>().len(),
            duration_secs: session.started.elapsed().as_secs(),
        };
        drop(store);
        *self.finished.lock() = Some(Finished { entries: session.entries, summary: summary.clone() });
        Ok(summary)
    }

    fn store(&self) -> Option<EntryStore> {
        if let Some(session) = self.session.lock().as_ref() {
            return Some(session.entries.clone());
        }
        self.finished.lock().as_ref().map(|f| f.entries.clone())
    }

    pub fn entries(&self, limit: usize) -> Vec<TrackedEntry> {
        let Some(store) = self.store() else { return Vec::new() };
        let store = store.lock();
        let start = store.len().saturating_sub(limit.max(1));
        store[start..].iter().map(|(_, e)| e.clone()).collect()
    }

    /// Export the current/last session as CSV.
    pub fn export_csv(&self) -> String {
        let mut out = String::from("first_seen,last_seen,hits,process,pid,protocol,local,remote,host,service\n");
        for e in self.entries(usize::MAX) {
            out.push_str(&format!(
                "{},{},{},\"{}\",{},{:?},{},{},{},{}\n",
                e.first_seen.to_rfc3339(),
                e.last_seen.to_rfc3339(),
                e.hits,
                e.process.replace('"', "\"\""),
                e.pid,
                e.proto,
                e.local,
                e.remote.unwrap_or_default(),
                e.host.clone().unwrap_or_default(),
                e.service.clone().unwrap_or_default(),
            ));
        }
        out
    }

    /// Current process list for the UI picker.
    pub fn processes(&self) -> Vec<ProcessInfo> {
        let mut procs = self.collector().processes();
        procs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        procs.dedup_by(|a, b| a.pid == b.pid);
        procs
    }
}

fn run_session(
    collector: Arc<Box<dyn Collector>>,
    config: TrackerConfig,
    stop: Arc<AtomicBool>,
    entries: EntryStore,
    on_entry: Option<OnEntry>,
) {
    let mut seen: HashMap<ConnectionKey, usize> = HashMap::new();
    let mut process_cache: HashMap<u32, ProcessInfo> = HashMap::new();
    let mut matched_pids: HashSet<u32> = HashSet::new();
    let process_refresh = config.process_refresh;
    let mut last_process_refresh = Instant::now() - process_refresh;

    // Reverse-DNS worker keeps blocking lookups off the polling loop.
    let (resolve_tx, resolve_rx): (Sender<IpAddr>, Receiver<IpAddr>) = channel();
    let dns_cache: Arc<Mutex<HashMap<IpAddr, Option<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let resolver = if config.resolve_hosts {
        std::thread::Builder::new()
            .name("integra-netmon-dns".into())
            .spawn({
                let entries = entries.clone();
                let cache = dns_cache.clone();
                let stop = stop.clone();
                move || resolver_loop(resolve_rx, entries, cache, stop)
            })
            .ok()
    } else {
        None
    };

    while !stop.load(Ordering::Relaxed) {
        let now = Local::now();
        let snapshot = collector.snapshot();

        if last_process_refresh.elapsed() > process_refresh {
            for proc in collector.processes() {
                process_cache.insert(proc.pid, proc);
            }
            matched_pids = match &config.filter {
                ProcessFilter::All => process_cache.keys().copied().collect(),
                ProcessFilter::Pids(pids) => pids.iter().copied().collect(),
                ProcessFilter::Name(pattern) => {
                    let pattern = pattern.to_lowercase();
                    process_cache
                        .iter()
                        .filter(|(_, p)| {
                            p.name.to_lowercase().contains(&pattern)
                                || p.exe.as_ref().is_some_and(|e| e.to_lowercase().contains(&pattern))
                        })
                        .map(|(pid, _)| *pid)
                        .collect()
                }
            };
            last_process_refresh = Instant::now();
        }

        let mut pending_resolve: Vec<IpAddr> = Vec::new();
        {
            let mut store = entries.lock();
            for conn in snapshot {
                if !matched_pids.contains(&conn.pid) {
                    continue;
                }
                let key = conn.key();
                if let Some(&index) = seen.get(&key) {
                    if let Some((_, entry)) = store.get_mut(index) {
                        entry.last_seen = now;
                        entry.hits += 1;
                    }
                    continue;
                }

                let proc = process_cache.get(&conn.pid);
                let remote_port = conn.remote.map(|r| r.port());
                let entry = TrackedEntry {
                    proto: conn.proto,
                    pid: conn.pid,
                    process: proc.map(|p| p.name.clone()).unwrap_or_else(|| format!("pid {}", conn.pid)),
                    exe: proc.and_then(|p| p.exe.clone()),
                    local: conn.local.to_string(),
                    remote: conn.remote.map(|r| r.to_string()),
                    remote_ip: conn.remote_ip().map(|ip| ip.to_string()),
                    remote_port,
                    host: None,
                    service: remote_port.and_then(label_for_port).map(str::to_string),
                    state: conn.state,
                    first_seen: now,
                    last_seen: now,
                    hits: 1,
                };
                seen.insert(key, store.len());
                store.push((key, entry.clone()));
                if store.len() > config.max_entries {
                    let (evicted, _) = store.remove(0);
                    seen.remove(&evicted);
                    for index in seen.values_mut() {
                        *index = index.saturating_sub(1);
                    }
                }
                if let Some(ip) = conn.remote_ip() {
                    if config.resolve_hosts && !ip.is_loopback() && !ip.is_unspecified() {
                        pending_resolve.push(ip);
                    }
                }
                if let Some(cb) = &on_entry {
                    cb(&entry);
                }
            }
        }
        for ip in pending_resolve {
            let cached = dns_cache.lock().get(&ip).cloned();
            match cached {
                Some(Some(host)) => apply_host(&entries, &ip, &host),
                Some(None) => {}
                None => {
                    if resolve_tx.send(ip).is_err() {
                        break;
                    }
                }
            }
        }

        std::thread::sleep(config.poll_interval);
    }
    drop(resolve_tx);
    if let Some(resolver) = resolver {
        let _ = resolver.join();
    }
}

fn apply_host(entries: &EntryStore, ip: &IpAddr, host: &str) {
    let needle = ip.to_string();
    let mut store = entries.lock();
    for (_, entry) in store.iter_mut() {
        if entry.host.is_none() && entry.remote_ip.as_deref() == Some(needle.as_str()) {
            entry.host = Some(host.to_string());
        }
    }
}

fn resolver_loop(
    rx: Receiver<IpAddr>,
    entries: EntryStore,
    cache: Arc<Mutex<HashMap<IpAddr, Option<String>>>>,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Relaxed) {
        let Ok(ip) = rx.recv_timeout(Duration::from_millis(250)) else { continue };
        if let Some(cached) = cache.lock().get(&ip).cloned() {
            if let Some(host) = cached {
                apply_host(&entries, &ip, &host);
            }
            continue;
        }
        let resolved = dns_lookup::lookup_addr(&ip).ok();
        cache.lock().insert(ip, resolved.clone());
        if let Some(host) = resolved {
            apply_host(&entries, &ip, &host);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::Collector;
    use crate::{Connection, Protocol, TcpState};
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
    use std::sync::atomic::AtomicUsize;

    fn addr(ip: [u8; 4], port: u16) -> SocketAddr {
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::from(ip), port))
    }

    /// Two fake processes; `r5apex.exe` "launches" only after a few polls,
    /// exercising auto-inclusion of new processes by name.
    struct FakeCollector {
        polls: AtomicUsize,
        apex_after: usize,
    }

    impl Collector for FakeCollector {
        fn snapshot(&self) -> Vec<Connection> {
            let poll = self.polls.fetch_add(1, Ordering::SeqCst);
            let mut conns = vec![Connection {
                proto: Protocol::Tcp4,
                local: addr([192, 168, 1, 10], 50000),
                remote: Some(addr([93, 184, 216, 34], 443)),
                state: Some(TcpState::Estab),
                pid: 1000,
            }];
            if poll >= self.apex_after {
                conns.push(Connection {
                    proto: Protocol::Udp4,
                    local: addr([192, 168, 1, 10], 50001),
                    remote: Some(addr([23, 45, 67, 89], 27015)),
                    state: None,
                    pid: 2000,
                });
            }
            conns
        }

        fn processes(&self) -> Vec<ProcessInfo> {
            let mut procs = vec![ProcessInfo { pid: 1000, name: "discord.exe".into(), exe: None }];
            if self.polls.load(Ordering::SeqCst) >= self.apex_after {
                procs.push(ProcessInfo { pid: 2000, name: "r5apex.exe".into(), exe: None });
            }
            procs
        }
    }

    fn config(filter: ProcessFilter) -> TrackerConfig {
        TrackerConfig {
            filter,
            poll_interval: Duration::from_millis(40),
            resolve_hosts: false,
            max_entries: 100,
            process_refresh: Duration::from_millis(90),
        }
    }

    #[test]
    fn name_filter_tracks_only_matching_process() {
        let netmon = Netmon::with_collector(Box::new(FakeCollector {
            polls: AtomicUsize::new(0),
            apex_after: 0,
        }));
        netmon.start(config(ProcessFilter::Name("apex".into()))).unwrap();
        assert!(netmon.start(TrackerConfig::default()).is_err(), "double start rejected");
        std::thread::sleep(Duration::from_millis(250));
        let entries = netmon.entries(100);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].process, "r5apex.exe");
        assert!(entries[0].hits >= 2, "repeated snapshots dedup into hits");
        assert_eq!(entries[0].service.as_deref(), Some("Steam"));

        let summary = netmon.stop().unwrap();
        assert_eq!(summary.entries, 1);
        assert!(!netmon.is_running());
        assert!(netmon.export_csv().contains("r5apex.exe"));
        // Entries stay readable after stop.
        assert_eq!(netmon.entries(100).len(), 1);
    }

    #[test]
    fn late_launched_process_is_auto_included() {
        let netmon = Netmon::with_collector(Box::new(FakeCollector {
            polls: AtomicUsize::new(0),
            apex_after: 3, // appears after the session already runs
        }));
        netmon.start(config(ProcessFilter::Name("apex".into()))).unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let entries = netmon.entries(100);
        assert_eq!(entries.len(), 1, "apex launched later is still captured");
        assert_eq!(entries[0].pid, 2000);
        netmon.stop().unwrap();
    }

    #[test]
    fn all_filter_captures_everything() {
        let netmon = Netmon::with_collector(Box::new(FakeCollector {
            polls: AtomicUsize::new(0),
            apex_after: 0,
        }));
        netmon.start(config(ProcessFilter::All)).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(netmon.entries(100).len(), 2);
        netmon.stop().unwrap();
        assert!(netmon.last_summary().is_some());
    }

    #[test]
    fn stop_without_session_errors() {
        let netmon = Netmon::new();
        assert!(matches!(netmon.stop(), Err(NetmonError::NotRunning)));
        assert!(netmon.entries(10).is_empty());
    }
}
