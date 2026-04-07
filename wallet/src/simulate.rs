//! Transaction simulation / dry-run.
//!
//! Preview transaction effects before signing and submitting:
//! - Balance changes (sender and receiver)
//! - Gas and fee costs
//! - Energy impact on objects
//! - Validation checks
//!
//! No state is modified — this is purely predictive.

use serde::Serialize;

use crate::gas::GasEstimator;
use crate::rpc::{RpcClient, RpcError};
use crate::validation;

// ──────────────────────────── Simulation Result ─────────────────────────

/// Outcome of simulating a transaction.
#[derive(Debug, Clone, Serialize)]
pub struct SimulationResult {
    /// Whether the transaction would succeed.
    pub success: bool,
    /// Human-readable summary.
    pub summary: String,
    /// Balance changes (address → delta). Negative means deducted.
    pub balance_changes: Vec<BalanceChange>,
    /// Gas and fee breakdown.
    pub fee: FeeBreakdown,
    /// Energy changes for objects (if applicable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_change: Option<EnergyChange>,
    /// Warnings (non-fatal issues).
    pub warnings: Vec<String>,
    /// Errors (would cause failure).
    pub errors: Vec<String>,
}

/// A balance change for a single address.
#[derive(Debug, Clone, Serialize)]
pub struct BalanceChange {
    pub address: String,
    pub before: u64,
    pub after: i128,
    pub delta: i128,
    pub label: String,
}

/// Fee breakdown for the simulation.
#[derive(Debug, Clone, Serialize)]
pub struct FeeBreakdown {
    pub gas_used: u64,
    pub base_fee: u64,
    pub gas_fee: u64,
    pub extra_fee: u64,
    pub total_fee: u64,
    pub breakdown: String,
}

/// Energy change for an object.
#[derive(Debug, Clone, Serialize)]
pub struct EnergyChange {
    pub object_id: String,
    pub energy_before: u64,
    pub energy_after: u64,
    pub delta: i64,
}

// ──────────────────────────── Simulator ──────────────────────────────────

/// Transaction simulator — previews effects without executing.
pub struct Simulator {
    rpc: RpcClient,
}

impl Simulator {
    /// Create a new simulator with an RPC client (for fetching current state).
    pub fn new(rpc: RpcClient) -> Self {
        Self { rpc }
    }

    /// Simulate a transfer transaction.
    pub async fn simulate_transfer(
        &self,
        from: &str,
        to: &str,
        amount: u64,
    ) -> Result<SimulationResult, RpcError> {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        // Validate inputs
        if let Err(e) = validation::validate_recipient(to) {
            errors.push(format!("Invalid recipient: {}", e));
        }
        if let Err(e) = validation::validate_amount(amount) {
            errors.push(format!("Invalid amount: {}", e));
        }

        // Fetch sender state
        let sender = self.rpc.get_address_detail(from).await?;

        // Estimate fees
        let estimator = GasEstimator::from_rpc(&self.rpc).await.unwrap_or_else(|_| {
            warnings.push("Could not fetch base fee — using default".into());
            GasEstimator::new(100)
        });
        let fee_est = estimator.estimate_transfer();
        let total_cost = amount + fee_est.total_fee;

        // Check balance
        if sender.balance < total_cost {
            errors.push(format!(
                "Insufficient balance: have {} EVAP, need {} EVAP (amount {} + fee {})",
                sender.balance, total_cost, amount, fee_est.total_fee
            ));
        }

        // Check if sending to self
        if from == to {
            warnings.push("Sending to yourself — this will only cost gas fees".into());
        }

        // Large transfer warning
        if sender.balance > 0 && amount > sender.balance * 90 / 100 {
            warnings.push(format!(
                "Large transfer: sending {}% of your balance",
                amount * 100 / sender.balance
            ));
        }

        // Build balance changes
        let sender_after = sender.balance as i128 - total_cost as i128;
        let balance_changes = vec![
            BalanceChange {
                address: from.to_string(),
                before: sender.balance,
                after: sender_after,
                delta: -(total_cost as i128),
                label: "sender (amount + fees)".to_string(),
            },
            BalanceChange {
                address: to.to_string(),
                before: 0, // we don't always know receiver's balance
                after: amount as i128,
                delta: amount as i128,
                label: "receiver".to_string(),
            },
        ];

        let success = errors.is_empty();
        let summary = if success {
            format!(
                "Transfer {} EVAP from {} to {} (fee: {} EVAP, remaining: {} EVAP)",
                amount,
                truncate_addr(from),
                truncate_addr(to),
                fee_est.total_fee,
                sender.balance.saturating_sub(total_cost)
            )
        } else {
            format!("Transfer would FAIL: {}", errors.join("; "))
        };

        Ok(SimulationResult {
            success,
            summary,
            balance_changes,
            fee: FeeBreakdown {
                gas_used: fee_est.gas_used,
                base_fee: fee_est.base_fee,
                gas_fee: fee_est.gas_fee,
                extra_fee: fee_est.extra_fee,
                total_fee: fee_est.total_fee,
                breakdown: fee_est.breakdown,
            },
            energy_change: None,
            warnings,
            errors,
        })
    }

