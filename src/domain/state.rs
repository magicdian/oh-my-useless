use std::time::{Duration, Instant};

use crate::domain::budget::Budget;
use crate::domain::schedule::BurstScheduler;
use crate::domain::signal::{SampleSnapshot, SessionSet};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Running,
    Cooling,
    Suspended,
}

impl Mode {
    /// Keeps the operator-visible log vocabulary from the pre-refactor policy.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Running => "Idle",
            Mode::Cooling => "Caution",
            Mode::Suspended => "Pause",
        }
    }

    /// Stable lowercase key for the state file and journal.
    pub fn key(self) -> &'static str {
        match self {
            Mode::Running => "running",
            Mode::Cooling => "cooling",
            Mode::Suspended => "suspended",
        }
    }
}

/// Ordered by severity; `max()` yields the most important active reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReasonCode {
    CollectorDegraded,
    LoadPressure,
    IoPressure,
    MemoryPressure,
    InteractiveSession,
}

impl ReasonCode {
    pub fn label(self) -> &'static str {
        match self {
            ReasonCode::CollectorDegraded => "collector_degraded",
            ReasonCode::LoadPressure => "load_pressure",
            ReasonCode::IoPressure => "io_pressure",
            ReasonCode::MemoryPressure => "memory_pressure",
            ReasonCode::InteractiveSession => "interactive_session",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModuleDirective {
    pub allowed: bool,
    pub duty_percent: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyOutput {
    pub mode: Mode,
    pub reasons: Vec<ReasonCode>,
    pub summary: String,
    pub cpu: ModuleDirective,
    pub memory: ModuleDirective,
    pub network: ModuleDirective,
    pub budget: Budget,
}

impl PolicyOutput {
    #[allow(dead_code)]
    pub fn primary_reason(&self) -> Option<ReasonCode> {
        self.reasons.iter().copied().max()
    }

    pub fn suspended(reason: ReasonCode, summary: impl Into<String>) -> Self {
        let directive = ModuleDirective {
            allowed: false,
            duty_percent: 0,
        };
        Self {
            mode: Mode::Suspended,
            reasons: vec![reason],
            summary: summary.into(),
            cpu: directive,
            memory: directive,
            network: directive,
            budget: Budget::default(),
        }
    }
}

#[derive(Debug)]
pub struct PolicyState {
    pub mode: Mode,
    pub reasons: Vec<ReasonCode>,
    pub last_interactive: Option<Instant>,
    pub last_sample: Option<SampleSnapshot>,
    pub sessions: SessionSet,
    pub degraded: bool,
    pub last_pressure: Option<Instant>,
    pub pressure_reason: Option<ReasonCode>,
    pub resumed_at: Option<Instant>,
    pub budget: Budget,
    pub cpu_schedule: BurstScheduler,
    pub memory_schedule: BurstScheduler,
    pub network_schedule: BurstScheduler,
}

impl PolicyState {
    pub fn new(seed: u64) -> Self {
        Self {
            mode: Mode::Running,
            reasons: Vec::new(),
            last_interactive: None,
            last_sample: None,
            sessions: SessionSet::default(),
            degraded: false,
            last_pressure: None,
            pressure_reason: None,
            resumed_at: None,
            budget: Budget::default(),
            cpu_schedule: BurstScheduler::new(seed ^ 0x1111),
            memory_schedule: BurstScheduler::new(seed ^ 0x2222),
            network_schedule: BurstScheduler::new(seed ^ 0x3333),
        }
    }
}

/// Fallback sampler cadence. Events always reset the next sample to the
/// `Running` value, so this only governs how often we probe when nothing is
/// pushing.
pub fn sampler_cadence(mode: Mode, base_secs: u64) -> Duration {
    let base = base_secs.max(1);
    match mode {
        Mode::Running => Duration::from_secs(base),
        Mode::Cooling => Duration::from_secs(base.saturating_mul(2)),
        Mode::Suspended => Duration::from_secs(base.saturating_mul(4).min(60)),
    }
}

#[cfg(test)]
mod tests {
    use super::{Mode, sampler_cadence};
    use std::time::Duration;

    #[test]
    fn cadence_scales_with_mode() {
        assert_eq!(sampler_cadence(Mode::Running, 5), Duration::from_secs(5));
        assert_eq!(sampler_cadence(Mode::Cooling, 5), Duration::from_secs(10));
        assert_eq!(sampler_cadence(Mode::Suspended, 5), Duration::from_secs(20));
        assert_eq!(
            sampler_cadence(Mode::Suspended, 30),
            Duration::from_secs(60)
        );
        assert_eq!(sampler_cadence(Mode::Running, 0), Duration::from_secs(1));
    }
}
