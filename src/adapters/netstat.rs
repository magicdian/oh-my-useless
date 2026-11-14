use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NetTotals {
    pub rx: u64,
    pub tx: u64,
}

pub trait NetOps {
    fn totals(&self, root: &Path) -> std::io::Result<NetTotals>;
}

pub struct RealNetOps;

impl NetOps for RealNetOps {
    fn totals(&self, root: &Path) -> std::io::Result<NetTotals> {
        read_totals(root)
    }
}

/// Sum rx/tx counters for interfaces that are up, excluding loopback.
pub fn read_totals(root: &Path) -> std::io::Result<NetTotals> {
    let mut totals = NetTotals::default();

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == "lo" {
            continue;
        }

        let path = entry.path();
        if !fs::read_to_string(path.join("operstate"))
            .map(|value| value.trim() == "up")
            .unwrap_or(false)
        {
            continue;
        }

        totals.rx += read_u64(path.join("statistics/rx_bytes")).unwrap_or(0);
        totals.tx += read_u64(path.join("statistics/tx_bytes")).unwrap_or(0);
    }

    Ok(totals)
}

fn read_u64(path: impl AsRef<Path>) -> std::io::Result<u64> {
    let value = fs::read_to_string(path)?;
    value.trim().parse::<u64>().map_err(std::io::Error::other)
}

/// The daemon's own network bytes, recorded as it downloads.
#[derive(Debug, Default)]
pub struct TrafficLedger {
    rx: AtomicU64,
    tx: AtomicU64,
}