    /// Simulate a refresh transaction.
    pub async fn simulate_refresh(
        &self,
        from: &str,
        object_id: &str,
        energy_deposit: u64,
    ) -> Result<SimulationResult, RpcError> {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        // Validate inputs
        if let Err(e) = validation::validate_energy(energy_deposit) {
            errors.push(format!("Invalid energy: {}", e));
        }

        // Fetch sender state
        let sender = self.rpc.get_address_detail(from).await?;

        // Estimate fees
        let estimator = GasEstimator::from_rpc(&self.rpc).await.unwrap_or_else(|_| {
            warnings.push("Could not fetch base fee — using default".into());
            GasEstimator::new(100)
        });
        let gas_fee = estimator.base_fee() * crate::gas::GAS_REFRESH;
        let refresh_fee = estimator.refresh_fee(energy_deposit);
        let total_fee = gas_fee + refresh_fee;

        // Check balance
        if sender.balance < total_fee {
            errors.push(format!(
                "Insufficient balance: have {} EVAP, need {} EVAP (gas {} + refresh fee {})",
                sender.balance, total_fee, gas_fee, refresh_fee
            ));
        }

        // Try to fetch object state
        let energy_change = match self.rpc.get_object(object_id).await {
            Ok(obj) => {
                if obj.state == "evaporated" || obj.state == "ghost" {
                    warnings.push("Object has evaporated — use resurrect instead of refresh".into());
                }
                if obj.current_energy + energy_deposit > obj.max_energy {
                    warnings.push(format!(
                        "Energy deposit exceeds max: {} + {} > {} max (excess will be wasted)",
                        obj.current_energy, energy_deposit, obj.max_energy
                    ));
                }
                Some(EnergyChange {
                    object_id: object_id.to_string(),
                    energy_before: obj.current_energy,
                    energy_after: (obj.current_energy + energy_deposit).min(obj.max_energy),
                    delta: energy_deposit as i64,
                })
            }
            Err(_) => {
                warnings.push("Could not fetch object state — cannot predict energy change".into());
                None
            }
        };

        let balance_changes = vec![BalanceChange {
            address: from.to_string(),
            before: sender.balance,
            after: sender.balance as i128 - total_fee as i128,
            delta: -(total_fee as i128),
            label: "sender (gas + refresh fee)".to_string(),
        }];

        let success = errors.is_empty();
        let summary = if success {
            format!(
                "Refresh object {} with {} energy (fee: {} EVAP)",
                truncate_addr(object_id),
                energy_deposit,
                total_fee
            )
        } else {
            format!("Refresh would FAIL: {}", errors.join("; "))
        };

        Ok(SimulationResult {
            success,
            summary,
            balance_changes,
            fee: FeeBreakdown {
                gas_used: crate::gas::GAS_REFRESH,
                base_fee: estimator.base_fee(),
                gas_fee,
                extra_fee: refresh_fee,
                total_fee,
                breakdown: format!(
                    "gas: {} × {} = {} + refresh fee {} = {} total",
                    estimator.base_fee(),
                    crate::gas::GAS_REFRESH,
                    gas_fee,
                    refresh_fee,
                    total_fee
                ),
            },
            energy_change,
            warnings,
            errors,
        })
    }

