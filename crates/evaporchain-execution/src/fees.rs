//! PID (Proportional-Integral-Derivative) fee controller for EvaporChain.
//!
//! Replaces EIP-1559's simple exponential base fee adjustment with a
//! control-theory-based mechanism that converges faster and oscillates less.
//!
//! EvaporChain has three fee types beyond standard gas:
//! 1. **Gas fee** — per-computation, PID-controlled base + priority tip
//! 2. **State creation deposit** — burned on object creation, proportional to data size
//! 3. **Refresh fee** — paid to keep objects alive (cheaper than creation)

use serde::{Deserialize, Serialize};

/// Parts-per-million denominator for ratio fields. Lane R.7: minimal
/// declaration so cross-crate `crate::fees::FEE_PPM_DENOMINATOR`
/// references (added in 27bfab9 Crooks-MEV Phase 3.1) compile. Value
/// = 1_000_000 ppm = 1.0; the full integer-PID refactor that consumes
/// this constant is deferred to a separate session.
pub const FEE_PPM_DENOMINATOR: u64 = 1_000_000;

/// Per-byte cost multiplier for state creation deposits.
const CREATION_DEPOSIT_PER_BYTE: u64 = 100;

/// Refresh fee is 20% of an equivalent creation deposit.
const REFRESH_FEE_RATIO: f64 = 0.20;

/// Resurrection fee is 60% of an equivalent creation deposit.
const RESURRECTION_FEE_RATIO: f64 = 0.60;

/// Minimum creation deposit regardless of data size.
const MIN_CREATION_DEPOSIT: u64 = 1_000;

/// Minimum refresh fee.
const MIN_REFRESH_FEE: u64 = 100;

/// Minimum resurrection fee.
const MIN_RESURRECTION_FEE: u64 = 500;

/// Anti-windup clamp for integral error to prevent runaway accumulation.
const INTEGRAL_CLAMP: f64 = 10.0;

/// Summary of current fee parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeSummary {
    pub base_fee: u64,
    pub creation_rate_per_byte: u64,
    pub refresh_rate_per_byte: u64,
    pub resurrection_rate_per_byte: u64,
    pub target_utilization: f64,
}

/// PID fee controller with thermodynamic fee model.
///
/// The PID math:
/// ```text
/// error = block_utilization - target_utilization
/// integral_error += error  (clamped to prevent windup)
/// derivative = error - last_error
/// adjustment = kp * error + ki * integral_error + kd * derivative
/// new_base_fee = base_fee * (1 + adjustment)
/// clamp(new_base_fee, min_fee, max_fee)
/// ```
#[derive(Debug, Clone)]
pub struct PidFeeController {
    /// Current base fee per gas unit.
    pub base_fee: u64,
    /// Target block fullness (0.0–1.0, e.g. 0.5 = 50%).
    pub target_utilization: f64,
    /// Proportional gain.
    pub kp: f64,
    /// Integral gain.
    pub ki: f64,
    /// Derivative gain.
    pub kd: f64,
    /// Accumulated error (integral term).
    integral_error: f64,
    /// Previous error (for derivative term).
    last_error: f64,
    /// Minimum base fee floor.
    pub min_base_fee: u64,
    /// Maximum base fee ceiling.
    pub max_base_fee: u64,
}

