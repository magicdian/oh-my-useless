pub mod cpu;
pub mod memory;
pub mod network;

use std::thread;

use crate::runtime::SharedRuntime;

pub fn spawn_all(shared: SharedRuntime) -> Vec<thread::JoinHandle<()>> {
    vec![
        thread::Builder::new()
            .name("useless-cpu".to_string())
            .spawn({
                let shared = shared.clone();
                move || cpu::run(shared)
            })
            .expect("spawn cpu worker"),
        thread::Builder::new()
            .name("useless-memory".to_string())
            .spawn({
                let shared = shared.clone();
                move || memory::run(shared)
            })
            .expect("spawn memory worker"),
        thread::Builder::new()
            .name("useless-network".to_string())
            .spawn(move || network::run(shared))
            .expect("spawn network worker"),
    ]
}
