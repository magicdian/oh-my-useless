use std::fs;
use std::path::Path;
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use crate::config::{Config, PID_FILE_PATH, load_config};
use crate::error::{AppResult, boxed_error};
use crate::policy::Supervisor;
use crate::runtime::{RuntimeState, SharedRuntime};
use crate::sensor::{ProcfsSensor, SensorBackend};
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
    let config = load_config(config_path)?;
    let mut sensor = ProcfsSensor::new();
    let shared = RuntimeState::shared(config.clone());
    let pid_guard = PidGuard::acquire(Path::new(PID_FILE_PATH))?;
    let worker_handles = workers::spawn_all(shared.clone());
    let mut supervisor = Supervisor::new();
    let mut last_summary = String::new();

    eprintln!(
        "[useless] daemon started with sensor backend {} and config {}",
        sensor.backend_name(),
        config_path.display()
    );

    let mut current_config = config;

    while !TERMINATE_FLAG.load(Ordering::Relaxed) {
        if RELOAD_FLAG.swap(false, Ordering::Relaxed) {
            match load_config(config_path) {
                Ok(updated) => {
                    eprintln!("[useless] reloaded config from {}", config_path.display());
                    current_config = updated;
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

        match sensor.sample(current_config.global.active_tty_idle_secs) {
            Ok(sample) => {
                let policy = supervisor.evaluate(&sample, &current_config, Instant::now());
                if policy.summary != last_summary {
                    eprintln!("[useless] {}", policy.summary);
                    last_summary = policy.summary.clone();
                }
                update_runtime_state(&shared, current_config.clone(), sample, policy);
            }
            Err(error) => {
                let mut state = shared.write().expect("daemon write lock");
                state.policy = crate::policy::PolicySnapshot::paused(format!(
                    "sensor backend failed: {error}"
                ));
                state.shutdown = false;
                state.last_updated = SystemTime::now();
            }
        }

        sleep_with_interrupts(Duration::from_secs(
            current_config.global.check_interval_secs,
        ));
    }

    {
        let mut state = shared.write().expect("daemon write lock");
        state.shutdown = true;
    }
    for handle in worker_handles {
        let _ = handle.join();
    }
    drop(pid_guard);

    eprintln!("[useless] daemon stopped");
    Ok(())
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

fn update_runtime_state(
    shared: &SharedRuntime,
    config: Config,
    sample: crate::sensor::SensorSample,
    policy: crate::policy::PolicySnapshot,
) {
    let mut state = shared.write().expect("daemon write lock");
    state.config = config;
    state.sensor_sample = sample;
    state.policy = policy;
    state.last_updated = SystemTime::now();
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
