use std::time::{Duration, Instant};

use crate::config::Config;
use crate::domain::budget::{BudgetInput, compute};
use crate::domain::signal::Signal;
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

    let sample = state.last_sample.clone().unwrap_or_default();
    let mut reasons = Vec::new();

    let mode = if state.degraded {
        reasons.push(ReasonCode::CollectorDegraded);
        Mode::Suspended
    } else if interactive_active {
        reasons.push(ReasonCode::InteractiveSession);
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

    let duty_percent = match mode {
        Mode::Running => 100,
        Mode::Cooling => config.global.caution_duty_percent,
        Mode::Suspended => 0,
    };

    // Advance every schedule on every evaluation, including while suspended.
    // Freezing the phase caused an immediate full-size burst on resume.
    let cpu_phase = state.cpu_schedule.is_active(now, &config.cpu.window);
    let memory_phase = state.memory_schedule.is_active(now, &config.memory.window);
    let network_phase = state
        .network_schedule
        .is_active(now, &config.network.window);

    // Informational in this child: child 3's chunked ledger consumes this
    // budget. The memory worker still sizes from the legacy ratio until then.
    let budget = compute(&BudgetInput {
        mem_total: sample.mem_total_bytes,
        mem_available: sample.mem_available_bytes,
        max_percent: config.memory.max_percent,
        duty_percent,
        max_absolute_mb: 0,
        reserve_mb: 0,
        reserve_percent: 0,
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
        _ => format!(
            "pressure={} load={} mem_avail={} net_rx={}B/s net_tx={}B/s",
            mode.label(),
            sample.load_percent,
            sample.mem_available_percent,
            sample.net_rx_bytes_per_sec,
            sample.net_tx_bytes_per_sec
        ),
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
    use crate::domain::signal::{SampleSnapshot, Session, SessionSet, Signal};
    use crate::domain::state::{Mode, PolicyState};

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
        let config = Config::default();
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
        let config = Config::default();
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
