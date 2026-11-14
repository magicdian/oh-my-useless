use std::time::{Duration, Instant};

use crate::config::BurstWindow;

#[derive(Debug)]
pub struct BurstScheduler {
    rng: SimpleRng,
    phase: BurstPhase,
    until: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BurstPhase {
    Burst,
    Rest,
}

impl BurstScheduler {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: SimpleRng::new(seed),
            phase: BurstPhase::Rest,
            until: None,
        }
    }

    /// Advances the schedule to cover `now` and reports whether work is allowed.
    ///
    /// The caller must invoke this on every evaluation, even when duty is zero,
    /// so the phase does not freeze while the daemon is suspended.
    pub fn is_active(&mut self, now: Instant, window: &BurstWindow) -> bool {
        let mut until = self.until.unwrap_or(now);
        while now >= until {
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
            until = now + Duration::from_secs(seconds.max(1));
        }

        self.until = Some(until);
        matches!(self.phase, BurstPhase::Burst)
    }

    #[allow(dead_code)]
    pub fn phase(&self) -> BurstPhase {
        self.phase
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::config::BurstWindow;

    use super::{BurstPhase, BurstScheduler};

    fn fixed_window(burst: u64, rest: u64) -> BurstWindow {
        BurstWindow {
            burst_secs_min: burst,
            burst_secs_max: burst,
            rest_secs_min: rest,
            rest_secs_max: rest,
        }
    }

    #[test]
    fn alternates_burst_and_rest() {
        let window = fixed_window(10, 20);
        let mut schedule = BurstScheduler::new(1);
        let start = Instant::now();

        assert!(schedule.is_active(start, &window));
        assert_eq!(schedule.phase(), BurstPhase::Burst);
        assert!(schedule.is_active(start + Duration::from_secs(9), &window));
        assert!(!schedule.is_active(start + Duration::from_secs(10), &window));
        assert_eq!(schedule.phase(), BurstPhase::Rest);
        assert!(!schedule.is_active(start + Duration::from_secs(29), &window));
        assert!(schedule.is_active(start + Duration::from_secs(30), &window));
    }
}
