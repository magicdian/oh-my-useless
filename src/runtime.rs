use std::sync::{Arc, RwLock};
use std::time::SystemTime;

use crate::config::Config;
use crate::policy::PolicySnapshot;
use crate::sensor::SensorSample;

pub type SharedRuntime = Arc<RwLock<RuntimeState>>;

#[derive(Debug, Clone)]
pub struct RuntimeState {
    pub config: Config,
    pub policy: PolicySnapshot,
    pub sensor_sample: SensorSample,
    pub last_updated: SystemTime,
    pub shutdown: bool,
}

impl RuntimeState {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            policy: PolicySnapshot::paused("waiting for the first supervisor sample"),
            sensor_sample: SensorSample::default(),
            last_updated: SystemTime::now(),
            shutdown: false,
        }
    }

    pub fn shared(config: Config) -> SharedRuntime {
        Arc::new(RwLock::new(Self::new(config)))
    }
}
