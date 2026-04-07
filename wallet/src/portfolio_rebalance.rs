// wallet/src/portfolio_rebalance.rs — Portfolio rebalancing engine
//
// Track multi-token portfolios, compute allocation drift,
// generate rebalance plans, and execute trades to restore targets.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ── Errors ───────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum RebalanceError {
    #[error("portfolio already exists: {0}")]
    PortfolioAlreadyExists(String),
    #[error("portfolio not found: {0}")]
    PortfolioNotFound(String),
    #[error("plan not found: {0}")]
    PlanNotFound(String),
    #[error("plan not in planned status: {0}")]
    PlanNotPlanned(String),
    #[error("invalid targets: must sum to 100%, got {0}%")]
    InvalidTargets(f64),
    #[error("empty portfolio holdings: {0}")]
    EmptyHoldings(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
}

// ── Enums ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RebalanceStrategy {
    Threshold(f64),
    Periodic,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RebalanceStatus {
    Planned,
    Executing,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeAction {
    Buy,
    Sell,
}

// ── Structs ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetAllocation {
    pub token: String,
    pub target_pct: f64,
    pub current_pct: f64,
    pub current_value: u64,
    pub drift: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebalancePlan {
    pub id: String,
    pub portfolio_id: String,
    pub trades: Vec<RebalanceTrade>,
    pub status: RebalanceStatus,
    pub created_at: String,
    pub executed_at: Option<String>,
    pub total_buy_value: u64,
    pub total_sell_value: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebalanceTrade {
    pub token: String,
    pub action: TradeAction,
    pub amount: u64,
    pub value: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Portfolio {
    pub id: String,
    pub name: String,
    pub targets: HashMap<String, f64>,
    pub holdings: HashMap<String, u64>,
    pub strategy: RebalanceStrategy,
    pub threshold_pct: f64,
    pub last_rebalance: Option<String>,
    pub rebalance_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebalanceStats {
    pub total_portfolios: usize,
    pub total_rebalances: usize,
    pub completed_rebalances: usize,
    pub total_trade_volume: u64,
    pub avg_drift: f64,
}

// ── Manager ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RebalanceManager {
    pub portfolios: HashMap<String, Portfolio>,
    pub plans: HashMap<String, RebalancePlan>,
}

impl RebalanceManager {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Portfolio CRUD ───────────────────────────────────────

    pub fn create_portfolio(&mut self, portfolio: Portfolio) -> Result<(), RebalanceError> {
        if self.portfolios.contains_key(&portfolio.id) {
            return Err(RebalanceError::PortfolioAlreadyExists(portfolio.id));
        }
        let sum: f64 = portfolio.targets.values().sum();
        if (sum - 100.0).abs() > 0.01 {
            return Err(RebalanceError::InvalidTargets(sum));
        }
        self.portfolios.insert(portfolio.id.clone(), portfolio);
        Ok(())
    }

    pub fn remove_portfolio(&mut self, id: &str) -> Result<Portfolio, RebalanceError> {
        self.portfolios
            .remove(id)
            .ok_or_else(|| RebalanceError::PortfolioNotFound(id.to_string()))
    }

    pub fn get_portfolio(&self, id: &str) -> Option<&Portfolio> {
        self.portfolios.get(id)
    }

    pub fn list_portfolios(&self) -> Vec<&Portfolio> {
        self.portfolios.values().collect()
    }

    // ── Holdings ─────────────────────────────────────────────

    pub fn update_holdings(
        &mut self,
        portfolio_id: &str,
        holdings: HashMap<String, u64>,
    ) -> Result<(), RebalanceError> {
        let portfolio = self
            .portfolios
            .get_mut(portfolio_id)
            .ok_or_else(|| RebalanceError::PortfolioNotFound(portfolio_id.to_string()))?;
        portfolio.holdings = holdings;
        Ok(())
    }

    // ── Allocation analysis ──────────────────────────────────

    pub fn calculate_allocations(
        &self,
        portfolio_id: &str,
    ) -> Result<Vec<TargetAllocation>, RebalanceError> {
        let portfolio = self
            .portfolios
            .get(portfolio_id)
            .ok_or_else(|| RebalanceError::PortfolioNotFound(portfolio_id.to_string()))?;

        let total_value: u64 = portfolio.holdings.values().sum();
        if total_value == 0 {
            return Err(RebalanceError::EmptyHoldings(portfolio_id.to_string()));
        }

        let mut allocations = Vec::new();
        for (token, &target_pct) in &portfolio.targets {
            let current_value = portfolio.holdings.get(token).copied().unwrap_or(0);
            let current_pct = (current_value as f64 / total_value as f64) * 100.0;
            let drift = current_pct - target_pct;
            allocations.push(TargetAllocation {
                token: token.clone(),
                target_pct,
                current_pct,
                current_value,
                drift,
            });
        }
        allocations.sort_by(|a, b| a.token.cmp(&b.token));
        Ok(allocations)
    }

    pub fn check_drift(&self, portfolio_id: &str) -> Result<f64, RebalanceError> {
        let allocations = self.calculate_allocations(portfolio_id)?;
        let max_drift = allocations
            .iter()
            .map(|a| a.drift.abs())
            .fold(0.0_f64, f64::max);
        Ok(max_drift)
    }

    pub fn needs_rebalance(&self, portfolio_id: &str) -> Result<bool, RebalanceError> {
        let portfolio = self
            .portfolios
            .get(portfolio_id)
            .ok_or_else(|| RebalanceError::PortfolioNotFound(portfolio_id.to_string()))?;
        let max_drift = self.check_drift(portfolio_id)?;
        Ok(max_drift > portfolio.threshold_pct)
    }

    // ── Plan generation & execution ──────────────────────────

    pub fn generate_plan(&mut self, portfolio_id: &str) -> Result<String, RebalanceError> {
        let allocations = self.calculate_allocations(portfolio_id)?;
        let total_value: u64 = self
            .portfolios
            .get(portfolio_id)
            .unwrap()
            .holdings
            .values()
            .sum();

        let mut trades = Vec::new();
        let mut total_buy_value: u64 = 0;
        let mut total_sell_value: u64 = 0;

        for alloc in &allocations {
            let target_value = ((alloc.target_pct / 100.0) * total_value as f64) as u64;
            if alloc.current_value < target_value {
                let diff = target_value - alloc.current_value;
                total_buy_value += diff;
                trades.push(RebalanceTrade {
                    token: alloc.token.clone(),
                    action: TradeAction::Buy,
                    amount: diff,
                    value: diff,
                });
            } else if alloc.current_value > target_value {
                let diff = alloc.current_value - target_value;
                total_sell_value += diff;
                trades.push(RebalanceTrade {
                    token: alloc.token.clone(),
                    action: TradeAction::Sell,
                    amount: diff,
                    value: diff,
                });
            }
        }

        let plan_id = format!("plan-{}", self.plans.len() + 1);
        let plan = RebalancePlan {
            id: plan_id.clone(),
            portfolio_id: portfolio_id.to_string(),
            trades,
            status: RebalanceStatus::Planned,
            created_at: chrono::Utc::now().to_rfc3339(),
            executed_at: None,
            total_buy_value,
            total_sell_value,
        };
        self.plans.insert(plan_id.clone(), plan);
        Ok(plan_id)
    }

    pub fn execute_plan(&mut self, plan_id: &str) -> Result<(), RebalanceError> {
        let plan = self
            .plans
            .get(plan_id)
            .ok_or_else(|| RebalanceError::PlanNotFound(plan_id.to_string()))?;
        if plan.status != RebalanceStatus::Planned {
            return Err(RebalanceError::PlanNotPlanned(plan_id.to_string()));
        }
        let portfolio_id = plan.portfolio_id.clone();

        // Apply trades to holdings
        let plan = self.plans.get(plan_id).unwrap();
        let trades: Vec<RebalanceTrade> = plan.trades.clone();

        let portfolio = self
            .portfolios
            .get_mut(&portfolio_id)
            .ok_or_else(|| RebalanceError::PortfolioNotFound(portfolio_id.clone()))?;

        for trade in &trades {
            let holding = portfolio.holdings.entry(trade.token.clone()).or_insert(0);
            match trade.action {
                TradeAction::Buy => *holding += trade.amount,
                TradeAction::Sell => *holding = holding.saturating_sub(trade.amount),
            }
        }
        portfolio.last_rebalance = Some(chrono::Utc::now().to_rfc3339());
        portfolio.rebalance_count += 1;

        let plan = self.plans.get_mut(plan_id).unwrap();
        plan.status = RebalanceStatus::Completed;
        plan.executed_at = Some(chrono::Utc::now().to_rfc3339());

        Ok(())
    }

    pub fn get_plan(&self, id: &str) -> Option<&RebalancePlan> {
        self.plans.get(id)
    }

    pub fn plan_history(&self, portfolio_id: &str) -> Vec<&RebalancePlan> {
        self.plans
            .values()
            .filter(|p| p.portfolio_id == portfolio_id)
            .collect()
    }

    // ── Stats ────────────────────────────────────────────────

    pub fn stats(&self) -> RebalanceStats {
        let total_portfolios = self.portfolios.len();
        let total_rebalances = self.plans.len();
        let completed_rebalances = self
            .plans
            .values()
            .filter(|p| p.status == RebalanceStatus::Completed)
            .count();
        let total_trade_volume: u64 = self
            .plans
            .values()
            .map(|p| p.total_buy_value + p.total_sell_value)
            .sum();

        let drift_sum: f64 = self
            .portfolios
            .keys()
            .filter_map(|id| self.check_drift(id).ok())
            .sum();
        let avg_drift = if total_portfolios > 0 {
            drift_sum / total_portfolios as f64
        } else {
            0.0
        };

        RebalanceStats {
            total_portfolios,
            total_rebalances,
            completed_rebalances,
            total_trade_volume,
            avg_drift,
        }
    }

    // ── Persistence ──────────────────────────────────────────

    pub fn save(&self, path: &Path) -> Result<(), RebalanceError> {
        let json =
            serde_json::to_string_pretty(self).map_err(|e| RebalanceError::Json(e.to_string()))?;
        std::fs::write(path, json).map_err(|e| RebalanceError::Io(e.to_string()))
    }

    pub fn load(path: &Path) -> Result<Self, RebalanceError> {
        let data =
            std::fs::read_to_string(path).map_err(|e| RebalanceError::Io(e.to_string()))?;
        serde_json::from_str(&data).map_err(|e| RebalanceError::Json(e.to_string()))
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_portfolio(id: &str, threshold: f64) -> Portfolio {
        let mut targets = HashMap::new();
        targets.insert("BTC".to_string(), 50.0);
        targets.insert("ETH".to_string(), 30.0);
        targets.insert("EVAP".to_string(), 20.0);

        let mut holdings = HashMap::new();
        holdings.insert("BTC".to_string(), 5000);
        holdings.insert("ETH".to_string(), 3000);
        holdings.insert("EVAP".to_string(), 2000);

        Portfolio {
            id: id.to_string(),
            name: format!("Portfolio {}", id),
            targets,
            holdings,
            strategy: RebalanceStrategy::Threshold(threshold),
            threshold_pct: threshold,
            last_rebalance: None,
            rebalance_count: 0,
        }
    }

    fn make_drifted_portfolio(id: &str) -> Portfolio {
        let mut targets = HashMap::new();
        targets.insert("BTC".to_string(), 50.0);
        targets.insert("ETH".to_string(), 30.0);
        targets.insert("EVAP".to_string(), 20.0);

        let mut holdings = HashMap::new();
        holdings.insert("BTC".to_string(), 7000); // 70% — drift +20
        holdings.insert("ETH".to_string(), 2000); // 20% — drift -10
        holdings.insert("EVAP".to_string(), 1000); // 10% — drift -10

        Portfolio {
            id: id.to_string(),
            name: "Drifted".to_string(),
            targets,
            holdings,
            strategy: RebalanceStrategy::Threshold(5.0),
            threshold_pct: 5.0,
            last_rebalance: None,
            rebalance_count: 0,
        }
    }

    // ── Portfolio CRUD ───────────────────────────────────────

    #[test]
    fn test_create_portfolio() {
        let mut mgr = RebalanceManager::new();
        assert!(mgr.create_portfolio(make_portfolio("p1", 5.0)).is_ok());
        assert!(mgr.get_portfolio("p1").is_some());
    }

    #[test]
    fn test_create_portfolio_duplicate() {
        let mut mgr = RebalanceManager::new();
        mgr.create_portfolio(make_portfolio("p1", 5.0)).unwrap();
        let result = mgr.create_portfolio(make_portfolio("p1", 5.0));
        assert!(matches!(
            result.unwrap_err(),
            RebalanceError::PortfolioAlreadyExists(_)
        ));
    }

    #[test]
    fn test_create_portfolio_invalid_targets() {
        let mut mgr = RebalanceManager::new();
        let mut p = make_portfolio("p1", 5.0);
        p.targets.insert("BTC".to_string(), 60.0); // now sums to 110
        let result = mgr.create_portfolio(p);
        assert!(matches!(
            result.unwrap_err(),
            RebalanceError::InvalidTargets(_)
        ));
    }

    #[test]
    fn test_remove_portfolio() {
        let mut mgr = RebalanceManager::new();
        mgr.create_portfolio(make_portfolio("p1", 5.0)).unwrap();
        let removed = mgr.remove_portfolio("p1").unwrap();
        assert_eq!(removed.id, "p1");
        assert!(mgr.get_portfolio("p1").is_none());
    }

    #[test]
    fn test_remove_portfolio_not_found() {
        let mut mgr = RebalanceManager::new();
        assert!(matches!(
            mgr.remove_portfolio("nope").unwrap_err(),
            RebalanceError::PortfolioNotFound(_)
        ));
    }

    #[test]
    fn test_list_portfolios() {
        let mut mgr = RebalanceManager::new();
        mgr.create_portfolio(make_portfolio("p1", 5.0)).unwrap();
        mgr.create_portfolio(make_portfolio("p2", 3.0)).unwrap();
        assert_eq!(mgr.list_portfolios().len(), 2);
    }

    // ── Holdings ─────────────────────────────────────────────

    #[test]
    fn test_update_holdings() {
        let mut mgr = RebalanceManager::new();
        mgr.create_portfolio(make_portfolio("p1", 5.0)).unwrap();
        let mut new_holdings = HashMap::new();
        new_holdings.insert("BTC".to_string(), 8000);
        new_holdings.insert("ETH".to_string(), 1500);
        new_holdings.insert("EVAP".to_string(), 500);
        mgr.update_holdings("p1", new_holdings).unwrap();
        let p = mgr.get_portfolio("p1").unwrap();
        assert_eq!(p.holdings.get("BTC"), Some(&8000));
    }

    #[test]
    fn test_update_holdings_not_found() {
        let mut mgr = RebalanceManager::new();
        let result = mgr.update_holdings("nope", HashMap::new());
        assert!(matches!(
            result.unwrap_err(),
            RebalanceError::PortfolioNotFound(_)
        ));
    }

    // ── Allocation analysis ──────────────────────────────────

    #[test]
    fn test_calculate_allocations_balanced() {
        let mut mgr = RebalanceManager::new();
        mgr.create_portfolio(make_portfolio("p1", 5.0)).unwrap();
        let allocs = mgr.calculate_allocations("p1").unwrap();
        assert_eq!(allocs.len(), 3);
        for a in &allocs {
            assert!(a.drift.abs() < 0.01);
        }
    }

    #[test]
    fn test_calculate_allocations_drifted() {
        let mut mgr = RebalanceManager::new();
        mgr.create_portfolio(make_drifted_portfolio("p1")).unwrap();
        let allocs = mgr.calculate_allocations("p1").unwrap();
        let btc = allocs.iter().find(|a| a.token == "BTC").unwrap();
        assert!((btc.drift - 20.0).abs() < 0.01);
    }

    #[test]
    fn test_calculate_allocations_not_found() {
        let mgr = RebalanceManager::new();
        assert!(matches!(
            mgr.calculate_allocations("nope").unwrap_err(),
            RebalanceError::PortfolioNotFound(_)
        ));
    }

    #[test]
    fn test_calculate_allocations_empty_holdings() {
        let mut mgr = RebalanceManager::new();
        let mut p = make_portfolio("p1", 5.0);
        p.holdings = HashMap::new();
        mgr.create_portfolio(p).unwrap();
        assert!(matches!(
            mgr.calculate_allocations("p1").unwrap_err(),
            RebalanceError::EmptyHoldings(_)
        ));
    }

    #[test]
    fn test_check_drift() {
        let mut mgr = RebalanceManager::new();
        mgr.create_portfolio(make_drifted_portfolio("p1")).unwrap();
        let drift = mgr.check_drift("p1").unwrap();
        assert!((drift - 20.0).abs() < 0.01);
    }

    #[test]
    fn test_needs_rebalance_true() {
        let mut mgr = RebalanceManager::new();
        mgr.create_portfolio(make_drifted_portfolio("p1")).unwrap();
        assert!(mgr.needs_rebalance("p1").unwrap());
    }

    #[test]
    fn test_needs_rebalance_false() {
        let mut mgr = RebalanceManager::new();
        mgr.create_portfolio(make_portfolio("p1", 5.0)).unwrap();
        assert!(!mgr.needs_rebalance("p1").unwrap());
    }

    // ── Plan generation & execution ──────────────────────────

    #[test]
    fn test_generate_plan() {
        let mut mgr = RebalanceManager::new();
        mgr.create_portfolio(make_drifted_portfolio("p1")).unwrap();
        let plan_id = mgr.generate_plan("p1").unwrap();
        let plan = mgr.get_plan(&plan_id).unwrap();
        assert_eq!(plan.status, RebalanceStatus::Planned);
        assert!(!plan.trades.is_empty());
        assert!(plan.total_sell_value > 0 || plan.total_buy_value > 0);
    }

    #[test]
    fn test_execute_plan() {
        let mut mgr = RebalanceManager::new();
        mgr.create_portfolio(make_drifted_portfolio("p1")).unwrap();
        let plan_id = mgr.generate_plan("p1").unwrap();
        mgr.execute_plan(&plan_id).unwrap();

        let plan = mgr.get_plan(&plan_id).unwrap();
        assert_eq!(plan.status, RebalanceStatus::Completed);
        assert!(plan.executed_at.is_some());

        let portfolio = mgr.get_portfolio("p1").unwrap();
        assert!(portfolio.last_rebalance.is_some());
        assert_eq!(portfolio.rebalance_count, 1);
    }

    #[test]
    fn test_execute_plan_not_found() {
        let mut mgr = RebalanceManager::new();
        assert!(matches!(
            mgr.execute_plan("nope").unwrap_err(),
            RebalanceError::PlanNotFound(_)
        ));
    }

    #[test]
    fn test_execute_plan_already_completed() {
        let mut mgr = RebalanceManager::new();
        mgr.create_portfolio(make_drifted_portfolio("p1")).unwrap();
        let plan_id = mgr.generate_plan("p1").unwrap();
        mgr.execute_plan(&plan_id).unwrap();
        let result = mgr.execute_plan(&plan_id);
        assert!(matches!(
            result.unwrap_err(),
            RebalanceError::PlanNotPlanned(_)
        ));
    }

    #[test]
    fn test_plan_history() {
        let mut mgr = RebalanceManager::new();
        mgr.create_portfolio(make_drifted_portfolio("p1")).unwrap();
        mgr.generate_plan("p1").unwrap();
        mgr.generate_plan("p1").unwrap();
        let history = mgr.plan_history("p1");
        assert_eq!(history.len(), 2);
    }

    // ── Stats ────────────────────────────────────────────────

    #[test]
    fn test_stats() {
        let mut mgr = RebalanceManager::new();
        mgr.create_portfolio(make_drifted_portfolio("p1")).unwrap();
        let plan_id = mgr.generate_plan("p1").unwrap();
        mgr.execute_plan(&plan_id).unwrap();

        let stats = mgr.stats();
        assert_eq!(stats.total_portfolios, 1);
        assert_eq!(stats.total_rebalances, 1);
        assert_eq!(stats.completed_rebalances, 1);
        assert!(stats.total_trade_volume > 0);
    }

    // ── Persistence ──────────────────────────────────────────

    #[test]
    fn test_save_and_load() {
        let mut mgr = RebalanceManager::new();
        mgr.create_portfolio(make_portfolio("p1", 5.0)).unwrap();

        let dir = std::env::temp_dir();
        let path = dir.join(format!("rebalance_data_{}.json", std::process::id()));

        mgr.save(&path).unwrap();
        let loaded = RebalanceManager::load(&path).unwrap();
        assert_eq!(loaded.portfolios.len(), 1);
        assert!(loaded.get_portfolio("p1").is_some());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("rebalance_missing_{}.json", std::process::id()));
        let mgr = RebalanceManager::load_or_default(&path);
        assert_eq!(mgr.portfolios.len(), 0);
    }
}
