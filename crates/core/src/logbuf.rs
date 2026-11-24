//! In-memory ring log buffer with fan-out to subscribers (UI logs page,
//! Tauri events, crash diagnostics).

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::thread::ThreadId;

use chrono::{DateTime, Local};
use parking_lot::Mutex;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum LogLevel {
    Debug,
    Info,
    Success,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Success => "SUCCESS",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
        }
    }

    pub fn parse(value: &str) -> LogLevel {
        match value.to_ascii_uppercase().as_str() {
            "DEBUG" => LogLevel::Debug,
            "SUCCESS" => LogLevel::Success,
            "WARN" | "WARNING" => LogLevel::Warn,
            "ERROR" => LogLevel::Error,
            _ => LogLevel::Info,
        }
    }

    fn weight(&self) -> u8 {
        match self {
            LogLevel::Debug => 0,
            LogLevel::Info => 1,
            LogLevel::Success => 1,
            LogLevel::Warn => 2,
            LogLevel::Error => 3,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub ts: DateTime<Local>,
    pub level: LogLevel,
    pub source: String,
    pub message: String,
}

pub type LogSubscriber = Arc<dyn Fn(&LogEntry) + Send + Sync>;

pub struct LogBuffer {
    entries: Mutex<VecDeque<LogEntry>>,
    subscribers: Mutex<Vec<LogSubscriber>>,
    min_level: Mutex<LogLevel>,
    /// Threads currently inside the fan-out section: a nested `log()` from a
    /// subscriber callback is recorded in the ring but must not re-enter the
    /// fan-out, or a callback that logs would recurse without bound.
    notifying: Mutex<HashSet<ThreadId>>,
    capacity: usize,
}

impl LogBuffer {
    pub fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            entries: Mutex::new(VecDeque::with_capacity(capacity.min(4096))),
            subscribers: Mutex::new(Vec::new()),
            min_level: Mutex::new(LogLevel::Debug),
            notifying: Mutex::new(HashSet::new()),
            capacity,
        })
    }

    pub fn set_min_level(&self, level: LogLevel) {
        *self.min_level.lock() = level;
    }

    pub fn subscribe(&self, callback: LogSubscriber) {
        self.subscribers.lock().push(callback);
    }

    pub fn log(&self, level: LogLevel, source: impl Into<String>, message: impl Into<String>) {
        if level.weight() < self.min_level.lock().weight() {
            return;
        }
        let entry = LogEntry {
            ts: Local::now(),
            level,
            source: source.into(),
            message: message.into(),
        };
        {
            let mut entries = self.entries.lock();
            if entries.len() >= self.capacity {
                entries.pop_front();
            }
            entries.push_back(entry.clone());
        }
        // Fan out OUTSIDE the entries lock: a subscriber that (un)subscribes
        // from within the callback would otherwise deadlock on the
        // non-reentrant subscribers mutex. Clone the list under a short lock,
        // release it, then invoke the callbacks.
        let tid = std::thread::current().id();
        {
            let mut notifying = self.notifying.lock();
            if !notifying.insert(tid) {
                // Nested call from a subscriber callback on this thread: the
                // entry is already stored in the ring above; skip re-fanning
                // out to bound the recursion depth at one level.
                return;
            }
        }
        struct FanOutGuard<'a>(&'a Mutex<HashSet<ThreadId>>, ThreadId);
        impl Drop for FanOutGuard<'_> {
            fn drop(&mut self) {
                self.0.lock().remove(&self.1);
            }
        }
        let _guard = FanOutGuard(&self.notifying, tid);
        let subscribers = self.subscribers.lock().clone();
        for sub in subscribers {
            sub(&entry);
        }
    }

    pub fn snapshot(&self, limit: usize) -> Vec<LogEntry> {
        let entries = self.entries.lock();
        let start = entries.len().saturating_sub(limit.max(1));
        entries.iter().skip(start).cloned().collect()
    }

    pub fn clear(&self) {
        self.entries.lock().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn ring_buffer_caps_and_notifies() {
        let buffer = LogBuffer::new(3);
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        buffer.subscribe(Arc::new(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        }));
        for i in 0..5 {
            buffer.log(LogLevel::Info, "test", format!("msg {i}"));
        }
        let snap = buffer.snapshot(10);
        assert_eq!(snap.len(), 3);
        assert_eq!(snap[0].message, "msg 2");
        assert_eq!(hits.load(Ordering::SeqCst), 5);
    }

    #[test]
    fn level_filter() {
        let buffer = LogBuffer::new(10);
        buffer.set_min_level(LogLevel::Warn);
        buffer.log(LogLevel::Info, "t", "hidden");
        buffer.log(LogLevel::Error, "t", "shown");
        assert_eq!(buffer.snapshot(10).len(), 1);
    }

    #[test]
    fn subscriber_may_log_recursively() {
        // Regression: a callback that logs back into the buffer used to run
        // while the subscribers mutex was held → deadlock; and without the
        // re-entrancy guard the recursion would be unbounded (stack
        // overflow). Now: nested entry lands in the ring, fan-out stays at
        // depth one.
        let buffer = LogBuffer::new(10);
        let reentrant: Arc<LogBuffer> = buffer.clone();
        buffer.subscribe(Arc::new(move |entry: &LogEntry| {
            reentrant.log(LogLevel::Warn, "reentrant", &entry.message);
        }));
        buffer.log(LogLevel::Info, "test", "hello");
        let snap = buffer.snapshot(10);
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].message, "hello");
        assert_eq!(snap[0].source, "test");
        assert_eq!(snap[1].source, "reentrant");
        assert_eq!(snap[1].message, "hello");
    }
}
