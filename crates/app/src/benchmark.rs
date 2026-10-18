//! Strategy benchmark: iterate every `general*.bat` of a profile,
//! score each one with the diagnostic checks and restore the original state.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use parking_lot::RwLock;
use serde::Serialize;

use integra_diag::CheckKind;
use integra_zapret::{Status, ZapretManager};

const WARMUP: Duration = Duration::from_millis(2500);
const CHECK_TIMEOUT: Duration = Duration::from_secs(4);
pub const MAX_BENCH_SITES: usize = 6;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BenchPhase {
    Idle,
    Starting,
    Warmup,
    Checking,
    Restoring,
    Done,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct BenchProgress {
    pub phase: BenchPhase,
    pub strategy: String,
    pub index: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct BenchEntry {
    pub strategy: String,
    pub score: u32,
    pub success_ratio: u32,
    pub avg_latency_ms: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BenchResult {
    pub entries: Vec<BenchEntry>,
    pub best: Option<String>,
    pub cancelled: bool,
    pub restored_strategy: Option<String>,
}

pub struct Benchmark {
    pub running: AtomicBool,
    pub cancel: AtomicBool,
    pub progress: RwLock<BenchProgress>,
    pub result: Mutex<Option<BenchResult>>,
}

impl Default for Benchmark {
    fn default() -> Self {
        Self {
            running: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            progress: RwLock::new(BenchProgress {
                phase: BenchPhase::Idle,
                strategy: String::new(),
                index: 0,
                total: 0,
            }),
            result: Mutex::new(None),
        }
    }
}

impl Benchmark {
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn progress(&self) -> BenchProgress {
        self.progress.read().clone()
    }

    pub fn result(&self) -> Option<BenchResult> {
        self.result.lock().clone()
    }
}

fn check_sites(hosts: &[(String, String, u16)]) -> (usize, usize, u64) {
    // (passed, total, latency sum over passed TCP/TLS/HTTP probes)
    let mut passed = 0usize;
    let mut total = 0usize;
    let mut latency_sum = 0u64;
    for (_, host, port) in hosts {
        let site = integra_diag::Site { id: String::new(), name: String::new(), host: host.clone(), port: *port };
        let outcomes = integra_diag::check_site(
            &site,
            &[CheckKind::Tcp, CheckKind::Tls, CheckKind::Http],
            1,
            CHECK_TIMEOUT,
        );
        for outcome in outcomes {
            total += 1;
            if outcome.ok {
                passed += 1;
                if let Some(stats) = outcome.latency {
                    latency_sum += stats.avg_ms;
                }
            }
        }
    }
    (passed, total, latency_sum)
}

/// Run the whole benchmark in the background. `emit` receives (phase, strategy, index, total).
pub fn spawn(
    manager: Arc<ZapretManager>,
    strategies: Vec<String>,
    sites: Vec<(String, String, u16)>,
    on_event: impl Fn(BenchProgress) + Send + Sync + 'static,
) -> Arc<Benchmark> {
    let bench = Arc::new(Benchmark::default());
    bench.running.store(true, Ordering::Relaxed);
    {
        let mut progress = bench.progress.write();
        progress.phase = BenchPhase::Starting;
        progress.total = strategies.len() as u32;
    }

    let bench_ref = bench.clone();
    std::thread::Builder::new()
        .name("integra-benchmark".into())
        .spawn(move || {
            let original_strategy = manager.current_strategy();
            let was_running = manager.status() == Status::Running;

            let mut entries: Vec<BenchEntry> = Vec::new();
            let total = strategies.len() as u32;
            'outer: for (index, strategy) in strategies.iter().enumerate() {
                let index = index as u32 + 1;

                let set_phase = |phase: BenchPhase| {
                    let mut progress = bench_ref.progress.write();
                    progress.phase = phase;
                    progress.strategy = strategy.clone();
                    progress.index = index;
                };
                let notify = |phase: BenchPhase| {
                    set_phase(phase);
                    on_event(bench_ref.progress());
                };

                notify(BenchPhase::Starting);
                if bench_ref.cancel.load(Ordering::Relaxed) {
                    break 'outer;
                }

                let _ = manager.stop();
                if let Err(err) = manager.start(strategy) {
                    entries.push(BenchEntry {
                        strategy: strategy.clone(),
                        score: 0,
                        success_ratio: 0,
                        avg_latency_ms: None,
                        error: Some(crate::CommandError::from(err).message),
                    });
                    continue;
                }

                notify(BenchPhase::Warmup);
                for _ in 0..WARMUP.as_millis() as u32 / 100 {
                    if bench_ref.cancel.load(Ordering::Relaxed) {
                        break 'outer;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }

                notify(BenchPhase::Checking);
                let (passed, total_checks, latency_sum) = check_sites(&sites);
                let success_ratio = if total_checks == 0 {
                    0
                } else {
                    (passed * 100 / total_checks) as u32
                };
                let latency_score: u32 = if passed == 0 {
                    0
                } else {
                    let avg = latency_sum / passed.max(1) as u64;
                    // 40 ms → 100 points, 6 s+ → 0 points.
                    let over = avg.saturating_sub(40);
                    let penalty = (over * 100 / 5960).min(100);
                    (100 - penalty) as u32
                };
                let score = (success_ratio as u64 * 80 / 100 + latency_score as u64 * 20 / 100) as u32;
                entries.push(BenchEntry {
                    strategy: strategy.clone(),
                    score,
                    success_ratio,
                    avg_latency_ms: (passed > 0).then(|| latency_sum / passed.max(1) as u64),
                    error: None,
                });
            }

            // Restore the original state.
            {
                let mut progress = bench_ref.progress.write();
                progress.phase = BenchPhase::Restoring;
            }
            on_event(bench_ref.progress());
            let _ = manager.stop();
            if was_running {
                if let Some(strategy) = original_strategy.as_ref() {
                    let _ = manager.start(strategy);
                }
            }

            let cancelled = bench_ref.cancel.load(Ordering::Relaxed);
            let best = entries
                .iter()
                .filter(|e| e.error.is_none())
                .max_by_key(|e| (e.score, e.success_ratio))
                .map(|e| e.strategy.clone());

            {
                let mut progress = bench_ref.progress.write();
                progress.phase = if cancelled { BenchPhase::Cancelled } else { BenchPhase::Done };
            }
            *bench_ref.result.lock() = Some(BenchResult {
                entries,
                best,
                cancelled,
                restored_strategy: original_strategy,
            });
            bench_ref.running.store(false, Ordering::Relaxed);
            on_event(bench_ref.progress());
        })
        .ok();
    bench
}
