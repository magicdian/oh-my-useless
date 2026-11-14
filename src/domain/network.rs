/// A burst may start only when neither direction exceeds the configured
/// external-traffic threshold. `0` disables the gate.
pub fn should_start_burst(ingress_kbps: u64, egress_kbps: u64, threshold_kbps: u32) -> bool {
    threshold_kbps == 0
        || (ingress_kbps <= threshold_kbps as u64 && egress_kbps <= threshold_kbps as u64)
}

#[cfg(test)]
mod tests {
    use super::should_start_burst;

    #[test]
    fn zero_threshold_disables_the_gate() {
        assert!(should_start_burst(999_999, 999_999, 0));
    }

    #[test]
    fn allows_traffic_below_the_threshold() {
        assert!(should_start_burst(100, 200, 1024));
    }

    #[test]
    fn blocks_when_either_direction_exceeds_the_threshold() {
        assert!(!should_start_burst(2048, 0, 1024));
        assert!(!should_start_burst(0, 2048, 1024));
    }
}