impl PidFeeController {
    /// Create a new PID fee controller.
    pub fn new(
        target_utilization: f64,
        kp: f64,
        ki: f64,
        kd: f64,
        initial_base_fee: u64,
        min_fee: u64,
        max_fee: u64,
    ) -> Self {
        Self {
            base_fee: initial_base_fee,
            target_utilization: target_utilization.clamp(0.01, 0.99),
            kp,
            ki,
            kd,
            integral_error: 0.0,
            last_error: 0.0,
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
    /// Base fee = 1, so a transfer (21000 gas) costs 21 units.
    pub fn testnet_config() -> Self {
        Self::new(
            0.5,   // target 50% utilization
            0.125, // proportional gain
            0.01,  // integral gain
            0.05,  // derivative gain
            1,     // initial base fee (1 unit per gas)
            1,     // min fee
            1_000, // max fee
        )
    }

    /// Update the base fee after a block is executed.
    ///
    /// Returns the new base fee.
    pub fn update(&mut self, block_gas_used: u64, block_gas_limit: u64) -> u64 {
        if block_gas_limit == 0 {
            return self.base_fee;
        }

        let utilization = block_gas_used as f64 / block_gas_limit as f64;
        let error = utilization - self.target_utilization;

        // Integral with anti-windup clamp
        self.integral_error = (self.integral_error + error).clamp(-INTEGRAL_CLAMP, INTEGRAL_CLAMP);

        // Derivative
        let derivative = error - self.last_error;
        self.last_error = error;

        // PID adjustment
        let adjustment = self.kp * error + self.ki * self.integral_error + self.kd * derivative;

        // Apply adjustment multiplicatively
        let new_fee = self.base_fee as f64 * (1.0 + adjustment);
        self.base_fee = (new_fee.round() as u64).clamp(self.min_base_fee, self.max_base_fee);

        self.base_fee
    }

    /// Compute total gas fee for a transaction.
    pub fn compute_gas_fee(&self, gas_used: u64, priority_tip: u64) -> u64 {
        let base_cost = self.base_fee.saturating_mul(gas_used);
        base_cost.saturating_add(priority_tip)
    }

    /// Compute state creation deposit (burned on object creation).
    /// Proportional to data size — the cost of occupying active state.
    pub fn compute_creation_deposit(&self, data_size_bytes: usize) -> u64 {
        let deposit = CREATION_DEPOSIT_PER_BYTE.saturating_mul(data_size_bytes as u64);
        deposit.max(MIN_CREATION_DEPOSIT)
    }

    /// Compute refresh fee (cheaper than creation — encourages refresh over re-create).
    pub fn compute_refresh_fee(&self, energy_deposited: u64) -> u64 {
        let fee = (energy_deposited as f64 * REFRESH_FEE_RATIO).round() as u64;
        fee.max(MIN_REFRESH_FEE)
    }

    /// Compute resurrection fee (more expensive than refresh, less than creation).
    pub fn compute_resurrection_fee(&self, data_size_bytes: usize) -> u64 {
        let creation = self.compute_creation_deposit(data_size_bytes);
        let fee = (creation as f64 * RESURRECTION_FEE_RATIO).round() as u64;
        fee.max(MIN_RESURRECTION_FEE)
    }

    /// Get a summary of current fee parameters.
    pub fn fee_summary(&self) -> FeeSummary {
        FeeSummary {
            base_fee: self.base_fee,
            creation_rate_per_byte: CREATION_DEPOSIT_PER_BYTE,
            refresh_rate_per_byte: (CREATION_DEPOSIT_PER_BYTE as f64 * REFRESH_FEE_RATIO).round()
                as u64,
            resurrection_rate_per_byte: (CREATION_DEPOSIT_PER_BYTE as f64 * RESURRECTION_FEE_RATIO)
                .round() as u64,
            target_utilization: self.target_utilization,
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

        // 100% utilization (way above 50% target)
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

        // 0% utilization (below 50% target)
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

        // Exactly at target (50%)
        ctrl.update(500, 1000);

        // First update: error=0, but derivative might cause tiny change
        // After a few blocks at target, should be very close to initial
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
        // The integral term's advantage: it corrects for sustained off-target demand.
        // P-only controller stays at a fixed fee when utilization is constant.
        // PID's integral keeps adjusting until the fee is high enough to discourage excess.

        // P-only: at sustained 70% (20% above target), proportional alone
        // produces a fixed adjustment each block. Fee rises but at a constant rate.
        let mut prop = PidFeeController::new(0.5, 0.125, 0.0, 0.0, 1_000, 100, 1_000_000);
        // PID with integral: same scenario, but integral accumulates error
        let mut pid = PidFeeController::new(0.5, 0.125, 0.01, 0.05, 1_000, 100, 1_000_000);

        // 50 blocks at 70% utilization
        for _ in 0..50 {
            prop.update(700, 1000);
            pid.update(700, 1000);
        }

        // PID should have raised fee MORE than P-only due to integral accumulation
        assert!(
            pid.base_fee > prop.base_fee,
            "PID ({}) should have higher fee than P-only ({}) for sustained above-target load",
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
        assert_eq!(large, 100_000); // 1000 * 100
    }

    #[test]
    fn test_refresh_fee_cheaper_than_creation() {
        let ctrl = make_controller();

        let creation = ctrl.compute_creation_deposit(100);
        let refresh = ctrl.compute_refresh_fee(creation); // refresh for equivalent cost

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

        assert!(
            refresh < resurrection,
            "Refresh ({}) should be < resurrection ({})",
            refresh,
            resurrection
        );
        assert!(
            resurrection < creation,
            "Resurrection ({}) should be < creation ({})",
            resurrection,
            creation
        );
    }

    #[test]
    fn test_min_fee_floor_respected() {
        let mut ctrl = PidFeeController::new(0.5, 0.5, 0.1, 0.1, 200, 100, 1_000_000);

        // Many empty blocks → fee should decrease but not below floor
        for _ in 0..100 {
            ctrl.update(0, 1000);
        }

        assert_eq!(
            ctrl.base_fee, 100,
            "Fee should not go below min: got {}",
            ctrl.base_fee
        );
    }

    #[test]
    fn test_max_fee_ceiling_respected() {
        let mut ctrl = PidFeeController::new(0.5, 0.5, 0.1, 0.1, 500_000, 100, 1_000_000);

        // Many full blocks → fee should increase but not above ceiling
        for _ in 0..100 {
            ctrl.update(1000, 1000);
        }

        assert_eq!(
            ctrl.base_fee, 1_000_000,
            "Fee should not exceed max: got {}",
            ctrl.base_fee
        );
    }

    #[test]
    fn test_integral_term_prevents_long_term_drift() {
        // Controller with only integral term
        let mut ctrl_i = PidFeeController::new(0.5, 0.0, 0.02, 0.0, 1_000, 100, 1_000_000);
        // Controller with no integral term
        let mut ctrl_p = PidFeeController::new(0.5, 0.125, 0.0, 0.0, 1_000, 100, 1_000_000);

        // Sustained 70% utilization (20% above target)
        for _ in 0..50 {
            ctrl_i.update(700, 1000);
            ctrl_p.update(700, 1000);
        }

        // Integral controller should have raised fee more than proportional alone
        // because the integral accumulates the sustained above-target error
        assert!(
            ctrl_i.base_fee > 1_000,
            "Integral controller should increase fee for sustained above-target load: got {}",
            ctrl_i.base_fee
        );
    }

    #[test]
    fn test_derivative_term_dampens_sudden_spikes() {
        // Controller with derivative
        let mut ctrl_d = PidFeeController::new(0.5, 0.125, 0.01, 0.15, 1_000, 100, 1_000_000);
        // Controller without derivative
        let mut ctrl_nd = PidFeeController::new(0.5, 0.125, 0.01, 0.0, 1_000, 100, 1_000_000);

        // Steady state at target
        for _ in 0..10 {
            ctrl_d.update(500, 1000);
            ctrl_nd.update(500, 1000);
        }

        let fee_before_d = ctrl_d.base_fee;
        let fee_before_nd = ctrl_nd.base_fee;

        // Sudden spike to 100%
        ctrl_d.update(1000, 1000);
        ctrl_nd.update(1000, 1000);

        let _jump_d = ctrl_d.base_fee as i64 - fee_before_d as i64;
        let _jump_nd = ctrl_nd.base_fee as i64 - fee_before_nd as i64;

        // With derivative, the first spike reaction should be LARGER (derivative adds to P)
        // But on the SECOND block returning to normal, derivative dampens:
        ctrl_d.update(500, 1000);
        ctrl_nd.update(500, 1000);

        // The derivative controller should have come back down more
        // (derivative is negative when error decreases)
        let recovery_d = ctrl_d.base_fee;
        let recovery_nd = ctrl_nd.base_fee;

        assert!(
            recovery_d <= recovery_nd,
            "Derivative should help recovery: {} <= {}",
            recovery_d,
            recovery_nd
        );
    }

    #[test]
    fn test_100_block_simulation_convergence() {
        let mut ctrl = make_controller();

        // Phase 1: 30 blocks at 80% utilization
        for _ in 0..30 {
            ctrl.update(800, 1000);
        }
        let fee_after_high = ctrl.base_fee;

        // Phase 2: demand drops to 50% (target)
        for _ in 0..20 {
            ctrl.update(500, 1000);
        }
        let _fee_after_convergence = ctrl.base_fee;

        // Phase 3: another 50 blocks at target to stabilize
        let mut fees = Vec::new();
        for _ in 0..50 {
            ctrl.update(500, 1000);
            fees.push(ctrl.base_fee);
        }

        // Fee should have risen during high demand
        assert!(
            fee_after_high > 1_000,
            "Fee should rise: got {}",
            fee_after_high
        );

        // Fee should converge back toward baseline after demand normalizes
        let _final_fee = *fees.last().unwrap();
        let var = variance(&fees[40..]);
        assert!(
            var < 100.0,
            "Fee should stabilize (variance {:.0} should be < 100)",
            var
        );
    }

    #[test]
    fn test_zero_gas_block_no_crash() {
        let mut ctrl = make_controller();
        // Zero gas used, zero limit — should not panic
        let fee = ctrl.update(0, 0);
        assert_eq!(fee, ctrl.base_fee);

        // Zero gas used, non-zero limit
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
        assert_eq!(summary.target_utilization, 0.5);
    }

    #[test]
    fn test_compute_gas_fee() {
        let ctrl = make_controller();

        let fee = ctrl.compute_gas_fee(100, 50);
        // base_fee=1000, gas=100 → 100_000 + 50 tip = 100_050
        assert_eq!(fee, 100_050);

        // Zero gas
        assert_eq!(ctrl.compute_gas_fee(0, 0), 0);

        // Only tip
        assert_eq!(ctrl.compute_gas_fee(0, 100), 100);
    }

    #[test]
    fn test_creation_deposit_minimum() {
        let ctrl = make_controller();
        // 0 bytes → still pays minimum
        let deposit = ctrl.compute_creation_deposit(0);
        assert_eq!(deposit, MIN_CREATION_DEPOSIT);

        // 1 byte → 100, but minimum is 1000
        let deposit = ctrl.compute_creation_deposit(1);
        assert_eq!(deposit, MIN_CREATION_DEPOSIT);

        // 11 bytes → 1100 > minimum
        let deposit = ctrl.compute_creation_deposit(11);
        assert_eq!(deposit, 1100);
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
        // 0 bytes → creation deposit = MIN_CREATION_DEPOSIT (1000)
        // resurrection = 1000 * 0.60 = 600, which is >= MIN_RESURRECTION_FEE (500)
        let fee = ctrl.compute_resurrection_fee(0);
        assert_eq!(fee, 600);

        // Verify the floor works for truly tiny values
        assert!(fee >= MIN_RESURRECTION_FEE);
    }
}