    /// Simulate a create-object transaction.
    pub async fn simulate_create_object(
        &self,
        from: &str,
        energy: u64,
        half_life: u64,
        data_size: usize,
    ) -> Result<SimulationResult, RpcError> {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        if let Err(e) = validation::validate_energy(energy) {
            errors.push(format!("Invalid energy: {}", e));
        }
        if let Err(e) = validation::validate_half_life(half_life) {
            errors.push(format!("Invalid half-life: {}", e));
        }

        let sender = self.rpc.get_address_detail(from).await?;

        let estimator = GasEstimator::from_rpc(&self.rpc).await.unwrap_or_else(|_| {
            warnings.push("Could not fetch base fee — using default".into());
            GasEstimator::new(100)
        });
        let fee_est = estimator.estimate_create_object(data_size);

        if sender.balance < fee_est.total_fee {
            errors.push(format!(
                "Insufficient balance: have {} EVAP, need {} EVAP",
                sender.balance, fee_est.total_fee
            ));
        }

        if half_life < 10 {
            warnings.push("Very short half-life — object will decay rapidly".into());
        }

        let balance_changes = vec![BalanceChange {
            address: from.to_string(),
            before: sender.balance,
            after: sender.balance as i128 - fee_est.total_fee as i128,
            delta: -(fee_est.total_fee as i128),
            label: "sender (gas + creation deposit)".to_string(),
        }];

        let success = errors.is_empty();
        let summary = if success {
            format!(
                "Create object: {} energy, {} half-life, {} bytes (fee: {} EVAP)",
                energy, half_life, data_size, fee_est.total_fee
            )
        } else {
            format!("Create object would FAIL: {}", errors.join("; "))
        };

        Ok(SimulationResult {
            success,
            summary,
            balance_changes,
            fee: FeeBreakdown {
                gas_used: fee_est.gas_used,
                base_fee: fee_est.base_fee,
                gas_fee: fee_est.gas_fee,
                extra_fee: fee_est.extra_fee,
                total_fee: fee_est.total_fee,
                breakdown: fee_est.breakdown,
            },
            energy_change: None,
            warnings,
            errors,
        })
    }
}

// ──────────────────────────── Standalone Simulation ──────────────────────

/// Simulate a transfer without RPC (offline, using provided state).
pub fn simulate_transfer_offline(
    sender_balance: u64,
    amount: u64,
    base_fee: u64,
) -> SimulationResult {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    if let Err(e) = validation::validate_amount(amount) {
        errors.push(format!("Invalid amount: {}", e));
    }

    let estimator = GasEstimator::new(base_fee);
    let fee_est = estimator.estimate_transfer();
    let total_cost = amount + fee_est.total_fee;

    if sender_balance < total_cost {
        errors.push(format!(
            "Insufficient balance: have {} EVAP, need {} EVAP",
            sender_balance, total_cost
        ));
    }

    if sender_balance > 0 && amount > sender_balance * 90 / 100 {
        warnings.push(format!(
            "Large transfer: {}% of balance",
            amount * 100 / sender_balance
        ));
    }

    let balance_changes = vec![BalanceChange {
        address: "sender".to_string(),
        before: sender_balance,
        after: sender_balance as i128 - total_cost as i128,
        delta: -(total_cost as i128),
        label: "sender (amount + fees)".to_string(),
    }];

    let success = errors.is_empty();
    let summary = if success {
        format!(
            "Transfer {} EVAP (fee: {} EVAP, remaining: {} EVAP)",
            amount,
            fee_est.total_fee,
            sender_balance.saturating_sub(total_cost)
        )
    } else {
        format!("Transfer would FAIL: {}", errors.join("; "))
    };

    SimulationResult {
        success,
        summary,
        balance_changes,
        fee: FeeBreakdown {
            gas_used: fee_est.gas_used,
            base_fee: fee_est.base_fee,
            gas_fee: fee_est.gas_fee,
            extra_fee: fee_est.extra_fee,
            total_fee: fee_est.total_fee,
            breakdown: fee_est.breakdown,
        },
        energy_change: None,
        warnings,
        errors,
    }
}

