use std::time::{Duration, Instant};

use crate::config::Config;
use crate::domain::budget::{BudgetInput, compute};
use crate::domain::signal::{PressureClass, PressureScope, Signal};
use crate::domain::state::{Mode, ModuleDirective, PolicyOutput, PolicyState, ReasonCode};

/// Pure policy transition. No filesystem, clock, or logging access.
pub fn reduce(
    state: &mut PolicyState,
    signal: &Signal,
    config: &Config,
    now: Instant,
) -> PolicyOutput {
    match signal {
        Signal::Sample(sample) => {
            state.last_sample = Some(sample.clone());
            state.degraded = false;
        }
        Signal::Sessions(sessions) => {
            state.sessions = sessions.clone();
            if !sessions.is_empty() {
                state.last_interactive = Some(now);
            }
        }
        Signal::Pressure { class, scope, .. } => {
            state.last_pressure = Some(now);
            state.pressure_reason = Some(match class {
                PressureClass::Cpu => ReasonCode::LoadPressure,
                PressureClass::Memory => ReasonCode::MemoryPressure,
                PressureClass::Io => ReasonCode::IoPressure,
            });
            state.last_pressure_scope = match scope {
                PressureScope::Host => None,
                PressureScope::Cgroup(name) => Some(name.clone()),
            };
        }
        Signal::SourceDegraded { .. } => {
            state.degraded = true;
        }
    }

    let interactive_active = state
        .last_interactive
        .map(|last| {
            now.duration_since(last) < Duration::from_secs(config.global.interactive_grace_secs)
        })
        .unwrap_or(false);

    let pressure_signal = matches!(signal, Signal::Pressure { .. });
    let pressure_active = pressure_signal
        || state
            .last_pressure
            .map(|last| {
                now.duration_since(last) < Duration::from_secs(config.global.resume_quiet_secs)
            })
            .unwrap_or(false);
    if !pressure_active {
        state.last_pressure = None;
        state.pressure_reason = None;
        state.last_pressure_scope = None;
    }

    let sample = state.last_sample.clone().unwrap_or_default();
    let mut reasons = Vec::new();

    let mode = if state.degraded {
        reasons.push(ReasonCode::CollectorDegraded);
        Mode::Suspended
    } else if interactive_active {
        reasons.push(ReasonCode::InteractiveSession);
        Mode::Suspended
    } else if pressure_active {
        reasons.push(state.pressure_reason.unwrap_or(ReasonCode::LoadPressure));
        Mode::Suspended
    } else {
        let load_pause = sample.load_percent >= config.global.pause_load_percent;
        let memory_pause =
            sample.mem_available_percent <= config.global.pause_mem_available_percent;
        let load_caution = sample.load_percent >= config.global.caution_load_percent;
        let memory_caution =
            sample.mem_available_percent <= config.global.caution_mem_available_percent;

        if load_pause || memory_pause {
            if load_pause {
                reasons.push(ReasonCode::LoadPressure);
            }
            if memory_pause {
                reasons.push(ReasonCode::MemoryPressure);
            }
            Mode::Suspended
        } else if load_caution || memory_caution {
            if load_caution {
                reasons.push(ReasonCode::LoadPressure);
            }
            if memory_caution {
                reasons.push(ReasonCode::MemoryPressure);
            }
            Mode::Cooling
        } else {
            Mode::Running
        }
    };

    let was_suspended = state.mode == Mode::Suspended;
    let mut duty_percent = match mode {
        Mode::Running => 100,
        Mode::Cooling => config.global.caution_duty_percent,
        Mode::Suspended => 0,
    };

    if mode == Mode::Suspended {
        state.resumed_at = None;
    } else {
        if was_suspended {
            state.resumed_at = Some(now);
        }
        if let Some(resumed_at) = state.resumed_at {
            let ramp = Duration::from_secs(config.global.resume_ramp_secs);
            if ramp.is_zero() {
                state.resumed_at = None;
            } else {
                let elapsed = now.duration_since(resumed_at);
                if elapsed >= ramp {
                    state.resumed_at = None;
                } else {
                    let fraction = elapsed.as_secs_f64() / ramp.as_secs_f64();
                    let factor = if fraction < 1.0 / 3.0 {
                        25
                    } else if fraction < 2.0 / 3.0 {
                        50
                    } else {
                        100
                    };
                    duty_percent = ((duty_percent as u32 * factor) / 100) as u8;
                }
            }
        }
    }

    // Advance every schedule on every evaluation, including while suspended.
    // Freezing the phase caused an immediate full-size burst on resume.
    let cpu_phase = state.cpu_schedule.is_active(now, &config.cpu.window);
    let memory_phase = state.memory_schedule.is_active(now, &config.memory.window);
    let network_phase = state
        .network_schedule
        .is_active(now, &config.network.window);

    // Child 3 consumes this budget with the chunked ledger.
    let budget = compute(&BudgetInput {
        mem_total: sample.mem_total_bytes,
        mem_available: sample.mem_available_bytes,
        max_percent: config.memory.max_percent,
        duty_percent,
        max_absolute_mb: config.memory.max_absolute_mb,
        reserve_mb: config.global.reserve_mb,
        reserve_percent: config.global.reserve_percent,
    });

    let summary = match signal {
        Signal::SourceDegraded { message, .. } => {
            format!("sensor backend failed: {message}")
        }
        _ if interactive_active => format!(
            "interactive session detected (tty={}, ssh={})",
            state.sessions.tty_count(),
            state.sessions.ssh_count()
        ),
        _ => {
            let scope = state
                .last_pressure_scope
                .as_deref()
                .map(|scope| format!(" scope={scope}"))
                .unwrap_or_default();
            format!(
                "pressure={} load={} mem_avail={} net_rx={}B/s net_tx={}B/s{scope}",
                mode.label(),
                sample.load_percent,
                sample.mem_available_percent,
                sample.net_rx_bytes_per_sec,
                sample.net_tx_bytes_per_sec
            )
        }
    };

    let output = PolicyOutput {
        mode,
        reasons: reasons.clone(),
        summary,
        cpu: ModuleDirective {
            allowed: config.cpu.enabled && duty_percent > 0 && cpu_phase,
            duty_percent,
        },
        memory: ModuleDirective {
            allowed: config.memory.enabled && duty_percent > 0 && memory_phase,
            duty_percent,
        },
        network: ModuleDirective {
            allowed: config.network.enabled && duty_percent > 0 && network_phase,
            duty_percent,
        },
        budget,
    };

    state.mode = mode;
    state.reasons = reasons;
    state.budget = budget;

    output
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::config::{BurstWindow, Config};
    use crate::domain::schedule::BurstPhase;
    use crate::domain::signal::{
        PressureClass, PressureReading, PressureScope, SampleSnapshot, Session, SessionSet, Signal,
    };
    use crate::domain::state::{Mode, PolicyState, ReasonCode};

    use super::reduce;

    fn host_sample(load: u8, mem: u8) -> SampleSnapshot {
        SampleSnapshot {
            load_percent: load,
            mem_available_percent: mem,
            mem_available_bytes: 8 * 1024 * 1024 * 1024,
            mem_total_bytes: 16 * 1024 * 1024 * 1024,
            swap_used_bytes: 0,
            net_rx_bytes_per_sec: 0,
            net_tx_bytes_per_sec: 0,
        }
    }

    fn one_session() -> Signal {
        Signal::Sessions(SessionSet::new(vec![Session {
            tty: "pts/0".to_string(),
            remote: true,
        }]))
    }

    fn legacy_decision(sample: &SampleSnapshot, config: &Config, interactive: bool) -> (Mode, u8) {
        let mode = if interactive
            || sample.load_percent >= config.global.pause_load_percent
            || sample.mem_available_percent <= config.global.pause_mem_available_percent
        {
            Mode::Suspended
        } else if sample.load_percent >= config.global.caution_load_percent
            || sample.mem_available_percent <= config.global.caution_mem_available_percent
        {
            Mode::Cooling
        } else {
            Mode::Running
        };
        let duty = match mode {
            Mode::Running => 100,
            Mode::Cooling => config.global.caution_duty_percent,
            Mode::Suspended => 0,
        };
        (mode, duty)
    }

    #[test]
    fn pauses_on_interactive_sessions() {
        let config = Config::default();
        let mut state = PolicyState::new(1);
        let now = Instant::now();

        let output = reduce(&mut state, &one_session(), &config, now);
        assert_eq!(output.mode, Mode::Suspended);
        assert!(!output.cpu.allowed);
        assert!(
            output
                .reasons
                .contains(&crate::domain::state::ReasonCode::InteractiveSession)
        );
        assert_eq!(state.sessions.tty_count(), 1);

        let output = reduce(
            &mut state,
            &Signal::Sample(host_sample(10, 70)),
            &config,
            now,
        );
        assert_eq!(output.mode, Mode::Suspended);
        assert!(!output.memory.allowed);
    }

    #[test]
    fn resumes_after_interactive_grace() {
        let mut config = Config::default();
        config.global.resume_ramp_secs = 0;
        let mut state = PolicyState::new(1);
        let start = Instant::now();

        reduce(&mut state, &one_session(), &config, start);
        reduce(
            &mut state,
            &Signal::Sessions(SessionSet::default()),
            &config,
            start + Duration::from_secs(1),
        );
        let output = reduce(
            &mut state,
            &Signal::Sample(host_sample(10, 70)),
            &config,
            start + Duration::from_secs(config.global.interactive_grace_secs + 1),
        );

        assert_eq!(output.mode, Mode::Running);
        assert_eq!(output.cpu.duty_percent, 100);
    }

    #[test]
    fn matches_legacy_decision_table() {
        let config = Config::default();
        let start = Instant::now();
        let cases = [
            (host_sample(0, 80), false),
            (host_sample(54, 80), false),
            (host_sample(55, 80), false),
            (host_sample(60, 80), false),
            (host_sample(81, 80), false),
            (host_sample(82, 80), false),
            (host_sample(90, 80), false),
            (host_sample(100, 80), false),
            (host_sample(0, 21), false),
            (host_sample(0, 20), false),
            (host_sample(0, 19), false),
            (host_sample(0, 11), false),
            (host_sample(0, 10), false),
            (host_sample(0, 9), false),
            (host_sample(0, 0), false),
            (host_sample(81, 21), false),
            (host_sample(82, 21), false),
            (host_sample(81, 20), false),
            (host_sample(10, 70), true),
            (host_sample(81, 20), true),
        ];

        for (sample, interactive) in cases {
            let expected = legacy_decision(&sample, &config, interactive);
            let mut state = PolicyState::new(7);
            if interactive {
                reduce(&mut state, &one_session(), &config, start);
            }
            let output = reduce(&mut state, &Signal::Sample(sample), &config, start);

            assert_eq!(output.mode, expected.0, "mode mismatch for {:?}", expected);
            assert_eq!(output.cpu.duty_percent, expected.1);
            assert_eq!(output.memory.duty_percent, expected.1);
            assert_eq!(output.network.duty_percent, expected.1);

            // A fresh scheduler flips Rest -> Burst on its first evaluation, so
            // the legacy and new `allowed` values agree for this table.
            let active = expected.1 > 0;
            assert_eq!(output.cpu.allowed, config.cpu.enabled && active);
            assert_eq!(output.memory.allowed, config.memory.enabled && active);
            assert_eq!(output.network.allowed, config.network.enabled && active);
        }
    }

    #[test]
    fn advances_schedule_while_suspended() {
        let mut config = Config::default();
        config.memory.window = BurstWindow {
            burst_secs_min: 10,
            burst_secs_max: 10,
            rest_secs_min: 20,
            rest_secs_max: 20,
        };
        let mut state = PolicyState::new(1);
        let start = Instant::now();

        reduce(&mut state, &one_session(), &config, start);
        assert_eq!(state.memory_schedule.phase(), BurstPhase::Burst);

        reduce(
            &mut state,
            &one_session(),
            &config,
            start + Duration::from_secs(11),
        );
        assert_eq!(state.memory_schedule.phase(), BurstPhase::Rest);
    }

    #[test]
    fn parity_grace_expiry_matches_legacy() {
        let mut config = Config::default();
        config.global.resume_ramp_secs = 0;
        let mut state = PolicyState::new(3);
        let start = Instant::now();

        reduce(&mut state, &one_session(), &config, start);
        reduce(
            &mut state,
            &Signal::Sessions(SessionSet::default()),
            &config,
            start + Duration::from_secs(1),
        );

        let sample = host_sample(0, 80);
        let output = reduce(
            &mut state,
            &Signal::Sample(sample.clone()),
            &config,
            start + Duration::from_secs(config.global.interactive_grace_secs),
        );
        let expected = legacy_decision(&sample, &config, false);

        assert_eq!(output.mode, expected.0);
        assert_eq!(output.cpu.duty_percent, expected.1);
        assert_eq!(output.memory.duty_percent, expected.1);
        assert_eq!(output.network.duty_percent, expected.1);
    }

    #[test]
    fn allowed_respects_rest_phase() {
        let mut config = Config::default();
        config.memory.window = BurstWindow {
            burst_secs_min: 10,
            burst_secs_max: 10,
            rest_secs_min: 20,
            rest_secs_max: 20,
        };
        let mut state = PolicyState::new(1);
        let start = Instant::now();

        let output = reduce(
            &mut state,
            &Signal::Sample(host_sample(0, 80)),
            &config,
            start,
        );
        assert_eq!(output.mode, Mode::Running);
        assert!(output.memory.allowed);

        let output = reduce(
            &mut state,
            &Signal::Sample(host_sample(0, 80)),
            &config,
            start + Duration::from_secs(11),
        );
        assert_eq!(output.mode, Mode::Running);
        assert_eq!(output.memory.duty_percent, 100);
        assert!(!output.memory.allowed);
        assert_eq!(state.memory_schedule.phase(), BurstPhase::Rest);
    }

    #[test]
    fn primary_reason_orders_reasons_by_severity() {
        use crate::domain::state::ReasonCode;

        let config = Config::default();
        let mut state = PolicyState::new(1);
        let start = Instant::now();

        let output = reduce(
            &mut state,
            &Signal::Sample(host_sample(90, 5)),
            &config,
            start,
        );
        assert!(output.reasons.contains(&ReasonCode::LoadPressure));
        assert!(output.reasons.contains(&ReasonCode::MemoryPressure));
        assert_eq!(output.primary_reason(), Some(ReasonCode::MemoryPressure));

        let output = reduce(&mut state, &one_session(), &config, start);
        assert_eq!(
            output.primary_reason(),
            Some(ReasonCode::InteractiveSession)
        );
        assert!(ReasonCode::InteractiveSession > ReasonCode::LoadPressure);

        let mut fresh = PolicyState::new(2);
        let output = reduce(
            &mut fresh,
            &Signal::Sample(host_sample(0, 80)),
            &config,
            start,
        );
        assert_eq!(output.primary_reason(), None);
    }

    #[test]
    fn pressure_suspends_for_quiet_window_and_ramps_back() {
        let mut config = Config::default();
        config.global.resume_quiet_secs = 10;
        config.global.resume_ramp_secs = 90;
        let mut state = PolicyState::new(1);
        let start = Instant::now();

        let pressure = Signal::Pressure {
            class: PressureClass::Memory,
            scope: PressureScope::Host,
            reading: PressureReading {
                some_avg10_x100: 210,
                ..PressureReading::default()
            },
        };
        let output = reduce(&mut state, &pressure, &config, start);
        assert_eq!(output.mode, Mode::Suspended);
        assert!(output.reasons.contains(&ReasonCode::MemoryPressure));

        let output = reduce(
            &mut state,
            &Signal::Sample(host_sample(0, 80)),
            &config,
            start + Duration::from_secs(9),
        );
        assert_eq!(output.mode, Mode::Suspended);

        let resume_at = start + Duration::from_secs(11);
        let output = reduce(
            &mut state,
            &Signal::Sample(host_sample(0, 80)),
            &config,
            resume_at,
        );
        assert_eq!(output.mode, Mode::Running);
        assert_eq!(output.cpu.duty_percent, 25);

        let output = reduce(
            &mut state,
            &Signal::Sample(host_sample(0, 80)),
            &config,
            resume_at + Duration::from_secs(31),
        );
        assert_eq!(output.cpu.duty_percent, 50);

        let output = reduce(
            &mut state,
            &Signal::Sample(host_sample(0, 80)),
            &config,
            resume_at + Duration::from_secs(61),
        );
        assert_eq!(output.cpu.duty_percent, 100);

        let output = reduce(
            &mut state,
            &Signal::Sample(host_sample(0, 80)),
            &config,
            resume_at + Duration::from_secs(91),
        );
        assert_eq!(output.cpu.duty_percent, 100);
        assert!(state.resumed_at.is_none());
    }

    #[test]
    fn pressure_classes_map_to_reasons() {
        let config = Config::default();
        let start = Instant::now();
        for (class, reason) in [
            (PressureClass::Cpu, ReasonCode::LoadPressure),
            (PressureClass::Io, ReasonCode::IoPressure),
            (PressureClass::Memory, ReasonCode::MemoryPressure),
        ] {
            let mut state = PolicyState::new(1);
            let signal = Signal::Pressure {
                class,
                scope: PressureScope::Host,
                reading: PressureReading::default(),
            };
            let output = reduce(&mut state, &signal, &config, start);
            assert_eq!(output.mode, Mode::Suspended);
            assert_eq!(output.reasons, vec![reason]);
        }
    }

    #[test]
    fn zero_quiet_window_still_suspends_on_a_fresh_pressure_signal() {
        let mut config = Config::default();
        config.global.resume_quiet_secs = 0;
        let mut state = PolicyState::new(1);

        let signal = Signal::Pressure {
            class: PressureClass::Memory,
            scope: PressureScope::Host,
            reading: PressureReading::default(),
        };
        let output = reduce(&mut state, &signal, &config, Instant::now());

        assert_eq!(output.mode, Mode::Suspended);
        assert!(output.reasons.contains(&ReasonCode::MemoryPressure));
    }

    #[test]
    fn budget_respects_reserve_and_headroom() {
        let mut config = Config::default();
        config.global.reserve_mb = 512;
        config.global.reserve_percent = 10;
        config.memory.max_absolute_mb = 0;

        let mut state = PolicyState::new(1);
        let mut sample = host_sample(0, 80);
        sample.mem_total_bytes = 1024 * 1024 * 1024;
        sample.mem_available_bytes = 600 * 1024 * 1024;

        let output = reduce(&mut state, &Signal::Sample(sample), &config, Instant::now());

        assert_eq!(output.budget.reserve_bytes, 512 * 1024 * 1024);
        assert_eq!(output.budget.target_bytes, 88 * 1024 * 1024);

        let mut capped = Config::default();
        capped.memory.max_absolute_mb = 32;
        let mut capped_state = PolicyState::new(2);
        let mut capped_sample = host_sample(0, 80);
        capped_sample.mem_total_bytes = 1024 * 1024 * 1024;
        capped_sample.mem_available_bytes = 600 * 1024 * 1024;

        let output = reduce(
            &mut capped_state,
            &Signal::Sample(capped_sample),
            &capped,
            Instant::now(),
        );
        assert_eq!(output.budget.target_bytes, 32 * 1024 * 1024);
    }

    #[test]
    fn cgroup_pressure_scope_appears_in_summary() {
        let config = Config::default();
        let mut state = PolicyState::new(1);
        let signal = Signal::Pressure {
            class: PressureClass::Memory,
            scope: PressureScope::Cgroup("kubepods.slice".to_string()),
            reading: PressureReading::default(),
        };

        let output = reduce(&mut state, &signal, &config, Instant::now());

        assert_eq!(output.mode, Mode::Suspended);
        assert!(output.summary.contains("scope=kubepods.slice"));
    }

    #[test]
    fn degrades_and_recovers() {
        let config = Config::default();
        let mut state = PolicyState::new(1);
        let now = Instant::now();

        let output = reduce(
            &mut state,
            &Signal::SourceDegraded {
                source: "procfs".to_string(),
                message: "boom".to_string(),
            },
            &config,
            now,
        );
        assert_eq!(output.mode, Mode::Suspended);
        assert!(output.summary.contains("sensor backend failed"));

        let output = reduce(
            &mut state,
            &Signal::Sample(host_sample(5, 80)),
            &config,
            now,
        );
        assert_eq!(output.mode, Mode::Running);
    }
}
