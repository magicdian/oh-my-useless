use std::fs;
use std::path::Path;
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::adapters::cgroups::{self, ScopeSet};
use crate::adapters::psi::{cgroup_specs, host_specs};
use crate::adapters::sessions::{self, LegacySessionSampler, SessionMonitor, SessionRuntime};
use crate::adapters::{ProcfsSampler, PsiCapability, PsiSource, RealPsiOps, SignalSource};
use crate::config::{Config, JOURNAL_FILE_PATH, PID_FILE_PATH, STATE_FILE_PATH, load_config};
use crate::domain::{
    Mode, PolicyOutput, PolicyState, PressureClass, ReasonCode, Signal, reduce, sampler_cadence,
};
use crate::error::{AppResult, boxed_error};
use crate::ports::{Clock, Sampler, SystemClock};
use crate::runtime::journal::{Journal, StateSnapshot, Transition, write_state};
use crate::runtime::{Reactor, Store};
use crate::sys;
use crate::workers;

static TERMINATE_FLAG: AtomicBool = AtomicBool::new(false);
static RELOAD_FLAG: AtomicBool = AtomicBool::new(false);

const SIGHUP: i32 = 1;
const SIGINT: i32 = 2;
const SIGTERM: i32 = 15;

unsafe extern "C" {
    #[cfg(not(target_os = "linux"))]
    fn signal(signum: i32, handler: extern "C" fn(i32)) -> usize;
    fn kill(pid: i32, sig: i32) -> i32;
}