/// Simulate a refresh without RPC (offline, using provided state).
pub fn simulate_refresh_offline(
    sender_balance: u64,
    object_energy: u64,
    max_energy: u64,
    energy_deposit: u64,
    base_fee: u64,
) -> SimulationResult {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    if let Err(e) = validation::validate_energy(energy_deposit) {
        errors.push(format!("Invalid energy: {}", e));
    }

    let estimator = GasEstimator::new(base_fee);
    let gas_fee = base_fee * crate::gas::GAS_REFRESH;
    let refresh_fee = estimator.refresh_fee(energy_deposit);
    let total_fee = gas_fee + refresh_fee;

    if sender_balance < total_fee {
        errors.push(format!(
            "Insufficient balance: have {} EVAP, need {} EVAP",
            sender_balance, total_fee
        ));
    }

    if object_energy + energy_deposit > max_energy {
        warnings.push(format!(
            "Energy deposit exceeds max: {} + {} > {}",
            object_energy, energy_deposit, max_energy
        ));
    }

    let new_energy = (object_energy + energy_deposit).min(max_energy);

    let balance_changes = vec![BalanceChange {
        address: "sender".to_string(),
        before: sender_balance,
        after: sender_balance as i128 - total_fee as i128,
        delta: -(total_fee as i128),
        label: "sender (gas + refresh fee)".to_string(),
    }];

    let success = errors.is_empty();
    let summary = if success {
        format!(
            "Refresh: {} → {} energy (fee: {} EVAP)",
            object_energy, new_energy, total_fee
        )
    } else {
        format!("Refresh would FAIL: {}", errors.join("; "))
    };

    SimulationResult {
        success,
        summary,
        balance_changes,
        fee: FeeBreakdown {
            gas_used: crate::gas::GAS_REFRESH,
            base_fee,
            gas_fee,
            extra_fee: refresh_fee,
            total_fee,
            breakdown: format!(
                "gas: {} × {} = {} + refresh fee {} = {} total",
                base_fee,
                crate::gas::GAS_REFRESH,
                gas_fee,
                refresh_fee,
                total_fee
            ),
        },
        energy_change: Some(EnergyChange {
            object_id: "object".to_string(),
            energy_before: object_energy,
            energy_after: new_energy,
            delta: energy_deposit as i64,
        }),
        warnings,
        errors,
    }
}

// ──────────────────────────── Helpers ────────────────────────────────────

