/// The memory budget a worker may hold. Computed in the domain, applied by the
/// worker only after a fresh headroom check.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Budget {
    pub target_bytes: u64,
    pub reserve_bytes: u64,
    pub headroom_bytes: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct BudgetInput {
    pub mem_total: u64,
    pub mem_available: u64,
    pub max_percent: u8,
    pub duty_percent: u8,
    /// `0` disables the absolute cap.
    pub max_absolute_mb: u64,
    pub reserve_mb: u64,
    pub reserve_percent: u8,
}

pub fn compute(input: &BudgetInput) -> Budget {
    let mib = 1024 * 1024;
    let reserve = input.mem_total.saturating_mul(input.reserve_percent as u64) / 100;
    let reserve = reserve.max(input.reserve_mb.saturating_mul(mib));

    let ratio_cap = input.mem_total.saturating_mul(input.max_percent as u64) / 100
        * input.duty_percent as u64
        / 100;
    let headroom = input.mem_available.saturating_sub(reserve);
    let absolute_cap = if input.max_absolute_mb == 0 {
        u64::MAX
    } else {
        input.max_absolute_mb.saturating_mul(mib)
    };

    Budget {
        target_bytes: ratio_cap.min(headroom).min(absolute_cap),
        reserve_bytes: reserve,
        headroom_bytes: headroom,
    }
}

#[cfg(test)]
mod tests {
    use super::{BudgetInput, compute};

    const MIB: u64 = 1024 * 1024;

    fn input() -> BudgetInput {
        BudgetInput {
            mem_total: 1000 * MIB,
            mem_available: 500 * MIB,
            max_percent: 20,
            duty_percent: 100,
            max_absolute_mb: 0,
            reserve_mb: 0,
            reserve_percent: 0,
        }
    }

    #[test]
    fn ratio_cap_wins_when_headroom_is_plentiful() {
        assert_eq!(compute(&input()).target_bytes, 200 * MIB);
    }

    #[test]
    fn headroom_cap_wins_when_memory_is_tight() {
        let mut value = input();
        value.mem_available = 150 * MIB;
        let budget = compute(&value);
        assert_eq!(budget.headroom_bytes, 150 * MIB);
        assert_eq!(budget.target_bytes, 150 * MIB);
    }

    #[test]
    fn absolute_cap_wins_when_smaller() {
        let mut value = input();
        value.max_absolute_mb = 16;
        assert_eq!(compute(&value).target_bytes, 16 * MIB);
    }

    #[test]
    fn reserve_percent_dominates_absolute_reserve() {
        let mut value = input();
        value.reserve_mb = 10;
        value.reserve_percent = 10;
        value.mem_available = 150 * MIB;
        let budget = compute(&value);
        assert_eq!(budget.reserve_bytes, 100 * MIB);
        assert_eq!(budget.target_bytes, 50 * MIB);
    }

    #[test]
    fn negative_headroom_clamps_to_zero() {
        let mut value = input();
        value.reserve_percent = 10;
        value.mem_available = 50 * MIB;
        assert_eq!(compute(&value).target_bytes, 0);
    }

    #[test]
    fn duty_scales_the_ratio_cap() {
        let mut value = input();
        value.duty_percent = 50;
        assert_eq!(compute(&value).target_bytes, 100 * MIB);
    }

    #[test]
    fn zero_total_is_safe() {
        let mut value = input();
        value.mem_total = 0;
        value.mem_available = 0;
        let budget = compute(&value);
        assert_eq!(budget.target_bytes, 0);
        assert_eq!(budget.headroom_bytes, 0);
    }
}
