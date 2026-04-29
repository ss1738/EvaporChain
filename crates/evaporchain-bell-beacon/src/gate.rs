//! `bell_certified` — the chain-side gate.

/// `|S| × 1000 = 2000` is the local-realism (classical) boundary.
/// Anything strictly above this proves quantum-genuine randomness.
pub const LOCAL_REALISM_S_MILLI: u64 = 2000;

/// True iff `s_value_milli > threshold_milli`. Default threshold is
/// `LOCAL_REALISM_S_MILLI = 2000` — strict violation of the CHSH
/// inequality.
pub fn bell_certified(s_value_milli: u64, threshold_milli: u64) -> bool {
    s_value_milli > threshold_milli
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classical_max_not_certified() {
        // S = 2 exactly → not strictly greater → not certified.
        assert!(!bell_certified(LOCAL_REALISM_S_MILLI, LOCAL_REALISM_S_MILLI));
    }

    #[test]
    fn quantum_bell_certified() {
        // 2828 > 2000.
        assert!(bell_certified(2828, LOCAL_REALISM_S_MILLI));
    }

    #[test]
    fn just_above_classical_certified() {
        assert!(bell_certified(2001, LOCAL_REALISM_S_MILLI));
    }

    #[test]
    fn raised_threshold_rejects_marginal() {
        // If chain demands a stronger violation (>2.5σ), 2828 still
        // barely passes 2500, but 2200 wouldn't.
        assert!(bell_certified(2828, 2500));
        assert!(!bell_certified(2200, 2500));
    }
}