pub fn run(config_path: &Path) -> AppResult<()> {
    #[cfg(target_os = "linux")]
    sys::block_signals(&[SIGHUP, SIGINT, SIGTERM])?;
    #[cfg(not(target_os = "linux"))]
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

    let capability = Arc::new(Mutex::new(PsiCapability::default()));
    let mut current_config = config;
    let ReactorBundle {
        mut reactor,
        mut scopes,
        mut session_monitor,
        mut legacy_sessions,
    } = build_reactor(&current_config, &capability)?;
    let mut capabilities = Capabilities::detect(&capability, &scopes)
        .with_session_source(session_source(&session_monitor));

    let mut journal = Journal::with_file(512, Path::new(JOURNAL_FILE_PATH).to_path_buf());
    let mut last_summary = String::new();
    let mut last_mode = Mode::Suspended;
    let mut last_transition = "startup".to_string();
    let mut last_state_write: Option<Instant> = None;
    let mut last_scope_refresh = Instant::now();
    let mut next_sample = Instant::now();

    eprintln!(
        "[useless] daemon started with sensor backend {} and config {}",
        sampler.backend_name(),
        config_path.display()
    );
    eprintln!("[useless] sensors: {}", capabilities.label());

    while !TERMINATE_FLAG.load(Ordering::Relaxed) {
        if RELOAD_FLAG.swap(false, Ordering::Relaxed) {
            match load_config(config_path) {
                Ok(updated) => {
                    current_config = Arc::new(updated);
                    store.replace_config(Arc::clone(&current_config));
                    let bundle = build_reactor(&current_config, &capability)?;
                    reactor = bundle.reactor;
                    scopes = bundle.scopes;
                    session_monitor = bundle.session_monitor;
                    legacy_sessions = bundle.legacy_sessions;
                    capabilities = Capabilities::detect(&capability, &scopes)
                        .with_session_source(session_source(&session_monitor));
                    last_summary.clear();
                    last_scope_refresh = Instant::now();
                    next_sample = clock.now();
                    eprintln!("[useless] reloaded config from {}", config_path.display());
                    eprintln!("[useless] sensors: {}", capabilities.label());
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

        if last_scope_refresh.elapsed() >= Duration::from_secs(60) {
            let cgroup_root = Path::new("/sys/fs/cgroup");
            let own = cgroups::own_cgroup(cgroup_root);
            let next = cgroups::discover(&current_config.sensor.watch_slices, cgroup_root, &own);
            if next.watched != scopes.watched || next.missing != scopes.missing {
                let bundle = build_reactor(&current_config, &capability)?;
                reactor = bundle.reactor;
                scopes = bundle.scopes;
                session_monitor = bundle.session_monitor;
                legacy_sessions = bundle.legacy_sessions;
                capabilities = Capabilities::detect(&capability, &scopes)
                    .with_session_source(session_source(&session_monitor));
                eprintln!(
                    "[useless] cgroup scopes refreshed: {}",
                    capabilities.label()
                );
            }
            last_scope_refresh = Instant::now();
        }

        let sample_due = next_sample;
        let mut wait_until = next_sample;
        if let Some(monitor) = &session_monitor {
            if let Some(due) = monitor.next_deadline() {
                wait_until = due.min(wait_until);
            }
        }

        let mut batch = reactor.wait(wait_until)?;
        if TERMINATE_FLAG.load(Ordering::Relaxed) {
            break;
        }

        let now = clock.now();
        if let Some(signal) = session_monitor
            .as_ref()
            .and_then(|monitor| monitor.initial())
        {
            batch.push(signal);
        }
        if now >= sample_due {
            match sampler.sample(current_config.global.active_tty_idle_secs) {
                Ok(mut sample) => batch.append(&mut sample),
                Err(error) => batch.push(Signal::SourceDegraded {
                    source: sampler.backend_name().to_string(),
                    message: error.to_string(),
                }),
            }
            if let Some(legacy) = &mut legacy_sessions {
                batch.push(legacy.sample());
            }
        }
        if let Some(monitor) = &session_monitor {
            if let Some(signal) = monitor.probe(now) {
                batch.push(signal);
            }
        }

        if batch.is_empty() {
            continue;
        }

        let has_pressure = batch
            .iter()
            .any(|signal| matches!(signal, Signal::Pressure { .. }));
        let mut labels = Vec::with_capacity(batch.len());
        let mut output = None;

        for signal in &batch {
            labels.push(signal_name(signal));
            output = Some(reduce(&mut policy, signal, &current_config, now));
        }

        let Some(output) = output else {
            continue;
        };
        let label = labels.join("+");
        let available = mem_available_bytes(&policy);

        if output.summary != last_summary {
            eprintln!("[useless] {}", output.summary);
            last_summary = output.summary.clone();
        }

        let mode_before = last_mode;
        let mode_changed = output.mode != mode_before;
        if mode_changed {
            journal.record(Transition {
                at: SystemTime::now(),
                signal: label.clone(),
                from: mode_before,
                to: output.mode,
                reasons: output.reasons.clone(),
                budget_bytes: output.budget.target_bytes,
                mem_available_bytes: available,
            });
            last_transition = format!("{} -> {} ({label})", mode_before.key(), output.mode.key());
            last_mode = output.mode;
        }

        let due_state = last_state_write
            .map(|at| at.elapsed() >= Duration::from_secs(30))
            .unwrap_or(true);
        if mode_changed || due_state {
            let snapshot = state_snapshot(
                &output,
                &capabilities,
                &last_transition,
                available,
                policy.last_pressure_scope.as_deref(),
            );
            let _ = write_state(Path::new(STATE_FILE_PATH), &snapshot);
            last_state_write = Some(Instant::now());
        }

        let output_mode = output.mode;
        store.publish(output);

        let base_secs = current_config.global.check_interval_secs.max(1);
        next_sample = if has_pressure {
            now + Duration::from_secs(base_secs)
        } else {
            now + sampler_cadence(output_mode, base_secs)
        };
    }

    store.shutdown();
    for handle in worker_handles {
        let _ = handle.join();
    }
    drop(pid_guard);

    eprintln!("[useless] daemon stopped");
    Ok(())
}

struct ReactorBundle {
    reactor: Reactor,
    scopes: ScopeSet,
    session_monitor: Option<SessionMonitor>,
    legacy_sessions: Option<LegacySessionSampler>,
}

fn build_reactor(
    config: &Config,
    capability: &Arc<Mutex<PsiCapability>>,
) -> AppResult<ReactorBundle> {
    let mut reactor = Reactor::new()?;
    reactor.add(Box::new(SignalSource::new(
        2,
        &TERMINATE_FLAG,
        &RELOAD_FLAG,
    )?))?;

    let cgroup_root = Path::new("/sys/fs/cgroup");
    let own = cgroups::own_cgroup(cgroup_root);
    let scopes = cgroups::discover(&config.sensor.watch_slices, cgroup_root, &own);
    if !scopes.missing.is_empty() {
        eprintln!(
            "[useless] cgroup scopes missing: {}",
            scopes.missing.join(",")
        );
    }

    let mut specs = host_specs();
    for path in &scopes.watched {
        let name = cgroups::slice_name(path);
        specs.extend(cgroup_specs(&name, path));
    }

    let psi = PsiSource::new(
        Arc::new(RealPsiOps),
        config.global.pressure_enter_ms,
        config.global.pressure_window_ms,
        Arc::clone(capability),
        specs,
    );
    reactor.add(Box::new(psi))?;

    let (session_monitor, legacy_sessions) =
        match sessions::start(3, config.global.active_tty_idle_secs) {
            SessionRuntime::Event { source, monitor } => {
                reactor.add(source)?;
                (Some(monitor), None)
            }
            SessionRuntime::Legacy(sampler) => (None, Some(sampler)),
        };

    Ok(ReactorBundle {
        reactor,
        scopes,
        session_monitor,
        legacy_sessions,
    })
}

#[derive(Debug, Clone)]
struct Capabilities {
    psi: PsiCapability,
    cgroup_scopes: usize,
    missing_slices: Vec<String>,
    cgroup_v2: bool,
    inotify: bool,
    session_source: &'static str,
}

impl Capabilities {
    fn detect(capability: &Arc<Mutex<PsiCapability>>, scopes: &ScopeSet) -> Self {
        Self {
            psi: capability.lock().expect("psi capability").clone(),
            cgroup_scopes: scopes.watched.len(),
            missing_slices: scopes.missing.clone(),
            cgroup_v2: Path::new("/sys/fs/cgroup/cgroup.controllers").exists(),
            inotify: sys::inotify_probe(),
            session_source: "unknown",
        }
    }

    fn with_session_source(mut self, source: &'static str) -> Self {
        self.session_source = source;
        self
    }

    fn label(&self) -> String {
        let cgroups = if self.missing_slices.is_empty() {
            format!("cgroups={}", self.cgroup_scopes)
        } else {
            format!(
                "cgroups=degraded(missing: {})",
                self.missing_slices.join(",")
            )
        };
        format!(
            "{} {cgroups} cgroup_v2={} inotify={} sessions={}",
            self.psi.label(),
            status(self.cgroup_v2),
            status(self.inotify),
            self.session_source
        )
    }
}

fn session_source(monitor: &Option<SessionMonitor>) -> &'static str {
    if monitor.is_some() { "inotify" } else { "who" }
}

fn status(value: bool) -> &'static str {
    if value { "ok" } else { "unavailable" }
}

fn state_snapshot(
    output: &PolicyOutput,
    capabilities: &Capabilities,
    last_transition: &str,
    available: u64,
    scope: Option<&str>,
) -> StateSnapshot {
    StateSnapshot {
        mode: output.mode.key(),
        reasons: output
            .reasons
            .iter()
            .map(|reason| reason.label())
            .collect::<Vec<_>>()
            .join(","),
        scope: scope.unwrap_or("").to_string(),
        summary: output.summary.clone(),
        budget_mb: output.budget.target_bytes / (1024 * 1024),
        reserve_mb: output.budget.reserve_bytes / (1024 * 1024),
        available_mb: available / (1024 * 1024),
        capabilities: capabilities.label(),
        last_transition: last_transition.to_string(),
    }
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
        Signal::Pressure { class, .. } => match class {
            PressureClass::Cpu => "psi:cpu",
            PressureClass::Memory => "psi:memory",
            PressureClass::Io => "psi:io",
        },
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

#[cfg(not(target_os = "linux"))]
fn install_signal_handlers() {
    unsafe {
        signal(SIGHUP, handle_signal);
        signal(SIGINT, handle_signal);
        signal(SIGTERM, handle_signal);
    }
}

#[cfg(not(target_os = "linux"))]
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
