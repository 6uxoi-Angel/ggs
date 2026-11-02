//! Collector abstraction + platform factory.

use crate::{Connection, ProcessInfo};

pub trait Collector: Send + Sync {
    /// Current snapshot of TCP/UDP connections with owning PIDs.
    fn snapshot(&self) -> Vec<Connection>;
    /// All visible processes (for name filtering and the UI picker).
    fn processes(&self) -> Vec<ProcessInfo>;
}

pub fn default_collector() -> Box<dyn Collector> {
    #[cfg(windows)]
    {
        Box::new(crate::windows_tables::WindowsCollector)
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(crate::procfs::ProcFsCollector::default())
    }
    #[cfg(all(not(windows), not(target_os = "linux")))]
    {
        Box::new(EmptyCollector)
    }
}

#[cfg(all(not(windows), not(target_os = "linux")))]
pub struct EmptyCollector;

#[cfg(all(not(windows), not(target_os = "linux")))]
impl Collector for EmptyCollector {
    fn snapshot(&self) -> Vec<Connection> {
        Vec::new()
    }
    fn processes(&self) -> Vec<ProcessInfo> {
        Vec::new()
    }
}
