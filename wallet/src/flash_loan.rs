//! Flash loan composition and simulation engine.
//!
//! Provides tools for composing multi-step flash loan plans, simulating
//! their execution, assessing risk, and tracking lifecycle state.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum FlashLoanError {
    #[error("plan not found: {0}")]
    PlanNotFound(String),
    #[error("action index out of bounds: {0}")]
    ActionIndexOutOfBounds(usize),
    #[error("invalid state transition: plan {0} is {1}, expected {2}")]
    InvalidState(String, String, String),
    #[error("simulation failed: {0}")]
    SimulationFailed(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ──────────────────────────── Types ──────────────────────────────────────

/// Status of a flash loan plan through its lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LoanStatus {
    Draft,
    Submitted,
    Executed,
    Failed,
    Cancelled,
}

impl std::fmt::Display for LoanStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Draft => write!(f, "Draft"),
            Self::Submitted => write!(f, "Submitted"),
            Self::Executed => write!(f, "Executed"),
            Self::Failed => write!(f, "Failed"),
            Self::Cancelled => write!(f, "Cancelled"),
        }
    }
}

/// An individual action within a flash loan plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum FlashAction {
    Borrow {
        token: String,
        amount: u64,
    },
    Swap {
        from: String,
        to: String,
        amount: u64,
    },
    Repay {
        token: String,
        amount: u64,
    },
    Arbitrage {
        token_a: String,
        token_b: String,
        amount: u64,
    },
    Liquidate {
        target: String,
        collateral: String,
        debt: String,
        amount: u64,
    },
}

// ──────────────────────────── Plan ──────────────────────────────────────

/// A composed flash loan plan with ordered actions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlashLoanPlan {
    pub id: String,
    pub name: String,
    pub actions: Vec<FlashAction>,
    pub borrow_token: String,
    pub borrow_amount: u64,
    pub fee_bps: u32,
    pub expected_profit: i64,
    pub status: LoanStatus,
    pub created_at: String,
    pub executed_at: Option<String>,
    pub gas_estimate: u64,
    pub risk_score: u32,
}

// ──────────────────────────── Simulation ─────────────────────────────────

/// Result of simulating a flash loan plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationResult {
    pub plan_id: String,
    pub success: bool,
    pub profit: i64,
    pub gas_used: u64,
    pub steps_completed: usize,
    pub failure_step: Option<usize>,
    pub failure_reason: Option<String>,
}

// ──────────────────────────── Stats ─────────────────────────────────────

/// Aggregate statistics across all flash loan plans.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlashLoanStats {
    pub total_plans: usize,
    pub executed: usize,
    pub successful: usize,
    pub failed: usize,
    pub total_borrowed: u64,
    pub total_profit: i64,
    pub total_fees_paid: u64,
    pub avg_risk_score: u32,
}

// ──────────────────────────── Manager ────────────────────────────────────

/// Manages flash loan plans, simulations, and lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlashLoanManager {
    pub plans: HashMap<String, FlashLoanPlan>,
    pub simulations: Vec<SimulationResult>,
}

impl Default for FlashLoanManager {
    fn default() -> Self {
        Self::new()
    }
}

impl FlashLoanManager {
    /// Create a new empty manager.
    pub fn new() -> Self {
        Self {
            plans: HashMap::new(),
            simulations: Vec::new(),
        }
    }

    /// Create a draft flash loan plan, returning its ID.
    pub fn create_plan(
        &mut self,
        name: &str,
        borrow_token: &str,
        borrow_amount: u64,
        fee_bps: u32,
    ) -> String {
        let ts = chrono::Utc::now().to_rfc3339();
        let id_input = format!("{}{}", name, ts);
        let id = blake3::hash(id_input.as_bytes()).to_hex()[..16].to_string();

        let plan = FlashLoanPlan {
            id: id.clone(),
            name: name.to_string(),
            actions: Vec::new(),
            borrow_token: borrow_token.to_string(),
            borrow_amount,
            fee_bps,
            expected_profit: 0,
            status: LoanStatus::Draft,
            created_at: ts,
            executed_at: None,
            gas_estimate: 0,
            risk_score: 0,
        };

        self.plans.insert(id.clone(), plan);
        id
    }