impl TrafficLedger {
    pub fn add_rx(&self, bytes: u64) {
        self.rx.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn add_tx(&self, bytes: u64) {
        self.tx.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> (u64, u64) {
        (
            self.rx.load(Ordering::Relaxed),
            self.tx.load(Ordering::Relaxed),
        )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExternalTraffic {
    pub ingress_kbps: u64,
    pub egress_kbps: u64,
}

pub fn estimate(
    ops: &dyn NetOps,
    ledger: &TrafficLedger,
    root: &Path,
    window: Duration,
    sleep: impl Fn(Duration),
) -> std::io::Result<ExternalTraffic> {
    let before = ops.totals(root)?;
    let (own_rx_before, own_tx_before) = ledger.snapshot();
    sleep(window);
    let after = ops.totals(root)?;
    let (own_rx_after, own_tx_after) = ledger.snapshot();

    let total_rx = after.rx.saturating_sub(before.rx);
    let total_tx = after.tx.saturating_sub(before.tx);
    let own_rx = own_rx_after.saturating_sub(own_rx_before);
    let own_tx = own_tx_after.saturating_sub(own_tx_before);
    let external_rx = total_rx.saturating_sub(own_rx);
    let external_tx = total_tx.saturating_sub(own_tx);
    let millis = window.as_millis().max(1) as u64;

    Ok(ExternalTraffic {
        ingress_kbps: external_rx.saturating_mul(8) / millis,
        egress_kbps: external_tx.saturating_mul(8) / millis,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::Duration;

    use super::{ExternalTraffic, NetOps, NetTotals, TrafficLedger, estimate, read_totals};
    use crate::domain::should_start_burst;

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("useless-netstat-{}-{name}", std::process::id()))
    }

    fn write_interface(root: &Path, name: &str, up: bool, rx: u64, tx: u64) {
        let path = root.join(name);
        std::fs::create_dir_all(path.join("statistics")).expect("interface dir");
        std::fs::write(path.join("operstate"), if up { "up\n" } else { "down\n" })
            .expect("operstate");
        std::fs::write(path.join("statistics/rx_bytes"), format!("{rx}\n")).expect("rx");
        std::fs::write(path.join("statistics/tx_bytes"), format!("{tx}\n")).expect("tx");
    }

    #[test]
    fn reads_only_up_non_loopback_interfaces() {
        let root = temp_root("read");
        let _ = std::fs::remove_dir_all(&root);
        write_interface(&root, "eth0", true, 1000, 2000);
        write_interface(&root, "eth1", false, 5000, 5000);
        write_interface(&root, "lo", true, 9000, 9000);

        let totals = read_totals(&root).expect("totals");

        assert_eq!(totals, NetTotals { rx: 1000, tx: 2000 });
        let _ = std::fs::remove_dir_all(&root);
    }

    struct FakeNetOps {
        totals: Mutex<VecDeque<NetTotals>>,
    }

    impl NetOps for FakeNetOps {
        fn totals(&self, _root: &Path) -> std::io::Result<NetTotals> {
            Ok(self
                .totals
                .lock()
                .expect("totals")
                .pop_front()
                .unwrap_or_default())
        }
    }

    fn fake_ops(values: [NetTotals; 2]) -> FakeNetOps {
        FakeNetOps {
            totals: Mutex::new(VecDeque::from(values)),
        }
    }

    #[test]
    fn subtracts_own_traffic_from_the_estimate() {
        let ledger = TrafficLedger::default();
        let ops = fake_ops([
            NetTotals { rx: 0, tx: 0 },
            NetTotals {
                rx: 100_000,
                tx: 50_000,
            },
        ]);

        let estimate = estimate(
            &ops,
            &ledger,
            Path::new("/sys/class/net"),
            Duration::from_secs(1),
            |_| ledger.add_rx(400),
        )
        .expect("estimate");

        assert_eq!(
            estimate,
            ExternalTraffic {
                ingress_kbps: 796,
                egress_kbps: 400
            }
        );
    }

    #[test]
    fn own_equals_total_yields_no_external_traffic() {
        let ledger = TrafficLedger::default();
        let ops = fake_ops([NetTotals { rx: 0, tx: 0 }, NetTotals { rx: 100, tx: 0 }]);

        let estimate = estimate(
            &ops,
            &ledger,
            Path::new("/sys/class/net"),
            Duration::from_secs(1),
            |_| ledger.add_rx(100),
        )
        .expect("estimate");

        assert_eq!(estimate, ExternalTraffic::default());
        assert!(should_start_burst(
            estimate.ingress_kbps,
            estimate.egress_kbps,
            1024
        ));
    }

    #[test]
    fn counter_wrap_clamps_to_zero() {
        let ledger = TrafficLedger::default();
        let ops = fake_ops([
            NetTotals { rx: 1000, tx: 1000 },
            NetTotals { rx: 500, tx: 500 },
        ]);

        let estimate = estimate(
            &ops,
            &ledger,
            Path::new("/sys/class/net"),
            Duration::from_secs(1),
            |_| {},
        )
        .expect("estimate");

        assert_eq!(estimate, ExternalTraffic::default());
    }

    #[test]
    fn no_external_traffic_yields_zero() {
        let ledger = TrafficLedger::default();
        let ops = fake_ops([NetTotals { rx: 0, tx: 0 }, NetTotals { rx: 0, tx: 0 }]);

        let estimate = estimate(
            &ops,
            &ledger,
            Path::new("/sys/class/net"),
            Duration::from_secs(1),
            |_| {},
        )
        .expect("estimate");

        assert_eq!(estimate, ExternalTraffic::default());
    }

    #[test]
    fn ingress_only_leaves_egress_at_zero() {
        let ledger = TrafficLedger::default();
        let ops = fake_ops([NetTotals { rx: 0, tx: 0 }, NetTotals { rx: 1000, tx: 0 }]);

        let estimate = estimate(
            &ops,
            &ledger,
            Path::new("/sys/class/net"),
            Duration::from_secs(1),
            |_| {},
        )
        .expect("estimate");

        assert_eq!(
            estimate,
            ExternalTraffic {
                ingress_kbps: 8,
                egress_kbps: 0
            }
        );
    }

    #[test]
    fn egress_only_leaves_ingress_at_zero() {
        let ledger = TrafficLedger::default();
        let ops = fake_ops([NetTotals { rx: 0, tx: 0 }, NetTotals { rx: 0, tx: 2000 }]);

        let estimate = estimate(
            &ops,
            &ledger,
            Path::new("/sys/class/net"),
            Duration::from_secs(1),
            |_| {},
        )
        .expect("estimate");

        assert_eq!(
            estimate,
            ExternalTraffic {
                ingress_kbps: 0,
                egress_kbps: 16
            }
        );
    }

    struct FailingNetOps;

    impl NetOps for FailingNetOps {
        fn totals(&self, _root: &Path) -> std::io::Result<NetTotals> {
            Err(std::io::Error::other("no counters"))
        }
    }

    #[test]
    fn estimator_reports_counter_errors() {
        let estimate = estimate(
            &FailingNetOps,
            &TrafficLedger::default(),
            Path::new("/sys/class/net"),
            Duration::from_secs(1),
            |_| {},
        );

        assert!(estimate.is_err());
    }
}
