use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum DcaError {
    #[error("Plan already exists: {0}")]
    PlanAlreadyExists(String),
    #[error("Plan not found: {0}")]
    PlanNotFound(String),
    #[error("Plan not active: {0}")]
    PlanNotActive(String),
    #[error("Plan not paused: {0}")]
    PlanNotPaused(String),
    #[error("Price out of bounds: {0}")]
    PriceOutOfBounds(f64),
    #[error("Budget exhausted for plan: {0}")]
    BudgetExhausted(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DcaFrequency {
    Hourly,
    Daily,
    Weekly,
    Monthly,
    Custom(u64),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DcaStatus {
    Active,
    Paused,
    Completed,
    Cancelled,
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DcaPlan {
    pub id: String,
    pub name: String,
    pub token_from: String,
    pub token_to: String,
    pub amount_per_buy: u64,
    pub frequency: DcaFrequency,
    pub total_budget: Option<u64>,
    pub spent: u64,
    pub buys_completed: u32,
    pub max_buys: Option<u32>,
    pub status: DcaStatus,
    pub created_at: String,
    pub last_buy: Option<String>,
    pub next_buy: Option<String>,
    pub min_price: Option<f64>,
    pub max_price: Option<f64>,
    pub total_received: u64,
    pub avg_price: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DcaExecution {
    pub plan_id: String,
    pub timestamp: String,
    pub amount_spent: u64,
    pub amount_received: u64,
    pub price: f64,
    pub tx_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DcaStats {
    pub total_plans: usize,
    pub active_plans: usize,
    pub paused_plans: usize,
    pub completed_plans: usize,
    pub total_spent: u64,
    pub total_received: u64,
    pub total_buys: u32,
    pub avg_price_all: f64,
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DcaEngine {
    pub plans: HashMap<String, DcaPlan>,
    pub executions: Vec<DcaExecution>,
}

impl DcaEngine {
    pub fn new() -> Self {
        Self::default()
    }

    // -- Plan CRUD ----------------------------------------------------------

    pub fn create_plan(&mut self, plan: DcaPlan) -> Result<(), DcaError> {
        if self.plans.contains_key(&plan.id) {
            return Err(DcaError::PlanAlreadyExists(plan.id));
        }
        self.plans.insert(plan.id.clone(), plan);
        Ok(())
    }

    pub fn remove_plan(&mut self, id: &str) -> Result<DcaPlan, DcaError> {
        self.plans
            .remove(id)
            .ok_or_else(|| DcaError::PlanNotFound(id.to_string()))
    }

    pub fn get_plan(&self, id: &str) -> Option<&DcaPlan> {
        self.plans.get(id)
    }

    pub fn get_plan_mut(&mut self, id: &str) -> Option<&mut DcaPlan> {
        self.plans.get_mut(id)
    }

    // -- Status transitions -------------------------------------------------

    pub fn pause_plan(&mut self, id: &str) -> Result<(), DcaError> {
        let plan = self
            .plans
            .get_mut(id)
            .ok_or_else(|| DcaError::PlanNotFound(id.to_string()))?;
        if plan.status != DcaStatus::Active {
            return Err(DcaError::PlanNotActive(id.to_string()));
        }
        plan.status = DcaStatus::Paused;
        Ok(())
    }

    pub fn resume_plan(&mut self, id: &str) -> Result<(), DcaError> {
        let plan = self
            .plans
            .get_mut(id)
            .ok_or_else(|| DcaError::PlanNotFound(id.to_string()))?;
        if plan.status != DcaStatus::Paused {
            return Err(DcaError::PlanNotPaused(id.to_string()));
        }
        plan.status = DcaStatus::Active;
        Ok(())
    }

    pub fn cancel_plan(&mut self, id: &str) -> Result<(), DcaError> {
        let plan = self
            .plans
            .get_mut(id)
            .ok_or_else(|| DcaError::PlanNotFound(id.to_string()))?;
        plan.status = DcaStatus::Cancelled;
        Ok(())
    }

    // -- Execution ----------------------------------------------------------

    pub fn execute_buy(
        &mut self,
        plan_id: &str,
        price: f64,
        received: u64,
        tx_hash: Option<String>,
    ) -> Result<&DcaExecution, DcaError> {
        // Validate plan exists and is active
        let plan = self
            .plans
            .get(plan_id)
            .ok_or_else(|| DcaError::PlanNotFound(plan_id.to_string()))?;

        if plan.status != DcaStatus::Active {
            return Err(DcaError::PlanNotActive(plan_id.to_string()));
        }

        // Check price bounds
        if let Some(min) = plan.min_price {
            if price < min {
                return Err(DcaError::PriceOutOfBounds(price));
            }
        }
        if let Some(max) = plan.max_price {
            if price > max {
                return Err(DcaError::PriceOutOfBounds(price));
            }
        }

        let amount_spent = plan.amount_per_buy;

        // Check budget
        if let Some(budget) = plan.total_budget {
            if plan.spent + amount_spent > budget {
                return Err(DcaError::BudgetExhausted(plan_id.to_string()));
            }
        }

        let now = chrono::Utc::now().to_rfc3339();

        // Update plan
        let plan = self.plans.get_mut(plan_id).unwrap();
        plan.spent += amount_spent;
        plan.buys_completed += 1;
        plan.total_received += received;
        plan.last_buy = Some(now.clone());

        // Update average price: weighted average
        let total_spent_f = plan.spent as f64;
        let total_received_f = plan.total_received as f64;
        plan.avg_price = if total_received_f > 0.0 {
            total_spent_f / total_received_f
        } else {
            0.0
        };

        // Compute next_buy based on frequency
        let next = compute_next_buy(&now, &plan.frequency);
        plan.next_buy = Some(next);

        // Auto-complete if limits reached
        if let Some(max_buys) = plan.max_buys {
            if plan.buys_completed >= max_buys {
                plan.status = DcaStatus::Completed;
            }
        }
        if let Some(budget) = plan.total_budget {
            if plan.spent >= budget {
                plan.status = DcaStatus::Completed;
            }
        }

        // Record execution
        let execution = DcaExecution {
            plan_id: plan_id.to_string(),
            timestamp: now,
            amount_spent,
            amount_received: received,
            price,
            tx_hash,
        };
        self.executions.push(execution);

        Ok(self.executions.last().unwrap())
    }

    // -- Queries ------------------------------------------------------------

    pub fn active_plans(&self) -> Vec<&DcaPlan> {
        self.plans
            .values()
            .filter(|p| p.status == DcaStatus::Active)
            .collect()
    }

    pub fn due_plans(&self) -> Vec<&DcaPlan> {
        let now = chrono::Utc::now().to_rfc3339();
        self.plans
            .values()
            .filter(|p| {
                p.status == DcaStatus::Active
                    && match &p.next_buy {
                        None => true, // first buy is always due
                        Some(next) => next.as_str() <= now.as_str(),
                    }
            })
            .collect()
    }

    pub fn plan_history(&self, plan_id: &str) -> Vec<&DcaExecution> {
        self.executions
            .iter()
            .filter(|e| e.plan_id == plan_id)
            .collect()
    }

    pub fn plan_performance(&self, plan_id: &str) -> Result<(f64, f64), DcaError> {
        let plan = self
            .plans
            .get(plan_id)
            .ok_or_else(|| DcaError::PlanNotFound(plan_id.to_string()))?;
        Ok((plan.avg_price, plan.total_received as f64))
    }

    // -- Analytics ----------------------------------------------------------

    pub fn stats(&self) -> DcaStats {
        let active_plans = self
            .plans
            .values()
            .filter(|p| p.status == DcaStatus::Active)
            .count();
        let paused_plans = self
            .plans
            .values()
            .filter(|p| p.status == DcaStatus::Paused)
            .count();
        let completed_plans = self
            .plans
            .values()
            .filter(|p| p.status == DcaStatus::Completed)
            .count();

        let total_spent: u64 = self.plans.values().map(|p| p.spent).sum();
        let total_received: u64 = self.plans.values().map(|p| p.total_received).sum();
        let total_buys: u32 = self.plans.values().map(|p| p.buys_completed).sum();

        let avg_price_all = if total_received > 0 {
            total_spent as f64 / total_received as f64
        } else {
            0.0
        };

        DcaStats {
            total_plans: self.plans.len(),
            active_plans,
            paused_plans,
            completed_plans,
            total_spent,
            total_received,
            total_buys,
            avg_price_all,
        }
    }

    // -- Persistence --------------------------------------------------------

    pub fn load(path: &Path) -> Result<Self, DcaError> {
        let data = std::fs::read_to_string(path)?;
        let engine: Self = serde_json::from_str(&data)?;
        Ok(engine)
    }

    pub fn save(&self, path: &Path) -> Result<(), DcaError> {
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn compute_next_buy(from: &str, frequency: &DcaFrequency) -> String {
    let seconds = match frequency {
        DcaFrequency::Hourly => 3600,
        DcaFrequency::Daily => 86400,
        DcaFrequency::Weekly => 604800,
        DcaFrequency::Monthly => 2592000, // 30 days
        DcaFrequency::Custom(s) => *s,
    };
    // Parse the RFC3339 timestamp and add seconds
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(from) {
        let next = dt + chrono::Duration::seconds(seconds as i64);
        next.to_rfc3339()
    } else {
        from.to_string()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("evaporchain_dca_test_{}_{}", std::process::id(), name))
    }

    fn sample_plan(id: &str) -> DcaPlan {
        DcaPlan {
            id: id.to_string(),
            name: format!("DCA Plan {}", id),
            token_from: "USDC".to_string(),
            token_to: "EVP".to_string(),
            amount_per_buy: 100,
            frequency: DcaFrequency::Daily,
            total_budget: Some(1000),
            spent: 0,
            buys_completed: 0,
            max_buys: Some(10),
            status: DcaStatus::Active,
            created_at: chrono::Utc::now().to_rfc3339(),
            last_buy: None,
            next_buy: None,
            min_price: None,
            max_price: None,
            total_received: 0,
            avg_price: 0.0,
        }
    }

    fn sample_plan_with_bounds(id: &str, min: f64, max: f64) -> DcaPlan {
        let mut plan = sample_plan(id);
        plan.min_price = Some(min);
        plan.max_price = Some(max);
        plan
    }

    #[test]
    fn test_create_plan() {
        let mut engine = DcaEngine::new();
        let plan = sample_plan("p1");
        assert!(engine.create_plan(plan).is_ok());
        assert!(engine.get_plan("p1").is_some());
    }

    #[test]
    fn test_create_plan_duplicate_error() {
        let mut engine = DcaEngine::new();
        engine.create_plan(sample_plan("p1")).unwrap();
        let result = engine.create_plan(sample_plan("p1"));
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_plan() {
        let mut engine = DcaEngine::new();
        engine.create_plan(sample_plan("p1")).unwrap();
        let removed = engine.remove_plan("p1").unwrap();
        assert_eq!(removed.id, "p1");
        assert!(engine.get_plan("p1").is_none());
    }

    #[test]
    fn test_remove_plan_not_found() {
        let mut engine = DcaEngine::new();
        assert!(engine.remove_plan("nonexistent").is_err());
    }

    #[test]
    fn test_get_plan_mut() {
        let mut engine = DcaEngine::new();
        engine.create_plan(sample_plan("p1")).unwrap();
        let plan = engine.get_plan_mut("p1").unwrap();
        plan.name = "Updated".to_string();
        assert_eq!(engine.get_plan("p1").unwrap().name, "Updated");
    }

    #[test]
    fn test_pause_plan() {
        let mut engine = DcaEngine::new();
        engine.create_plan(sample_plan("p1")).unwrap();
        assert!(engine.pause_plan("p1").is_ok());
        assert_eq!(engine.get_plan("p1").unwrap().status, DcaStatus::Paused);
    }

    #[test]
    fn test_pause_plan_not_active_error() {
        let mut engine = DcaEngine::new();
        engine.create_plan(sample_plan("p1")).unwrap();
        engine.pause_plan("p1").unwrap();
        assert!(engine.pause_plan("p1").is_err());
    }

    #[test]
    fn test_resume_plan() {
        let mut engine = DcaEngine::new();
        engine.create_plan(sample_plan("p1")).unwrap();
        engine.pause_plan("p1").unwrap();
        assert!(engine.resume_plan("p1").is_ok());
        assert_eq!(engine.get_plan("p1").unwrap().status, DcaStatus::Active);
    }

    #[test]
    fn test_resume_plan_not_paused_error() {
        let mut engine = DcaEngine::new();
        engine.create_plan(sample_plan("p1")).unwrap();
        assert!(engine.resume_plan("p1").is_err());
    }

    #[test]
    fn test_cancel_plan() {
        let mut engine = DcaEngine::new();
        engine.create_plan(sample_plan("p1")).unwrap();
        assert!(engine.cancel_plan("p1").is_ok());
        assert_eq!(engine.get_plan("p1").unwrap().status, DcaStatus::Cancelled);
    }

    #[test]
    fn test_cancel_plan_not_found() {
        let mut engine = DcaEngine::new();
        assert!(engine.cancel_plan("nonexistent").is_err());
    }

    #[test]
    fn test_execute_buy() {
        let mut engine = DcaEngine::new();
        engine.create_plan(sample_plan("p1")).unwrap();
        let exec = engine.execute_buy("p1", 0.5, 200, Some("tx123".to_string())).unwrap();
        assert_eq!(exec.plan_id, "p1");
        assert_eq!(exec.amount_spent, 100);
        assert_eq!(exec.amount_received, 200);
        assert_eq!(exec.price, 0.5);
        assert_eq!(exec.tx_hash, Some("tx123".to_string()));

        let plan = engine.get_plan("p1").unwrap();
        assert_eq!(plan.spent, 100);
        assert_eq!(plan.buys_completed, 1);
        assert_eq!(plan.total_received, 200);
        assert!(plan.last_buy.is_some());
        assert!(plan.next_buy.is_some());
    }

    #[test]
    fn test_execute_buy_not_active_error() {
        let mut engine = DcaEngine::new();
        engine.create_plan(sample_plan("p1")).unwrap();
        engine.pause_plan("p1").unwrap();
        assert!(engine.execute_buy("p1", 0.5, 200, None).is_err());
    }

    #[test]
    fn test_execute_buy_price_below_min() {
        let mut engine = DcaEngine::new();
        engine.create_plan(sample_plan_with_bounds("p1", 1.0, 10.0)).unwrap();
        let result = engine.execute_buy("p1", 0.5, 200, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_execute_buy_price_above_max() {
        let mut engine = DcaEngine::new();
        engine.create_plan(sample_plan_with_bounds("p1", 1.0, 10.0)).unwrap();
        let result = engine.execute_buy("p1", 15.0, 200, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_execute_buy_auto_complete_max_buys() {
        let mut engine = DcaEngine::new();
        let mut plan = sample_plan("p1");
        plan.max_buys = Some(2);
        plan.total_budget = None;
        engine.create_plan(plan).unwrap();

        engine.execute_buy("p1", 0.5, 200, None).unwrap();
        assert_eq!(engine.get_plan("p1").unwrap().status, DcaStatus::Active);

        engine.execute_buy("p1", 0.5, 200, None).unwrap();
        assert_eq!(engine.get_plan("p1").unwrap().status, DcaStatus::Completed);
    }

    #[test]
    fn test_execute_buy_auto_complete_budget() {
        let mut engine = DcaEngine::new();
        let mut plan = sample_plan("p1");
        plan.total_budget = Some(200);
        plan.max_buys = None;
        engine.create_plan(plan).unwrap();

        engine.execute_buy("p1", 0.5, 200, None).unwrap();
        engine.execute_buy("p1", 0.5, 200, None).unwrap();
        assert_eq!(engine.get_plan("p1").unwrap().status, DcaStatus::Completed);
    }

    #[test]
    fn test_execute_buy_budget_exhausted_error() {
        let mut engine = DcaEngine::new();
        let mut plan = sample_plan("p1");
        plan.total_budget = Some(150);
        plan.max_buys = None;
        engine.create_plan(plan).unwrap();

        engine.execute_buy("p1", 0.5, 200, None).unwrap();
        // Next buy would spend 100 more (total 200), exceeding budget of 150
        let result = engine.execute_buy("p1", 0.5, 200, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_active_plans() {
        let mut engine = DcaEngine::new();
        engine.create_plan(sample_plan("p1")).unwrap();
        engine.create_plan(sample_plan("p2")).unwrap();
        engine.pause_plan("p2").unwrap();
        assert_eq!(engine.active_plans().len(), 1);
        assert_eq!(engine.active_plans()[0].id, "p1");
    }

    #[test]
    fn test_due_plans_first_buy() {
        let mut engine = DcaEngine::new();
        engine.create_plan(sample_plan("p1")).unwrap();
        // next_buy is None, so plan is due for first buy
        let due = engine.due_plans();
        assert_eq!(due.len(), 1);
    }

    #[test]
    fn test_plan_history() {
        let mut engine = DcaEngine::new();
        engine.create_plan(sample_plan("p1")).unwrap();
        engine.create_plan(sample_plan("p2")).unwrap();
        engine.execute_buy("p1", 0.5, 200, None).unwrap();
        engine.execute_buy("p1", 0.6, 166, None).unwrap();
        engine.execute_buy("p2", 0.7, 142, None).unwrap();

        let history = engine.plan_history("p1");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].price, 0.5);
        assert_eq!(history[1].price, 0.6);
    }

    #[test]
    fn test_plan_performance() {
        let mut engine = DcaEngine::new();
        engine.create_plan(sample_plan("p1")).unwrap();
        engine.execute_buy("p1", 0.5, 200, None).unwrap();
        engine.execute_buy("p1", 1.0, 100, None).unwrap();

        let (avg_price, total_value) = engine.plan_performance("p1").unwrap();
        // spent 200 total, received 300 total => avg_price = 200/300 ~ 0.666
        assert!((avg_price - (200.0 / 300.0)).abs() < 0.01);
        assert!((total_value - 300.0).abs() < 0.01);
    }

    #[test]
    fn test_plan_performance_not_found() {
        let engine = DcaEngine::new();
        assert!(engine.plan_performance("nonexistent").is_err());
    }

    #[test]
    fn test_stats() {
        let mut engine = DcaEngine::new();
        engine.create_plan(sample_plan("p1")).unwrap();
        engine.create_plan(sample_plan("p2")).unwrap();
        engine.pause_plan("p2").unwrap();
        engine.execute_buy("p1", 0.5, 200, None).unwrap();

        let stats = engine.stats();
        assert_eq!(stats.total_plans, 2);
        assert_eq!(stats.active_plans, 1);
        assert_eq!(stats.paused_plans, 1);
        assert_eq!(stats.completed_plans, 0);
        assert_eq!(stats.total_spent, 100);
        assert_eq!(stats.total_received, 200);
        assert_eq!(stats.total_buys, 1);
        assert!(stats.avg_price_all > 0.0);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let mut engine = DcaEngine::new();
        engine.create_plan(sample_plan("p1")).unwrap();
        engine.execute_buy("p1", 0.5, 200, None).unwrap();

        let path = tmp_path("roundtrip.json");
        engine.save(&path).unwrap();

        let loaded = DcaEngine::load(&path).unwrap();
        assert_eq!(loaded.plans.len(), 1);
        assert_eq!(loaded.executions.len(), 1);
        assert_eq!(loaded.get_plan("p1").unwrap().spent, 100);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = tmp_path("nonexistent_dca.json");
        let engine = DcaEngine::load_or_default(&path);
        assert_eq!(engine.plans.len(), 0);
        assert_eq!(engine.executions.len(), 0);
    }
}
