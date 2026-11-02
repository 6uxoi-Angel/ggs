//! Remote control agent lifecycle inside the Integra app.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

pub struct RemoteCtrl {
    running: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
    pub last_log: Arc<Mutex<String>>,
}

impl RemoteCtrl {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            thread: Mutex::new(None),
            last_log: Arc::new(Mutex::new(String::new())),
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Spawn the agent loop. Returns false if a thread could not be started.
    pub fn start(&self, servers: Vec<String>, token: String) -> bool {
        if self.is_running() {
            return true;
        }
        self.running.store(true, Ordering::Relaxed);
        let running = self.running.clone();
        let err_flag = self.running.clone();
        let last_log = self.last_log.clone();
        let cfg = integra_remote::AgentCfg {
            servers,
            token,
            prog_dir: integra_remote::DEFAULT_PROG_DIR.into(),
            foreground_log: false,
            stop: running.clone(),
        };        let handle = std::thread::Builder::new()
            .name("integra-remote".into())
            .spawn(move || {
                let _ = last_log.lock().map(|mut l| *l = "starting".into());
                integra_remote::run_agent(cfg);
                running.store(false, Ordering::Relaxed);
            });
        match handle {
            Ok(h) => {
                *self.thread.lock().unwrap() = Some(h);
                true
            }
            Err(e) => {
                err_flag.store(false, Ordering::Relaxed);
                let _ = self.last_log.lock().map(|mut l| *l = format!("thread error: {e}"));
                false
            }
        }
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(h) = self.thread.lock().unwrap().take() {
            let _ = h.join();
        }
    }
}

impl Default for RemoteCtrl {
    fn default() -> Self {
        Self::new()
    }
}