    /// Append an action to a draft plan.
    pub fn add_action(&mut self, plan_id: &str, action: FlashAction) -> Result<(), FlashLoanError> {
        let plan = self
            .plans
            .get_mut(plan_id)
            .ok_or_else(|| FlashLoanError::PlanNotFound(plan_id.to_string()))?;
        if plan.status != LoanStatus::Draft {
            return Err(FlashLoanError::InvalidState(
                plan_id.to_string(),
                plan.status.to_string(),
                "Draft".to_string(),
            ));
        }
        plan.actions.push(action);
        Ok(())
    }

    /// Remove the action at `index` from a draft plan.
    pub fn remove_action(
        &mut self,
        plan_id: &str,
        index: usize,
    ) -> Result<FlashAction, FlashLoanError> {
        let plan = self
            .plans
            .get_mut(plan_id)
            .ok_or_else(|| FlashLoanError::PlanNotFound(plan_id.to_string()))?;
        if plan.status != LoanStatus::Draft {
            return Err(FlashLoanError::InvalidState(
                plan_id.to_string(),
                plan.status.to_string(),
                "Draft".to_string(),
            ));
        }
        if index >= plan.actions.len() {
            return Err(FlashLoanError::ActionIndexOutOfBounds(index));
        }
        Ok(plan.actions.remove(index))
    }

    /// Get a reference to a plan by ID.
    pub fn get_plan(&self, id: &str) -> Option<&FlashLoanPlan> {
        self.plans.get(id)
    }

    /// List all plans.
    pub fn list_plans(&self) -> Vec<&FlashLoanPlan> {
        self.plans.values().collect()
    }

    /// Simulate a plan's execution without changing its status.
    pub fn simulate(&mut self, plan_id: &str) -> Result<SimulationResult, FlashLoanError> {
        let plan = self
            .plans
            .get(plan_id)
            .ok_or_else(|| FlashLoanError::PlanNotFound(plan_id.to_string()))?;

        // Check a Borrow action exists
        let has_borrow = plan
            .actions
            .iter()
            .any(|a| matches!(a, FlashAction::Borrow { .. }));
        if !has_borrow {
            let result = SimulationResult {
                plan_id: plan_id.to_string(),
                success: false,
                profit: 0,
                gas_used: 0,
                steps_completed: 0,
                failure_step: Some(0),
                failure_reason: Some("no borrow action in plan".to_string()),
            };
            self.simulations.push(result.clone());
            return Ok(result);
        }

        // Sum borrow and repay amounts for the borrow token
        let mut total_borrowed: u64 = 0;
        let mut total_repaid: u64 = 0;
        for action in &plan.actions {
            match action {
                FlashAction::Borrow { amount, .. } => total_borrowed += amount,
                FlashAction::Repay { amount, .. } => total_repaid += amount,
                _ => {}
            }
        }

        let required = Self::calculate_required_repay(plan.borrow_amount, plan.fee_bps);
        let gas_used = (plan.actions.len() as u64) * 21_000;

        if total_repaid < required {
            let result = SimulationResult {
                plan_id: plan_id.to_string(),
                success: false,
                profit: total_repaid as i64 - required as i64,
                gas_used,
                steps_completed: plan.actions.len().saturating_sub(1),
                failure_step: Some(plan.actions.len().saturating_sub(1)),
                failure_reason: Some(format!(
                    "repay {} insufficient, need {}",
                    total_repaid, required
                )),
            };
            self.simulations.push(result.clone());
            return Ok(result);
        }

        let profit = total_repaid as i64 - required as i64;
        let risk = self.risk_assessment(plan_id)?;

        let result = SimulationResult {
            plan_id: plan_id.to_string(),
            success: true,
            profit,
            gas_used,
            steps_completed: plan.actions.len(),
            failure_step: None,
            failure_reason: None,
        };
        self.simulations.push(result.clone());

        // Update plan with simulation findings
        if let Some(p) = self.plans.get_mut(plan_id) {
            p.expected_profit = profit;
            p.gas_estimate = gas_used;
            p.risk_score = risk;
        }

        Ok(result)
    }

    /// Execute a draft plan (set status to Executed).
    pub fn execute(&mut self, plan_id: &str) -> Result<(), FlashLoanError> {
        let plan = self
            .plans
            .get_mut(plan_id)
            .ok_or_else(|| FlashLoanError::PlanNotFound(plan_id.to_string()))?;
        if plan.status != LoanStatus::Draft {
            return Err(FlashLoanError::InvalidState(
                plan_id.to_string(),
                plan.status.to_string(),
                "Draft".to_string(),
            ));
        }
        plan.status = LoanStatus::Executed;
        plan.executed_at = Some(chrono::Utc::now().to_rfc3339());
        Ok(())
    }

