//! PID (Proportional-Integral-Derivative) fee controller for EvaporChain.
//!
//! Replaces EIP-1559's simple exponential base fee adjustment with a
//! control-theory-based mechanism that converges faster and oscillates less.
//!
//! ## Validator-determinism
//!
//! All PID math runs on **integer parts-per-million (ppm)** with i128
//! intermediate. The public `new()` constructor accepts f64 gains for
//! ergonomic callers; conversion to ppm happens **once at construction**
//! (always with literal-equivalent values across validators), and every
//! per-tick update + fee computation is pure-integer thereafter.
//!
//! EvaporChain has three fee types beyond standard gas:
//! 1. **Gas fee** — per-computation, PID-controlled base + priority tip
//! 2. **State creation deposit** — burned on object creation, proportional to data size
//! 3. **Refresh fee** — paid to keep objects alive (cheaper than creation)

use serde::{Deserialize, Serialize};

/// Parts-per-million denominator for ratio fields.
pub const FEE_PPM_DENOMINATOR: u64 = 1_000_000;

/// Per-byte cost multiplier for state creation deposits.
const CREATION_DEPOSIT_PER_BYTE: u64 = 100;

/// Refresh fee is 20% of an equivalent creation deposit.
const REFRESH_FEE_RATIO_PPM: u64 = 200_000;

/// Resurrection fee is 60% of an equivalent creation deposit.
const RESURRECTION_FEE_RATIO_PPM: u64 = 600_000;

/// Minimum creation deposit regardless of data size.
const MIN_CREATION_DEPOSIT: u64 = 1_000;

/// Minimum refresh fee.
const MIN_REFRESH_FEE: u64 = 100;

/// Minimum resurrection fee.
const MIN_RESURRECTION_FEE: u64 = 500;

/// Anti-windup clamp for integral error (in ppm). 10.0 in float terms.
const INTEGRAL_CLAMP_PPM: i64 = 10 * FEE_PPM_DENOMINATOR as i64;

/// Convert an f64 ratio in [0, 1] (or signed in [-1, 1]) to ppm.
/// Used only at construction-time boundaries, never on the per-tick path.
fn f64_to_ppm(v: f64) -> i64 {
    (v * FEE_PPM_DENOMINATOR as f64) as i64
}

/// Summary of current fee parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeSummary {
    pub base_fee: u64,
    pub creation_rate_per_byte: u64,
    pub refresh_rate_per_byte: u64,
    pub resurrection_rate_per_byte: u64,
    pub target_utilization: f64,
}

/// PID fee controller — pure-integer ppm math.
///
/// Per-tick math:
/// ```text
/// error_ppm = utilization_ppm - target_utilization_ppm
/// integral_error_ppm += error_ppm  (clamped to ±INTEGRAL_CLAMP_PPM)
/// derivative_ppm = error_ppm - last_error_ppm
/// adjustment_ppm = (kp_ppm * error_ppm
///                  + ki_ppm * integral_error_ppm
///                  + kd_ppm * derivative_ppm) / PPM_DENOMINATOR
/// new_base_fee = base_fee * (PPM + adjustment_ppm) / PPM
/// clamp(new_base_fee, min_fee, max_fee)
/// ```
#[derive(Debug, Clone)]
pub struct PidFeeController {
    /// Current base fee per gas unit.
    pub base_fee: u64,
    /// Target block fullness in ppm (0..=1_000_000).
    pub target_utilization_ppm: u32,
    /// Proportional gain in ppm. 1.0 == 1_000_000 ppm.
    pub kp_ppm: u32,
    /// Integral gain in ppm.
    pub ki_ppm: u32,
    /// Derivative gain in ppm.
    pub kd_ppm: u32,
    /// Accumulated error (integral term) in ppm. Signed.
    integral_error_ppm: i64,
    /// Previous error (for derivative term) in ppm. Signed.
    last_error_ppm: i64,
    /// Minimum base fee floor.
    pub min_base_fee: u64,
    /// Maximum base fee ceiling.
    pub max_base_fee: u64,
}

