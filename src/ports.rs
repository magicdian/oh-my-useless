use std::time::Instant;

use crate::domain::signal::Signal;
use crate::domain::state::PolicyOutput;
use crate::error::AppResult;
use crate::runtime::journal::Transition;

/// A pull-based observation source. Implementations may read `/proc` or spawn
/// helpers, but must not make policy decisions.
pub trait Sampler {
    fn sample(&mut self, active_tty_idle_secs: u64) -> AppResult<Vec<Signal>>;
}

/// Injectable time source. The domain never reads the clock itself.
pub trait Clock {
    fn now(&self) -> Instant;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Publisher for the directives consumed by workers.
#[allow(dead_code)]
pub trait StatePublisher {
    fn publish(&self, output: PolicyOutput) -> u64;
}

/// Bounded transition log. Child 2 adds a file-backed sink.
#[allow(dead_code)]
pub trait JournalSink {
    fn record(&mut self, transition: Transition);
}

/// A push-based observation source. Child 2 implements fd registration and
/// dispatch; the trait is declared here so the domain contract is stable.
#[allow(dead_code)]
pub trait EventSource {
    fn handle(&mut self) -> AppResult<Vec<Signal>>;
}