fn truncate_addr(addr: &str) -> String {
    if addr.len() > 16 {
        format!("{}...{}", &addr[..8], &addr[addr.len() - 6..])
    } else {
        addr.to_string()
    }
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simulate_transfer_offline_success() {
        let result = simulate_transfer_offline(10_000_000, 1_000, 100);
        assert!(result.success);
        assert!(result.errors.is_empty());
        assert!(result.summary.contains("1000 EVAP"));
        assert_eq!(result.balance_changes.len(), 1);
        assert!(result.balance_changes[0].delta < 0);
    }

    #[test]
    fn test_simulate_transfer_offline_insufficient_balance() {
        let result = simulate_transfer_offline(100, 1_000, 100);
        assert!(!result.success);
        assert!(!result.errors.is_empty());
        assert!(result.errors[0].contains("Insufficient balance"));
    }

    #[test]
    fn test_simulate_transfer_offline_zero_amount() {
        let result = simulate_transfer_offline(10_000, 0, 100);
        assert!(!result.success);
        assert!(result.errors[0].contains("Invalid amount"));
    }

    #[test]
    fn test_simulate_transfer_offline_large_warning() {
        // 95% of balance
        let result = simulate_transfer_offline(10_000, 9_500, 1);
        // May or may not succeed depending on gas, but should have warning
        assert!(result.warnings.iter().any(|w| w.contains("Large transfer")));
    }

    #[test]
    fn test_simulate_transfer_offline_fee_breakdown() {
        let result = simulate_transfer_offline(10_000_000, 1_000, 100);
        assert_eq!(result.fee.gas_used, crate::gas::GAS_TRANSFER);
        assert_eq!(result.fee.base_fee, 100);
        assert_eq!(result.fee.gas_fee, 100 * crate::gas::GAS_TRANSFER);
        assert_eq!(result.fee.extra_fee, 0);
        assert!(!result.fee.breakdown.is_empty());
    }

    #[test]
    fn test_simulate_refresh_offline_success() {
        let result = simulate_refresh_offline(10_000_000, 500, 10_000, 1_000, 100);
        assert!(result.success);
        assert!(result.errors.is_empty());
        let ec = result.energy_change.unwrap();
        assert_eq!(ec.energy_before, 500);
        assert_eq!(ec.energy_after, 1_500);
        assert_eq!(ec.delta, 1_000);
    }

    #[test]
    fn test_simulate_refresh_offline_exceeds_max() {
        let result = simulate_refresh_offline(10_000_000, 9_000, 10_000, 5_000, 100);
        assert!(result.warnings.iter().any(|w| w.contains("exceeds max")));
        let ec = result.energy_change.unwrap();
        assert_eq!(ec.energy_after, 10_000); // capped at max
    }

    #[test]
    fn test_simulate_refresh_offline_insufficient_balance() {
        let result = simulate_refresh_offline(100, 500, 10_000, 1_000, 100);
        assert!(!result.success);
        assert!(result.errors[0].contains("Insufficient balance"));
    }

    #[test]
    fn test_simulate_refresh_offline_zero_energy() {
        let result = simulate_refresh_offline(10_000_000, 500, 10_000, 0, 100);
        assert!(!result.success);
        assert!(result.errors[0].contains("Invalid energy"));
    }

    #[test]
    fn test_truncate_addr_short() {
        assert_eq!(truncate_addr("0xabc"), "0xabc");
    }

    #[test]
    fn test_truncate_addr_long() {
        let addr = format!("0x{}", "ab".repeat(32));
        let truncated = truncate_addr(&addr);
        assert!(truncated.contains("..."));
        assert!(truncated.len() < addr.len());
    }

    #[test]
    fn test_simulation_result_serializable() {
        let result = simulate_transfer_offline(10_000_000, 1_000, 100);
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("balance_changes"));
        assert!(json.contains("fee"));
    }

    #[test]
    fn test_fee_breakdown_serializable() {
        let fb = FeeBreakdown {
            gas_used: 21_000,
            base_fee: 100,
            gas_fee: 2_100_000,
            extra_fee: 0,
            total_fee: 2_100_000,
            breakdown: "test".to_string(),
        };
        let json = serde_json::to_string(&fb).unwrap();
        assert!(json.contains("\"gas_used\":21000"));
    }

    #[test]
    fn test_balance_change_serializable() {
        let bc = BalanceChange {
            address: "0xabc".to_string(),
            before: 1000,
            after: 500,
            delta: -500,
            label: "test".to_string(),
        };
        let json = serde_json::to_string(&bc).unwrap();
        assert!(json.contains("\"delta\":-500"));
    }

    #[test]
    fn test_energy_change_serializable() {
        let ec = EnergyChange {
            object_id: "0x123".to_string(),
            energy_before: 100,
            energy_after: 600,
            delta: 500,
        };
        let json = serde_json::to_string(&ec).unwrap();
        assert!(json.contains("\"delta\":500"));
    }
}