impl PidFeeController {
    /// Create a new PID fee controller.
    ///
    /// f64 args are converted to ppm **once at construction**. All
    /// subsequent per-tick math is pure-integer.
    pub fn new(
        target_utilization: f64,
        kp: f64,
        ki: f64,
        kd: f64,
        initial_base_fee: u64,
        min_fee: u64,
        max_fee: u64,
    ) -> Self {
        let target_clamped = target_utilization.clamp(0.01, 0.99);
        Self {
            base_fee: initial_base_fee,
            target_utilization_ppm: f64_to_ppm(target_clamped) as u32,
            kp_ppm: f64_to_ppm(kp) as u32,
            ki_ppm: f64_to_ppm(ki) as u32,
            kd_ppm: f64_to_ppm(kd) as u32,
            integral_error_ppm: 0,
            last_error_ppm: 0,
            min_base_fee: min_fee,
            max_base_fee: max_fee,
        }
    }

    /// Create a controller with sensible defaults for EvaporChain.
    pub fn default_config() -> Self {
        Self::new(
            0.5,       // target 50% utilization
            0.125,     // proportional gain
            0.01,      // integral gain
            0.05,      // derivative gain
            1_000,     // initial base fee
            100,       // min fee
            1_000_000, // max fee
        )
    }

    /// Create a controller with low fees suitable for testnet.
    pub fn testnet_config() -> Self {
        Self::new(
            0.5,   // target 50% utilization
            0.125, // proportional gain
            0.01,  // integral gain
            0.05,  // derivative gain
            1,     // initial base fee
            1,     // min fee
            1_000, // max fee
        )
    }

    /// Update the base fee after a block is executed.
    ///
    /// Pure-integer per-tick math; same byte-exact result on every
    /// validator regardless of architecture.
    pub fn update(&mut self, block_gas_used: u64, block_gas_limit: u64) -> u64 {
        if block_gas_limit == 0 {
            return self.base_fee;
        }

        // utilization_ppm = block_gas_used * PPM / block_gas_limit
        let utilization_ppm = (block_gas_used as u128)
            .saturating_mul(FEE_PPM_DENOMINATOR as u128)
            / block_gas_limit as u128;
        let utilization_ppm = utilization_ppm.min(FEE_PPM_DENOMINATOR as u128) as i64;
        let error_ppm: i64 = utilization_ppm - self.target_utilization_ppm as i64;

        // Integral with anti-windup clamp.
        self.integral_error_ppm = (self.integral_error_ppm.saturating_add(error_ppm))
            .clamp(-INTEGRAL_CLAMP_PPM, INTEGRAL_CLAMP_PPM);

        // Derivative.
        let derivative_ppm: i64 = error_ppm - self.last_error_ppm;
        self.last_error_ppm = error_ppm;

        // PID adjustment in ppm-units. i128 intermediate to avoid
        // overflow when gains × clamped errors approach 1e13.
        let adjustment_ppm_i128: i128 = (self.kp_ppm as i128 * error_ppm as i128
            + self.ki_ppm as i128 * self.integral_error_ppm as i128
            + self.kd_ppm as i128 * derivative_ppm as i128)
            / FEE_PPM_DENOMINATOR as i128;

        // new_fee = base_fee * (PPM + adjustment_ppm) / PPM, signed.
        let factor = FEE_PPM_DENOMINATOR as i128 + adjustment_ppm_i128;
        let new_fee_signed: i128 = if factor <= 0 {
            self.min_base_fee as i128
        } else {
            (self.base_fee as i128).saturating_mul(factor) / FEE_PPM_DENOMINATOR as i128
        };
        let new_fee = if new_fee_signed < 0 {
            0u64
        } else if new_fee_signed > u64::MAX as i128 {
            u64::MAX
        } else {
            new_fee_signed as u64
        };
        self.base_fee = new_fee.clamp(self.min_base_fee, self.max_base_fee);

        self.base_fee
    }

