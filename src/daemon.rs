use std::fs;
use std::path::Path;
use std::process;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::adapters::ProcfsSampler;
use crate::config::{PID_FILE_PATH, load_config};
use crate::domain::{Mode, PolicyOutput, PolicyState, ReasonCode, Signal, reduce};
use crate::error::{AppResult, boxed_error};
use crate::ports::{Clock, Sampler, SystemClock};
use crate::runtime::Store;
use crate::runtime::journal::{Journal, Transition};
use crate::workers;

static TERMINATE_FLAG: AtomicBool = AtomicBool::new(false);
static RELOAD_FLAG: AtomicBool = AtomicBool::new(false);

const SIGHUP: i32 = 1;
const SIGINT: i32 = 2;
const SIGTERM: i32 = 15;

unsafe extern "C" {
    fn signal(signum: i32, handler: extern "C" fn(i32)) -> usize;
    fn kill(pid: i32, sig: i32) -> i32;
}

pub fn run(config_path: &Path) -> AppResult<()> {
    install_signal_handlers();

    let config = Arc::new(load_config(config_path)?);
    let clock = SystemClock;
    let mut sampler = ProcfsSampler::new();
    let mut policy = PolicyState::new(seed_now());
    let store = Store::new(
        config.clone(),
        PolicyOutput::suspended(
            ReasonCode::CollectorDegraded,
            "waiting for the first supervisor sample",
        ),
    );

    let pid_guard = PidGuard::acquire(Path::new(PID_FILE_PATH))?;
    let worker_handles = workers::spawn_all(store.clone());

    let mut current_config = config;
    let mut journal = Journal::new(512);
    let mut last_summary = String::new();
    let mut last_mode = Mode::Suspended;

    eprintln!(
        "[useless] daemon started with sensor backend {} and config {}",
        sampler.backend_name(),
        config_path.display()
    );

    while !TERMINATE_FLAG.load(Ordering::Relaxed) {
        if RELOAD_FLAG.swap(false, Ordering::Relaxed) {
            match load_config(config_path) {
                Ok(updated) => {
                    current_config = Arc::new(updated);
                    store.replace_config(current_config.clone());
                    eprintln!("[useless] reloaded config from {}", config_path.display());
                }
                Err(error) => {
                    eprintln!(
                        "[useless] failed to reload config from {}: {}",
                        config_path.display(),
                        error
                    );
                }
            }
        }

        let now = clock.now();
        match sampler.sample(current_config.global.active_tty_idle_secs) {
            Ok(signals) => {
                let mut labels = Vec::with_capacity(signals.len());
                let mut output = None;

                for signal in &signals {
                    labels.push(signal_name(signal));
                    output = Some(reduce(&mut policy, signal, &current_config, now));
                }

                if let Some(output) = output {
                    let label = labels.join("+");
                    publish(
                        &store,
                        &mut journal,
                        &mut last_summary,
                        &mut last_mode,
                        output,
                        &label,
                        mem_available_bytes(&policy),
                    );
                }
            }
            Err(error) => {
                let signal = Signal::SourceDegraded {
                    source: sampler.backend_name().to_string(),
                    message: error.to_string(),
                };
                let output = reduce(&mut policy, &signal, &current_config, now);
                publish(
                    &store,
                    &mut journal,
                    &mut last_summary,
                    &mut last_mode,
                    output,
                    signal_name(&signal),
                    mem_available_bytes(&policy),
                );
            }
        }

        sleep_with_interrupts(Duration::from_secs(
            current_config.global.check_interval_secs,
        ));
    }

    store.shutdown();
    for handle in worker_handles {
        let _ = handle.join();
    }
    drop(pid_guard);

    eprintln!("[useless] daemon stopped");
    Ok(())
}

fn publish(
    store: &Store,
    journal: &mut Journal,
    last_summary: &mut String,
    last_mode: &mut Mode,
    output: PolicyOutput,
    signal: &str,
    mem_available_bytes: u64,
) {
    if output.summary != *last_summary {
        eprintln!("[useless] {}", output.summary);
        *last_summary = output.summary.clone();
    }

    if output.mode != *last_mode {
        journal.record(Transition {
            at: SystemTime::now(),
            signal: signal.to_string(),
            from: *last_mode,
            to: output.mode,
            reasons: output.reasons.clone(),
            budget_bytes: output.budget.target_bytes,
            mem_available_bytes,
        });
        *last_mode = output.mode;
    }

    store.publish(output);
}

fn mem_available_bytes(policy: &PolicyState) -> u64 {
    policy
        .last_sample
        .as_ref()
        .map(|sample| sample.mem_available_bytes)
        .unwrap_or(0)
}

fn signal_name(signal: &Signal) -> &'static str {
    match signal {
        Signal::Sample(_) => "sample",
        Signal::Sessions(_) => "sessions",
        Signal::SourceDegraded { .. } => "source_degraded",
    }
}

pub fn send_reload_from_pidfile() -> AppResult<()> {
    let pid_text = fs::read_to_string(PID_FILE_PATH)?;
    let pid = pid_text.trim().parse::<i32>()?;
    let rc = unsafe { kill(pid, SIGHUP) };
    if rc != 0 {
        return Err(boxed_error("failed to signal the useless daemon"));
    }
    Ok(())
}

fn sleep_with_interrupts(duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if TERMINATE_FLAG.load(Ordering::Relaxed) || RELOAD_FLAG.load(Ordering::Relaxed) {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn install_signal_handlers() {
    unsafe {
        signal(SIGHUP, handle_signal);
        signal(SIGINT, handle_signal);
        signal(SIGTERM, handle_signal);
    }
}

extern "C" fn handle_signal(signal_number: i32) {
    match signal_number {
        SIGHUP => RELOAD_FLAG.store(true, Ordering::Relaxed),
        SIGINT | SIGTERM => TERMINATE_FLAG.store(true, Ordering::Relaxed),
        _ => {}
    }
}

fn seed_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(1)
}

struct PidGuard {
    path: &'static Path,
}

impl PidGuard {
    fn acquire(path: &'static Path) -> AppResult<Self> {
        if let Ok(existing) = fs::read_to_string(path) {
            if let Ok(pid) = existing.trim().parse::<u32>() {
                let proc_path = format!("/proc/{pid}");
                if Path::new(&proc_path).exists() {
                    return Err(boxed_error(format!(
                        "pid file {} already points to a running process",
                        path.display()
                    )));
                }
            }
        }

        fs::write(path, process::id().to_string())?;
        Ok(Self { path })
    }
}

impl Drop for PidGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.path);
    }
}
