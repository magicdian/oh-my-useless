pub mod cpu;
pub mod memory;
pub mod network;

use std::sync::Arc;
use std::thread;

use crate::runtime::Store;

pub fn spawn_all(store: Arc<Store>) -> Vec<thread::JoinHandle<()>> {
    vec![
        thread::Builder::new()
            .name("useless-cpu".to_string())
            .spawn({
                let store = Arc::clone(&store);
                move || cpu::run(store)
            })
            .expect("spawn cpu worker"),
        thread::Builder::new()
            .name("useless-memory".to_string())
            .spawn({
                let store = Arc::clone(&store);
                move || memory::run(store)
            })
            .expect("spawn memory worker"),
        thread::Builder::new()
            .name("useless-network".to_string())
            .spawn(move || network::run(store))
            .expect("spawn network worker"),
    ]
}
