use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::config::{BurstWindow, Config};
use crate::sensor::SensorSample;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressureLevel {
    Idle,
    Caution,
    Pause,
}

#[derive(Debug, Clone)]
pub struct ModuleDirective {
    pub allowed: bool,
    pub duty_percent: u8,
}

#[derive(Debug, Clone)]
pub struct PolicySnapshot {
    #[allow(dead_code)]
    pub pressure: PressureLevel,
    #[allow(dead_code)]
    pub interactive_active: bool,
    pub summary: String,
    pub cpu: ModuleDirective,
    pub memory: ModuleDirective,
    pub network: ModuleDirective,
}

impl PolicySnapshot {
    pub fn paused(reason: impl Into<String>) -> Self {
        let paused = ModuleDirective {
            allowed: false,
            duty_percent: 0,
        };
        Self {
            pressure: PressureLevel::Pause,
            interactive_active: false,
            summary: reason.into(),
            cpu: paused.clone(),
            memory: paused.clone(),
            network: paused,
        }
    }
}

#[derive(Debug)]
pub struct Supervisor {
    cpu_schedule: BurstScheduler,
    memory_schedule: BurstScheduler,
    network_schedule: BurstScheduler,
    last_interactive: Option<Instant>,
}

impl Supervisor {
    pub fn new() -> Self {
        let seed = seed_now();
        Self {
            cpu_schedule: BurstScheduler::new(seed ^ 0x1111),
            memory_schedule: BurstScheduler::new(seed ^ 0x2222),
            network_schedule: BurstScheduler::new(seed ^ 0x3333),
            last_interactive: None,
        }
    }

    pub fn evaluate(
        &mut self,
        sample: &SensorSample,
        config: &Config,
        now: Instant,
    ) -> PolicySnapshot {
        if sample.interactive_session {
            self.last_interactive = Some(now);
        }

        let interactive_active = self
            .last_interactive
            .map(|last| {
                now.duration_since(last) < Duration::from_secs(config.global.interactive_grace_secs)
            })
            .unwrap_or(false);

        let pressure = if interactive_active
            || sample.load_percent >= config.global.pause_load_percent
            || sample.mem_available_percent <= config.global.pause_mem_available_percent
        {
            PressureLevel::Pause
        } else if sample.load_percent >= config.global.caution_load_percent
            || sample.mem_available_percent <= config.global.caution_mem_available_percent
        {
            PressureLevel::Caution
        } else {
            PressureLevel::Idle
        };

        let duty_percent = match pressure {
            PressureLevel::Idle => 100,
            PressureLevel::Caution => config.global.caution_duty_percent,
            PressureLevel::Pause => 0,
        };

        let cpu = ModuleDirective {
            allowed: config.cpu.enabled
                && duty_percent > 0
                && self.cpu_schedule.is_active(now, &config.cpu.window),
            duty_percent,
        };
        let memory = ModuleDirective {
            allowed: config.memory.enabled
                && duty_percent > 0
                && self.memory_schedule.is_active(now, &config.memory.window),
            duty_percent,
        };
        let network = ModuleDirective {
            allowed: config.network.enabled
                && duty_percent > 0
                && self.network_schedule.is_active(now, &config.network.window),
            duty_percent,
        };

        let summary = if interactive_active {
            format!(
                "interactive session detected (tty={}, ssh={})",
                sample.tty_sessions, sample.ssh_sessions
            )
        } else {
            format!(
                "pressure={pressure:?} load={} mem_avail={} net_rx={}B/s net_tx={}B/s",
                sample.load_percent,
                sample.mem_available_percent,
                sample.net_rx_bytes_per_sec,
                sample.net_tx_bytes_per_sec
            )
        };

        PolicySnapshot {
            pressure,
            interactive_active,
            summary,
            cpu,
            memory,
            network,
        }
    }
}

#[derive(Debug)]
struct BurstScheduler {
    rng: SimpleRng,
    phase: BurstPhase,
    until: Instant,
}

#[derive(Debug, Clone, Copy)]
enum BurstPhase {
    Burst,
    Rest,
}

impl BurstScheduler {
    fn new(seed: u64) -> Self {
        Self {
            rng: SimpleRng::new(seed),
            phase: BurstPhase::Rest,
            until: Instant::now(),
        }
    }

    fn is_active(&mut self, now: Instant, window: &BurstWindow) -> bool {
        while now >= self.until {
            self.phase = match self.phase {
                BurstPhase::Burst => BurstPhase::Rest,
                BurstPhase::Rest => BurstPhase::Burst,
            };
            let seconds = match self.phase {
                BurstPhase::Burst => self
                    .rng
                    .between(window.burst_secs_min, window.burst_secs_max),
                BurstPhase::Rest => self.rng.between(window.rest_secs_min, window.rest_secs_max),
            };
            self.until = now + Duration::from_secs(seconds.max(1));
        }

        matches!(self.phase, BurstPhase::Burst)
    }
}

#[derive(Debug)]
struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self { state: seed.max(1) }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn between(&mut self, min: u64, max: u64) -> u64 {
        if min >= max {
            return min;
        }
        min + (self.next_u64() % (max - min + 1))
    }
}

fn seed_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use crate::config::Config;
    use crate::sensor::SensorSample;

    use super::{PressureLevel, Supervisor};

    #[test]
    fn pauses_on_interactive_sessions() {
        let config = Config::default();
        let mut supervisor = Supervisor::new();
        let sample = SensorSample {
            load_percent: 10,
            mem_available_percent: 70,
            interactive_session: true,
            tty_sessions: 1,
            ssh_sessions: 1,
            net_rx_bytes_per_sec: 0,
            net_tx_bytes_per_sec: 0,
        };

        let snapshot = supervisor.evaluate(&sample, &config, Instant::now());
        assert_eq!(snapshot.pressure, PressureLevel::Pause);
        assert!(!snapshot.cpu.allowed);
        assert!(snapshot.interactive_active);
    }
}