    /// Cancel a draft plan.
    pub fn cancel(&mut self, plan_id: &str) -> Result<(), FlashLoanError> {
        let plan = self
            .plans
            .get_mut(plan_id)
            .ok_or_else(|| FlashLoanError::PlanNotFound(plan_id.to_string()))?;
        if plan.status != LoanStatus::Draft {
            return Err(FlashLoanError::InvalidState(
                plan_id.to_string(),
                plan.status.to_string(),
                "Draft".to_string(),
            ));
        }
        plan.status = LoanStatus::Cancelled;
        Ok(())
    }

    /// Mark a plan as failed with a reason.
    pub fn fail_plan(&mut self, plan_id: &str, reason: &str) -> Result<(), FlashLoanError> {
        let plan = self
            .plans
            .get_mut(plan_id)
            .ok_or_else(|| FlashLoanError::PlanNotFound(plan_id.to_string()))?;
        plan.status = LoanStatus::Failed;
        plan.executed_at = Some(format!("failed: {}", reason));
        Ok(())
    }

    /// Calculate the total repayment required (borrow + fee).
    pub fn calculate_required_repay(borrow_amount: u64, fee_bps: u32) -> u64 {
        borrow_amount + (borrow_amount * fee_bps as u64) / 10_000
    }

    /// Calculate profit for a plan: total repay minus total borrow minus fees.
    pub fn calculate_profit(&self, plan_id: &str) -> Result<i64, FlashLoanError> {
        let plan = self
            .plans
            .get(plan_id)
            .ok_or_else(|| FlashLoanError::PlanNotFound(plan_id.to_string()))?;

        let mut total_borrowed: u64 = 0;
        let mut total_repaid: u64 = 0;
        for action in &plan.actions {
            match action {
                FlashAction::Borrow { amount, .. } => total_borrowed += amount,
                FlashAction::Repay { amount, .. } => total_repaid += amount,
                _ => {}
            }
        }

        let fee = (plan.borrow_amount * plan.fee_bps as u64) / 10_000;
        Ok(total_repaid as i64 - total_borrowed as i64 - fee as i64)
    }

    /// Assess risk of a plan on a 0-100 scale.
    ///
    /// Factors: action count, borrow size, number of distinct tokens.
    pub fn risk_assessment(&self, plan_id: &str) -> Result<u32, FlashLoanError> {
        let plan = self
            .plans
            .get(plan_id)
            .ok_or_else(|| FlashLoanError::PlanNotFound(plan_id.to_string()))?;

        let mut tokens = std::collections::HashSet::new();
        for action in &plan.actions {
            match action {
                FlashAction::Borrow { token, .. } => {
                    tokens.insert(token.clone());
                }
                FlashAction::Swap { from, to, .. } => {
                    tokens.insert(from.clone());
                    tokens.insert(to.clone());
                }
                FlashAction::Repay { token, .. } => {
                    tokens.insert(token.clone());
                }
                FlashAction::Arbitrage {
                    token_a, token_b, ..
                } => {
                    tokens.insert(token_a.clone());
                    tokens.insert(token_b.clone());
                }
                FlashAction::Liquidate {
                    collateral, debt, ..
                } => {
                    tokens.insert(collateral.clone());
                    tokens.insert(debt.clone());
                }
            }
        }

        // Action count risk: more actions = riskier (max 40 points)
        let action_risk = (plan.actions.len() as u32 * 8).min(40);
        // Borrow size risk: larger borrows = riskier (max 35 points)
        let borrow_risk = ((plan.borrow_amount / 1_000_000).min(35) as u32).min(35);
        // Token diversity risk: more tokens = riskier (max 25 points)
        let token_risk = (tokens.len() as u32 * 5).min(25);

        Ok((action_risk + borrow_risk + token_risk).min(100))
    }

    /// Return the most recent `n` simulation results.
    pub fn recent_simulations(&self, n: usize) -> Vec<&SimulationResult> {
        let len = self.simulations.len();
        let start = len.saturating_sub(n);
        self.simulations[start..].iter().collect()
    }