    /// Compute total gas fee for a transaction.
    pub fn compute_gas_fee(&self, gas_used: u64, priority_tip: u64) -> u64 {
        let base_cost = self.base_fee.saturating_mul(gas_used);
        base_cost.saturating_add(priority_tip)
    }

    /// Compute state creation deposit (burned on object creation).
    pub fn compute_creation_deposit(&self, data_size_bytes: usize) -> u64 {
        let deposit = CREATION_DEPOSIT_PER_BYTE.saturating_mul(data_size_bytes as u64);
        deposit.max(MIN_CREATION_DEPOSIT)
    }

    /// Compute refresh fee. Pure-integer ppm math.
    pub fn compute_refresh_fee(&self, energy_deposited: u64) -> u64 {
        // fee = floor(energy * REFRESH_RATIO_PPM / PPM)
        let fee = (energy_deposited as u128)
            .saturating_mul(REFRESH_FEE_RATIO_PPM as u128)
            / FEE_PPM_DENOMINATOR as u128;
        let fee = fee.min(u64::MAX as u128) as u64;
        fee.max(MIN_REFRESH_FEE)
    }

    /// Compute resurrection fee. Pure-integer ppm math.
    pub fn compute_resurrection_fee(&self, data_size_bytes: usize) -> u64 {
        let creation = self.compute_creation_deposit(data_size_bytes);
        let fee = (creation as u128)
            .saturating_mul(RESURRECTION_FEE_RATIO_PPM as u128)
            / FEE_PPM_DENOMINATOR as u128;
        let fee = fee.min(u64::MAX as u128) as u64;
        fee.max(MIN_RESURRECTION_FEE)
    }

    /// Get a summary of current fee parameters.
    pub fn fee_summary(&self) -> FeeSummary {
        let refresh_rate = (CREATION_DEPOSIT_PER_BYTE as u128)
            .saturating_mul(REFRESH_FEE_RATIO_PPM as u128)
            / FEE_PPM_DENOMINATOR as u128;
        let resurrection_rate = (CREATION_DEPOSIT_PER_BYTE as u128)
            .saturating_mul(RESURRECTION_FEE_RATIO_PPM as u128)
            / FEE_PPM_DENOMINATOR as u128;
        FeeSummary {
            base_fee: self.base_fee,
            creation_rate_per_byte: CREATION_DEPOSIT_PER_BYTE,
            refresh_rate_per_byte: refresh_rate as u64,
            resurrection_rate_per_byte: resurrection_rate as u64,
            // f64 view-only conversion at the public-API boundary.
            target_utilization: self.target_utilization_ppm as f64
                / FEE_PPM_DENOMINATOR as f64,
        }
    }
}

