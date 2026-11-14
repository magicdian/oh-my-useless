use std::time::Instant;

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
}

/// Ordered by severity; `max()` yields the most important active reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReasonCode {
    CollectorDegraded,
    LoadPressure,
    MemoryPressure,
    InteractiveSession,
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
            budget: Budget::default(),
            cpu_schedule: BurstScheduler::new(seed ^ 0x1111),
            memory_schedule: BurstScheduler::new(seed ^ 0x2222),
            network_schedule: BurstScheduler::new(seed ^ 0x3333),
        }
    }
}