    /// Compute aggregate statistics across all plans.
    pub fn stats(&self) -> FlashLoanStats {
        let total_plans = self.plans.len();
        let mut executed = 0usize;
        let mut successful = 0usize;
        let mut failed = 0usize;
        let mut total_borrowed = 0u64;
        let mut total_profit = 0i64;
        let mut total_fees_paid = 0u64;
        let mut risk_sum = 0u64;

        for plan in self.plans.values() {
            match plan.status {
                LoanStatus::Executed => {
                    executed += 1;
                    successful += 1;
                    total_borrowed += plan.borrow_amount;
                    total_profit += plan.expected_profit;
                    total_fees_paid += (plan.borrow_amount * plan.fee_bps as u64) / 10_000;
                }
                LoanStatus::Failed => {
                    failed += 1;
                    total_borrowed += plan.borrow_amount;
                    total_fees_paid += (plan.borrow_amount * plan.fee_bps as u64) / 10_000;
                }
                _ => {}
            }
            risk_sum += plan.risk_score as u64;
        }

        let avg_risk_score = if total_plans > 0 {
            (risk_sum / total_plans as u64) as u32
        } else {
            0
        };

        FlashLoanStats {
            total_plans,
            executed,
            successful,
            failed,
            total_borrowed,
            total_profit,
            total_fees_paid,
            avg_risk_score,
        }
    }

    /// Load from a JSON file.
    pub fn load(path: &Path) -> Result<Self, FlashLoanError> {
        let data = std::fs::read_to_string(path)?;
        let mgr: Self = serde_json::from_str(&data)?;
        Ok(mgr)
    }