// ─────────────────────────── Tests ───────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_controller() -> PidFeeController {
        PidFeeController::new(0.5, 0.125, 0.01, 0.05, 1_000, 100, 1_000_000)
    }

    #[test]
    fn test_base_fee_increases_when_blocks_full() {
        let mut ctrl = make_controller();
        let initial = ctrl.base_fee;
        ctrl.update(1000, 1000);
        assert!(
            ctrl.base_fee > initial,
            "Expected increase: {} > {}",
            ctrl.base_fee,
            initial
        );
    }

    #[test]
    fn test_base_fee_decreases_when_blocks_empty() {
        let mut ctrl = make_controller();
        let initial = ctrl.base_fee;
        ctrl.update(0, 1000);
        assert!(
            ctrl.base_fee < initial,
            "Expected decrease: {} < {}",
            ctrl.base_fee,
            initial
        );
    }

    #[test]
    fn test_base_fee_stable_at_target() {
        let mut ctrl = make_controller();
        let initial = ctrl.base_fee;
        ctrl.update(500, 1000);
        for _ in 0..10 {
            ctrl.update(500, 1000);
        }
        let diff = (ctrl.base_fee as i64 - initial as i64).unsigned_abs();
        assert!(
            diff <= 10,
            "Fee should converge near initial at target: got {} vs {}",
            ctrl.base_fee,
            initial
        );
    }

    #[test]
    fn test_pid_integral_corrects_sustained_bias() {
        let mut prop = PidFeeController::new(0.5, 0.125, 0.0, 0.0, 1_000, 100, 1_000_000);
        let mut pid = PidFeeController::new(0.5, 0.125, 0.01, 0.05, 1_000, 100, 1_000_000);
        for _ in 0..50 {
            prop.update(700, 1000);
            pid.update(700, 1000);
        }
        assert!(
            pid.base_fee > prop.base_fee,
            "PID ({}) should have higher fee than P-only ({})",
            pid.base_fee,
            prop.base_fee
        );
    }

    fn variance(values: &[u64]) -> f64 {
        let n = values.len() as f64;
        let mean = values.iter().sum::<u64>() as f64 / n;
        values
            .iter()
            .map(|&v| (v as f64 - mean).powi(2))
            .sum::<f64>()
            / n
    }

    #[test]
    fn test_creation_deposit_scales_with_data_size() {
        let ctrl = make_controller();
        let small = ctrl.compute_creation_deposit(10);
        let medium = ctrl.compute_creation_deposit(100);
        let large = ctrl.compute_creation_deposit(1000);
        assert!(small <= medium);
        assert!(medium < large);
        assert_eq!(large, 100_000);
    }

    #[test]
    fn test_refresh_fee_cheaper_than_creation() {
        let ctrl = make_controller();
        let creation = ctrl.compute_creation_deposit(100);
        let refresh = ctrl.compute_refresh_fee(creation);
        assert!(
            refresh < creation,
            "Refresh ({}) should be cheaper than creation ({})",
            refresh,
            creation
        );
    }

    #[test]
    fn test_resurrection_fee_between_refresh_and_creation() {
        let ctrl = make_controller();
        let size = 200;
        let creation = ctrl.compute_creation_deposit(size);
        let resurrection = ctrl.compute_resurrection_fee(size);
        let refresh = ctrl.compute_refresh_fee(creation);
        assert!(refresh < resurrection);
        assert!(resurrection < creation);
    }

    #[test]
    fn test_min_fee_floor_respected() {
        let mut ctrl = PidFeeController::new(0.5, 0.5, 0.1, 0.1, 200, 100, 1_000_000);
        for _ in 0..100 {
            ctrl.update(0, 1000);
        }
        assert_eq!(ctrl.base_fee, 100);
    }

    #[test]
    fn test_max_fee_ceiling_respected() {
        let mut ctrl = PidFeeController::new(0.5, 0.5, 0.1, 0.1, 500_000, 100, 1_000_000);
        for _ in 0..100 {
            ctrl.update(1000, 1000);
        }
        assert_eq!(ctrl.base_fee, 1_000_000);
    }

    #[test]
    fn test_integral_term_prevents_long_term_drift() {
        let mut ctrl_i = PidFeeController::new(0.5, 0.0, 0.02, 0.0, 1_000, 100, 1_000_000);
        let mut ctrl_p = PidFeeController::new(0.5, 0.125, 0.0, 0.0, 1_000, 100, 1_000_000);
        for _ in 0..50 {
            ctrl_i.update(700, 1000);
            ctrl_p.update(700, 1000);
        }
        assert!(
            ctrl_i.base_fee > 1_000,
            "Integral controller should increase fee: got {}",
            ctrl_i.base_fee
        );
    }

    #[test]
    fn test_derivative_term_dampens_sudden_spikes() {
        let mut ctrl_d = PidFeeController::new(0.5, 0.125, 0.01, 0.15, 1_000, 100, 1_000_000);
        let mut ctrl_nd = PidFeeController::new(0.5, 0.125, 0.01, 0.0, 1_000, 100, 1_000_000);
        for _ in 0..10 {
            ctrl_d.update(500, 1000);
            ctrl_nd.update(500, 1000);
        }
        let _fee_before_d = ctrl_d.base_fee;
        let _fee_before_nd = ctrl_nd.base_fee;
        ctrl_d.update(1000, 1000);
        ctrl_nd.update(1000, 1000);
        ctrl_d.update(500, 1000);
        ctrl_nd.update(500, 1000);
        let recovery_d = ctrl_d.base_fee;
        let recovery_nd = ctrl_nd.base_fee;
        assert!(recovery_d <= recovery_nd);
    }

    #[test]
    fn test_100_block_simulation_convergence() {
        let mut ctrl = make_controller();
        for _ in 0..30 {
            ctrl.update(800, 1000);
        }
        let fee_after_high = ctrl.base_fee;
        for _ in 0..20 {
            ctrl.update(500, 1000);
        }
        let mut fees = Vec::new();
        for _ in 0..50 {
            ctrl.update(500, 1000);
            fees.push(ctrl.base_fee);
        }
        assert!(fee_after_high > 1_000);
        let var = variance(&fees[40..]);
        assert!(var < 100.0, "Fee should stabilize (variance {:.0})", var);
    }

    #[test]
    fn test_zero_gas_block_no_crash() {
        let mut ctrl = make_controller();
        let fee = ctrl.update(0, 0);
        assert_eq!(fee, ctrl.base_fee);
        let fee = ctrl.update(0, 1000);
        assert!(fee <= ctrl.base_fee || fee >= ctrl.min_base_fee);
    }

    #[test]
    fn test_fee_summary_returns_correct_values() {
        let ctrl = make_controller();
        let summary = ctrl.fee_summary();
        assert_eq!(summary.base_fee, 1_000);
        assert_eq!(summary.creation_rate_per_byte, 100);
        assert_eq!(summary.refresh_rate_per_byte, 20); // 100 * 0.20
        assert_eq!(summary.resurrection_rate_per_byte, 60); // 100 * 0.60
        assert!((summary.target_utilization - 0.5).abs() < 1e-9);
    }

    #[test]
    fn test_compute_gas_fee() {
        let ctrl = make_controller();
        let fee = ctrl.compute_gas_fee(100, 50);
        assert_eq!(fee, 100_050);
        assert_eq!(ctrl.compute_gas_fee(0, 0), 0);
        assert_eq!(ctrl.compute_gas_fee(0, 100), 100);
    }

    #[test]
    fn test_creation_deposit_minimum() {
        let ctrl = make_controller();
        assert_eq!(ctrl.compute_creation_deposit(0), MIN_CREATION_DEPOSIT);
        assert_eq!(ctrl.compute_creation_deposit(1), MIN_CREATION_DEPOSIT);
        assert_eq!(ctrl.compute_creation_deposit(11), 1100);
    }

    #[test]
    fn test_refresh_fee_minimum() {
        let ctrl = make_controller();
        let fee = ctrl.compute_refresh_fee(0);
        assert_eq!(fee, MIN_REFRESH_FEE);
    }

    #[test]
    fn test_resurrection_fee_minimum() {
        let ctrl = make_controller();
        let fee = ctrl.compute_resurrection_fee(0);
        assert_eq!(fee, 600);
        assert!(fee >= MIN_RESURRECTION_FEE);
    }

    /// Validator-determinism witness: PID update produces byte-exact
    /// same `base_fee` across two independent controllers driven with
    /// identical input sequences. No f64 anywhere on the per-tick path.
    #[test]
    fn test_pid_update_is_validator_deterministic() {
        let mut a = make_controller();
        let mut b = make_controller();
        let trace = [(800, 1000), (300, 1000), (1000, 1000), (0, 1000), (500, 1000)];
        for (used, limit) in trace {
            a.update(used, limit);
            b.update(used, limit);
            assert_eq!(a.base_fee, b.base_fee, "PID update must be deterministic");
        }
    }

    /// Pure-integer refresh / resurrection — same byte-exact result
    /// from any architecture.
    #[test]
    fn test_refresh_resurrection_deterministic() {
        let ctrl = make_controller();
        // refresh of 1000 at 20% ppm = 200; resurrection of 100 bytes at 60% = 6000
        assert_eq!(ctrl.compute_refresh_fee(1000), 200);
        assert_eq!(ctrl.compute_resurrection_fee(100), 6000);
    }
}
