use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime};

use crate::config::Config;
use crate::domain::state::PolicyOutput;
use crate::ports::StatePublisher;

#[derive(Clone)]
pub struct StoreState {
    pub config: Arc<Config>,
    pub directives: PolicyOutput,
    pub generation: u64,
    pub shutdown: bool,
    pub last_updated: SystemTime,
}

/// Shared directives with a generation counter. Workers wait for a change
/// instead of sampling the lock on a fixed cadence.
pub struct Store {
    inner: Mutex<StoreState>,
    changed: Condvar,
}

impl Store {
    pub fn new(config: Arc<Config>, directives: PolicyOutput) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(StoreState {
                config,
                directives,
                generation: 1,
                shutdown: false,
                last_updated: SystemTime::now(),
            }),
            changed: Condvar::new(),
        })
    }

    pub fn publish(&self, output: PolicyOutput) -> u64 {
        let mut state = self.inner.lock().expect("store write lock");
        state.directives = output;
        state.generation = state.generation.wrapping_add(1);
        state.last_updated = SystemTime::now();
        let generation = state.generation;
        drop(state);
        self.changed.notify_all();
        generation
    }

    pub fn replace_config(&self, config: Arc<Config>) -> u64 {
        let mut state = self.inner.lock().expect("store write lock");
        state.config = config;
        state.generation = state.generation.wrapping_add(1);
        state.last_updated = SystemTime::now();
        let generation = state.generation;
        drop(state);
        self.changed.notify_all();
        generation
    }

    pub fn shutdown(&self) {
        let mut state = self.inner.lock().expect("store write lock");
        state.shutdown = true;
        state.generation = state.generation.wrapping_add(1);
        state.last_updated = SystemTime::now();
        drop(state);
        self.changed.notify_all();
    }

    pub fn is_shutdown(&self) -> bool {
        self.inner.lock().expect("store read lock").shutdown
    }

    pub fn network_allowed(&self) -> bool {
        self.inner
            .lock()
            .expect("store read lock")
            .directives
            .network
            .allowed
    }

    pub fn snapshot(&self) -> StoreState {
        self.inner.lock().expect("store read lock").clone()
    }

    /// Blocks until the generation moves past `seen`, the store shuts down, or
    /// `timeout` elapses. The timeout is a safety net, not the primary path.
    pub fn wait_for_change(&self, seen: u64, timeout: Duration) -> StoreState {
        let mut state = self.inner.lock().expect("store read lock");
        while state.generation == seen && !state.shutdown {
            let (guard, _) = self
                .changed
                .wait_timeout(state, timeout)
                .expect("store wait lock");
            state = guard;
        }
        state.clone()
    }
}

impl StatePublisher for Store {
    fn publish(&self, output: PolicyOutput) -> u64 {
        Store::publish(self, output)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use crate::config::Config;
    use crate::domain::state::{PolicyOutput, ReasonCode};

    use super::Store;

    fn paused() -> PolicyOutput {
        PolicyOutput::suspended(ReasonCode::CollectorDegraded, "test")
    }

    #[test]
    fn publish_bumps_generation_and_wakes_waiters() {
        let store = Store::new(Arc::new(Config::default()), paused());
        let seen = store.snapshot().generation;

        let publisher = Arc::clone(&store);
        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            publisher.publish(paused());
        });

        let state = store.wait_for_change(seen, Duration::from_secs(2));
        assert!(state.generation > seen);
        handle.join().expect("publisher joins");
    }

    #[test]
    fn shutdown_wakes_waiters() {
        let store = Store::new(Arc::new(Config::default()), paused());
        let seen = store.snapshot().generation;

        let stopper = Arc::clone(&store);
        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            stopper.shutdown();
        });

        let state = store.wait_for_change(seen, Duration::from_secs(2));
        assert!(state.shutdown);
        assert!(store.is_shutdown());
        handle.join().expect("stopper joins");
    }
}