    /// Save to a JSON file.
    pub fn save(&self, path: &Path) -> Result<(), FlashLoanError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load from file, or return a default manager if it fails.
    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ──────────────────────────── Tests ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("flash_loan_test_{}.json", name))
    }

    #[test]
    fn test_create_plan() {
        let mut mgr = FlashLoanManager::new();
        let id = mgr.create_plan("arb-eth-usdc", "ETH", 1_000_000, 30);
        assert!(!id.is_empty());
        let plan = mgr.get_plan(&id).unwrap();
        assert_eq!(plan.name, "arb-eth-usdc");
        assert_eq!(plan.borrow_token, "ETH");
        assert_eq!(plan.borrow_amount, 1_000_000);
        assert_eq!(plan.fee_bps, 30);
        assert_eq!(plan.status, LoanStatus::Draft);
        assert!(plan.executed_at.is_none());
    }

    #[test]
    fn test_add_action() {
        let mut mgr = FlashLoanManager::new();
        let id = mgr.create_plan("test", "ETH", 100, 10);
        mgr.add_action(
            &id,
            FlashAction::Borrow {
                token: "ETH".into(),
                amount: 100,
            },
        )
        .unwrap();
        mgr.add_action(
            &id,
            FlashAction::Swap {
                from: "ETH".into(),
                to: "USDC".into(),
                amount: 100,
            },
        )
        .unwrap();
        assert_eq!(mgr.get_plan(&id).unwrap().actions.len(), 2);
    }

    #[test]
    fn test_add_action_not_draft() {
        let mut mgr = FlashLoanManager::new();
        let id = mgr.create_plan("test", "ETH", 100, 10);
        mgr.execute(&id).unwrap();
        let res = mgr.add_action(
            &id,
            FlashAction::Borrow {
                token: "ETH".into(),
                amount: 100,
            },
        );
        assert!(res.is_err());
    }

    #[test]
    fn test_add_action_plan_not_found() {
        let mut mgr = FlashLoanManager::new();
        let res = mgr.add_action(
            "nonexistent",
            FlashAction::Borrow {
                token: "ETH".into(),
                amount: 1,
            },
        );
        assert!(res.is_err());
    }

    #[test]
    fn test_remove_action() {
        let mut mgr = FlashLoanManager::new();
        let id = mgr.create_plan("test", "ETH", 100, 10);
        mgr.add_action(
            &id,
            FlashAction::Borrow {
                token: "ETH".into(),
                amount: 100,
            },
        )
        .unwrap();
        mgr.add_action(
            &id,
            FlashAction::Repay {
                token: "ETH".into(),
                amount: 100,
            },
        )
        .unwrap();
        let removed = mgr.remove_action(&id, 0).unwrap();
        assert!(matches!(removed, FlashAction::Borrow { .. }));
        assert_eq!(mgr.get_plan(&id).unwrap().actions.len(), 1);
    }

    #[test]
    fn test_remove_action_out_of_bounds() {
        let mut mgr = FlashLoanManager::new();
        let id = mgr.create_plan("test", "ETH", 100, 10);
        let res = mgr.remove_action(&id, 5);
        assert!(res.is_err());
    }

    #[test]
    fn test_simulate_success() {
        let mut mgr = FlashLoanManager::new();
        let id = mgr.create_plan("arb", "ETH", 1000, 30); // fee = 3
        mgr.add_action(
            &id,
            FlashAction::Borrow {
                token: "ETH".into(),
                amount: 1000,
            },
        )
        .unwrap();
        mgr.add_action(
            &id,
            FlashAction::Swap {
                from: "ETH".into(),
                to: "USDC".into(),
                amount: 1000,
            },
        )
        .unwrap();
        mgr.add_action(
            &id,
            FlashAction::Repay {
                token: "ETH".into(),
                amount: 1050,
            },
        )
        .unwrap();
        let result = mgr.simulate(&id).unwrap();
        assert!(result.success);
        assert!(result.profit > 0);
        assert_eq!(result.steps_completed, 3);
        assert!(result.failure_step.is_none());
    }

    #[test]
    fn test_simulate_no_borrow() {
        let mut mgr = FlashLoanManager::new();
        let id = mgr.create_plan("bad", "ETH", 1000, 30);
        mgr.add_action(
            &id,
            FlashAction::Repay {
                token: "ETH".into(),
                amount: 1000,
            },
        )
        .unwrap();
        let result = mgr.simulate(&id).unwrap();
        assert!(!result.success);
        assert!(result
            .failure_reason
            .as_ref()
            .unwrap()
            .contains("no borrow"));
    }

    #[test]
    fn test_simulate_insufficient_repay() {
        let mut mgr = FlashLoanManager::new();
        let id = mgr.create_plan("short", "ETH", 1000, 30);
        mgr.add_action(
            &id,
            FlashAction::Borrow {
                token: "ETH".into(),
                amount: 1000,
            },
        )
        .unwrap();
        mgr.add_action(
            &id,
            FlashAction::Repay {
                token: "ETH".into(),
                amount: 500,
            },
        )
        .unwrap();
        let result = mgr.simulate(&id).unwrap();
        assert!(!result.success);
        assert!(result
            .failure_reason
            .as_ref()
            .unwrap()
            .contains("insufficient"));
    }

    #[test]
    fn test_execute() {
        let mut mgr = FlashLoanManager::new();
        let id = mgr.create_plan("exec", "ETH", 1000, 10);
        mgr.execute(&id).unwrap();
        let plan = mgr.get_plan(&id).unwrap();
        assert_eq!(plan.status, LoanStatus::Executed);
        assert!(plan.executed_at.is_some());
    }

    #[test]
    fn test_execute_not_draft() {
        let mut mgr = FlashLoanManager::new();
        let id = mgr.create_plan("exec", "ETH", 1000, 10);
        mgr.cancel(&id).unwrap();
        let res = mgr.execute(&id);
        assert!(res.is_err());
    }

    #[test]
    fn test_cancel() {
        let mut mgr = FlashLoanManager::new();
        let id = mgr.create_plan("cancel-me", "ETH", 1000, 10);
        mgr.cancel(&id).unwrap();
        assert_eq!(mgr.get_plan(&id).unwrap().status, LoanStatus::Cancelled);
    }

    #[test]
    fn test_cancel_not_draft() {
        let mut mgr = FlashLoanManager::new();
        let id = mgr.create_plan("cancel", "ETH", 1000, 10);
        mgr.execute(&id).unwrap();
        let res = mgr.cancel(&id);
        assert!(res.is_err());
    }

    #[test]
    fn test_fail_plan() {
        let mut mgr = FlashLoanManager::new();
        let id = mgr.create_plan("fail", "ETH", 1000, 10);
        mgr.fail_plan(&id, "revert at step 2").unwrap();
        assert_eq!(mgr.get_plan(&id).unwrap().status, LoanStatus::Failed);
    }

    #[test]
    fn test_calculate_required_repay() {
        // 1000 borrowed at 30 bps = 1000 + 3 = 1003
        assert_eq!(FlashLoanManager::calculate_required_repay(1000, 30), 1003);
        // 10000 at 100 bps = 10000 + 100 = 10100
        assert_eq!(
            FlashLoanManager::calculate_required_repay(10_000, 100),
            10_100
        );
        // Zero fee
        assert_eq!(FlashLoanManager::calculate_required_repay(500, 0), 500);
    }

    #[test]
    fn test_calculate_profit() {
        let mut mgr = FlashLoanManager::new();
        let id = mgr.create_plan("profit", "ETH", 1000, 30);
        mgr.add_action(
            &id,
            FlashAction::Borrow {
                token: "ETH".into(),
                amount: 1000,
            },
        )
        .unwrap();
        mgr.add_action(
            &id,
            FlashAction::Repay {
                token: "ETH".into(),
                amount: 1100,
            },
        )
        .unwrap();
        // profit = repaid(1100) - borrowed(1000) - fee(3) = 97
        let profit = mgr.calculate_profit(&id).unwrap();
        assert_eq!(profit, 97);
    }

    #[test]
    fn test_risk_assessment() {
        let mut mgr = FlashLoanManager::new();
        let id = mgr.create_plan("risky", "ETH", 50_000_000, 30);
        mgr.add_action(
            &id,
            FlashAction::Borrow {
                token: "ETH".into(),
                amount: 50_000_000,
            },
        )
        .unwrap();
        mgr.add_action(
            &id,
            FlashAction::Swap {
                from: "ETH".into(),
                to: "USDC".into(),
                amount: 25_000_000,
            },
        )
        .unwrap();
        mgr.add_action(
            &id,
            FlashAction::Arbitrage {
                token_a: "USDC".into(),
                token_b: "DAI".into(),
                amount: 25_000_000,
            },
        )
        .unwrap();
        mgr.add_action(
            &id,
            FlashAction::Repay {
                token: "ETH".into(),
                amount: 51_000_000,
            },
        )
        .unwrap();

        let risk = mgr.risk_assessment(&id).unwrap();
        assert!(risk > 0 && risk <= 100);
    }

    #[test]
    fn test_recent_simulations() {
        let mut mgr = FlashLoanManager::new();
        // Create and simulate 3 plans
        for i in 0..3 {
            let id = mgr.create_plan(&format!("plan{}", i), "ETH", 1000, 10);
            mgr.add_action(
                &id,
                FlashAction::Borrow {
                    token: "ETH".into(),
                    amount: 1000,
                },
            )
            .unwrap();
            mgr.add_action(
                &id,
                FlashAction::Repay {
                    token: "ETH".into(),
                    amount: 1100,
                },
            )
            .unwrap();
            mgr.simulate(&id).unwrap();
        }
        let recent = mgr.recent_simulations(2);
        assert_eq!(recent.len(), 2);
    }

    #[test]
    fn test_stats() {
        let mut mgr = FlashLoanManager::new();
        let id1 = mgr.create_plan("exec1", "ETH", 1000, 30);
        mgr.execute(&id1).unwrap();
        let id2 = mgr.create_plan("fail1", "ETH", 2000, 50);
        mgr.fail_plan(&id2, "reverted").unwrap();
        let _id3 = mgr.create_plan("draft1", "ETH", 500, 10);

        let s = mgr.stats();
        assert_eq!(s.total_plans, 3);
        assert_eq!(s.executed, 1);
        assert_eq!(s.successful, 1);
        assert_eq!(s.failed, 1);
        assert_eq!(s.total_borrowed, 3000); // 1000 + 2000
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path("roundtrip");
        let mut mgr = FlashLoanManager::new();
        let id = mgr.create_plan("persist", "ETH", 5000, 25);
        mgr.add_action(
            &id,
            FlashAction::Borrow {
                token: "ETH".into(),
                amount: 5000,
            },
        )
        .unwrap();
        mgr.save(&path).unwrap();

        let loaded = FlashLoanManager::load(&path).unwrap();
        assert_eq!(loaded.plans.len(), 1);
        let plan = loaded.get_plan(&id).unwrap();
        assert_eq!(plan.name, "persist");
        assert_eq!(plan.borrow_amount, 5000);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let missing = test_path("nonexistent_flash");
        let mgr = FlashLoanManager::load_or_default(&missing);
        assert!(mgr.plans.is_empty());
        assert!(mgr.simulations.is_empty());
    }

    #[test]
    fn test_list_plans() {
        let mut mgr = FlashLoanManager::new();
        mgr.create_plan("a", "ETH", 100, 10);
        mgr.create_plan("b", "USDC", 200, 20);
        let plans = mgr.list_plans();
        assert_eq!(plans.len(), 2);
    }
}
