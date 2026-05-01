// wallet/src/intent.rs — Intent-based transactions
//
// Declare desired outcomes and let solvers find optimal execution paths:
//   - Swap intents (best price across DEXes)
//   - Batch intents (multiple ops in one)
//   - Conditional intents (if-then execution)
//   - Solver ranking and selection

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IntentError {
    #[error("intent not found: {0}")]
    NotFound(String),
    #[error("solver not found: {0}")]
    SolverNotFound(String),
    #[error("no solutions available")]
    NoSolutions,
    #[error("intent expired")]
    Expired,
    #[error("intent already resolved")]
    AlreadyResolved,
    #[error("invalid intent: {0}")]
    Invalid(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
}

// ── Intent types ──────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntentType {
    Swap,
    Transfer,
    BatchTransfer,
    Conditional,
    Recurring,
    Bridge,
}

impl IntentType {
    pub fn name(&self) -> &'static str {
        match self {
            IntentType::Swap => "swap",
            IntentType::Transfer => "transfer",
            IntentType::BatchTransfer => "batch_transfer",
            IntentType::Conditional => "conditional",
            IntentType::Recurring => "recurring",
            IntentType::Bridge => "bridge",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntentStatus {
    Open,
    Solving,
    Solved,
    Executing,
    Completed,
    Failed,
    Expired,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Constraint {
    pub key: String,
    pub operator: ConstraintOp,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConstraintOp {
    Eq,
    Lt,
    Gt,
    Lte,
    Gte,
    Contains,
}

impl Constraint {
    pub fn new(key: &str, op: ConstraintOp, value: &str) -> Self {
        Self {
            key: key.to_string(),
            operator: op,
            value: value.to_string(),
        }
    }

    /// Evaluate against a context value
    pub fn check(&self, actual: &str) -> bool {
        match self.operator {
            ConstraintOp::Eq => actual == self.value,
            ConstraintOp::Contains => actual.contains(&self.value),
            ConstraintOp::Lt | ConstraintOp::Gt | ConstraintOp::Lte | ConstraintOp::Gte => {
                if let (Ok(a), Ok(b)) = (actual.parse::<f64>(), self.value.parse::<f64>()) {
                    match self.operator {
                        ConstraintOp::Lt => a < b,
                        ConstraintOp::Gt => a > b,
                        ConstraintOp::Lte => a <= b,
                        ConstraintOp::Gte => a >= b,
                        _ => false,
                    }
                } else {
                    false
                }
            }
        }
    }
}

// ── Intent ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Intent {
    pub id: String,
    pub intent_type: IntentType,
    pub sender: String,
    pub description: String,
    pub params: HashMap<String, String>,
    pub constraints: Vec<Constraint>,
    pub max_gas: Option<u64>,
    pub deadline: Option<String>,
    pub status: IntentStatus,
    pub created_at: String,
    pub resolved_at: Option<String>,
    pub solution_id: Option<String>,
}

impl Intent {
    pub fn new(id: &str, intent_type: IntentType, sender: &str, description: &str) -> Self {
        Self {
            id: id.to_string(),
            intent_type,
            sender: sender.to_string(),
            description: description.to_string(),
            params: HashMap::new(),
            constraints: Vec::new(),
            max_gas: None,
            deadline: None,
            status: IntentStatus::Open,
            created_at: chrono::Utc::now().to_rfc3339(),
            resolved_at: None,
            solution_id: None,
        }
    }

    pub fn with_param(mut self, key: &str, value: &str) -> Self {
        self.params.insert(key.to_string(), value.to_string());
        self
    }

    pub fn with_constraint(mut self, constraint: Constraint) -> Self {
        self.constraints.push(constraint);
        self
    }

    pub fn with_max_gas(mut self, max_gas: u64) -> Self {
        self.max_gas = Some(max_gas);
        self
    }

    pub fn with_deadline(mut self, deadline: &str) -> Self {
        self.deadline = Some(deadline.to_string());
        self
    }

    pub fn is_expired(&self, now: &str) -> bool {
        match &self.deadline {
            Some(d) => now >= d.as_str(),
            None => false,
        }
    }

    pub fn cancel(&mut self) -> Result<(), IntentError> {
        if self.status == IntentStatus::Completed || self.status == IntentStatus::Cancelled {
            return Err(IntentError::AlreadyResolved);
        }
        self.status = IntentStatus::Cancelled;
        Ok(())
    }

    pub fn resolve(&mut self, solution_id: &str) -> Result<(), IntentError> {
        if self.status != IntentStatus::Open && self.status != IntentStatus::Solving {
            return Err(IntentError::AlreadyResolved);
        }
        self.status = IntentStatus::Solved;
        self.solution_id = Some(solution_id.to_string());
        self.resolved_at = Some(chrono::Utc::now().to_rfc3339());
        Ok(())
    }

    pub fn mark_completed(&mut self) {
        self.status = IntentStatus::Completed;
    }

    pub fn mark_failed(&mut self) {
        self.status = IntentStatus::Failed;
    }
}

// ── Solution ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Solution {
    pub id: String,
    pub intent_id: String,
    pub solver_id: String,
    pub steps: Vec<SolutionStep>,
    pub estimated_gas: u64,
    pub estimated_cost: u64,
    pub score: f64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolutionStep {
    pub action: String,
    pub target: String,
    pub params: HashMap<String, String>,
    pub estimated_gas: u64,
}

impl Solution {
    pub fn new(id: &str, intent_id: &str, solver_id: &str) -> Self {
        Self {
            id: id.to_string(),
            intent_id: intent_id.to_string(),
            solver_id: solver_id.to_string(),
            steps: Vec::new(),
            estimated_gas: 0,
            estimated_cost: 0,
            score: 0.0,
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub fn add_step(&mut self, action: &str, target: &str, gas: u64) {
        self.steps.push(SolutionStep {
            action: action.to_string(),
            target: target.to_string(),
            params: HashMap::new(),
            estimated_gas: gas,
        });
        self.estimated_gas += gas;
    }

    pub fn step_count(&self) -> usize {
        self.steps.len()
    }
}

// ── Solver ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Solver {
    pub id: String,
    pub name: String,
    pub supported_types: Vec<IntentType>,
    pub success_count: u64,
    pub failure_count: u64,
    pub avg_savings_percent: f64,
    pub active: bool,
}

impl Solver {
    pub fn new(id: &str, name: &str, supported: Vec<IntentType>) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            supported_types: supported,
            success_count: 0,
            failure_count: 0,
            avg_savings_percent: 0.0,
            active: true,
        }
    }

    pub fn reliability(&self) -> f64 {
        let total = self.success_count + self.failure_count;
        if total == 0 {
            return 1.0;
        }
        self.success_count as f64 / total as f64
    }

    pub fn supports(&self, intent_type: &IntentType) -> bool {
        self.supported_types.contains(intent_type)
    }

    pub fn record_success(&mut self, savings_percent: f64) {
        let total = self.success_count + self.failure_count + 1;
        self.avg_savings_percent =
            (self.avg_savings_percent * (total - 1) as f64 + savings_percent) / total as f64;
        self.success_count += 1;
    }

    pub fn record_failure(&mut self) {
        self.failure_count += 1;
    }
}

// ── Intent engine ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IntentEngine {
    pub intents: HashMap<String, Intent>,
    pub solutions: HashMap<String, Solution>,
    pub solvers: HashMap<String, Solver>,
}

impl IntentEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn submit_intent(&mut self, intent: Intent) -> Result<(), IntentError> {
        if intent.id.is_empty() {
            return Err(IntentError::Invalid("intent id required".into()));
        }
        self.intents.insert(intent.id.clone(), intent);
        Ok(())
    }

    pub fn get_intent(&self, id: &str) -> Option<&Intent> {
        self.intents.get(id)
    }

    pub fn get_intent_mut(&mut self, id: &str) -> Option<&mut Intent> {
        self.intents.get_mut(id)
    }

    pub fn list_intents(&self) -> Vec<&Intent> {
        self.intents.values().collect()
    }

    pub fn open_intents(&self) -> Vec<&Intent> {
        self.intents
            .values()
            .filter(|i| i.status == IntentStatus::Open)
            .collect()
    }

    pub fn register_solver(&mut self, solver: Solver) {
        self.solvers.insert(solver.id.clone(), solver);
    }

    pub fn list_solvers(&self) -> Vec<&Solver> {
        self.solvers.values().collect()
    }

    /// Find solvers that can handle a given intent type
    pub fn solvers_for(&self, intent_type: &IntentType) -> Vec<&Solver> {
        self.solvers
            .values()
            .filter(|s| s.active && s.supports(intent_type))
            .collect()
    }

    pub fn submit_solution(&mut self, solution: Solution) -> Result<(), IntentError> {
        // Verify intent exists
        if !self.intents.contains_key(&solution.intent_id) {
            return Err(IntentError::NotFound(solution.intent_id.clone()));
        }
        self.solutions.insert(solution.id.clone(), solution);
        Ok(())
    }

    /// Get best solution for an intent (highest score)
    pub fn best_solution(&self, intent_id: &str) -> Option<&Solution> {
        self.solutions
            .values()
            .filter(|s| s.intent_id == intent_id)
            .max_by(|a, b| {
                a.score
                    .partial_cmp(&b.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// Get all solutions for an intent
    pub fn solutions_for(&self, intent_id: &str) -> Vec<&Solution> {
        self.solutions
            .values()
            .filter(|s| s.intent_id == intent_id)
            .collect()
    }

    /// Stats
    pub fn stats(&self) -> IntentStats {
        let total = self.intents.len();
        let open = self
            .intents
            .values()
            .filter(|i| i.status == IntentStatus::Open)
            .count();
        let completed = self
            .intents
            .values()
            .filter(|i| i.status == IntentStatus::Completed)
            .count();
        let failed = self
            .intents
            .values()
            .filter(|i| i.status == IntentStatus::Failed)
            .count();
        IntentStats {
            total_intents: total,
            open,
            completed,
            failed,
            total_solutions: self.solutions.len(),
            total_solvers: self.solvers.len(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), IntentError> {
        let json =
            serde_json::to_string_pretty(self).map_err(|e| IntentError::Json(e.to_string()))?;
        std::fs::write(path, json).map_err(|e| IntentError::Io(e.to_string()))
    }

    pub fn load(path: &Path) -> Result<Self, IntentError> {
        let data = std::fs::read_to_string(path).map_err(|e| IntentError::Io(e.to_string()))?;
        serde_json::from_str(&data).map_err(|e| IntentError::Json(e.to_string()))
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentStats {
    pub total_intents: usize,
    pub open: usize,
    pub completed: usize,
    pub failed: usize,
    pub total_solutions: usize,
    pub total_solvers: usize,
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_intent(id: &str) -> Intent {
        Intent::new(id, IntentType::Transfer, "evap1alice", "send 100 to bob")
            .with_param("to", "evap1bob")
            .with_param("amount", "100")
    }

    #[test]
    fn test_intent_new() {
        let i = make_intent("i1");
        assert_eq!(i.status, IntentStatus::Open);
        assert_eq!(i.params.get("to"), Some(&"evap1bob".to_string()));
    }

    #[test]
    fn test_intent_with_constraint() {
        let i =
            make_intent("i1").with_constraint(Constraint::new("gas", ConstraintOp::Lte, "50000"));
        assert_eq!(i.constraints.len(), 1);
    }

    #[test]
    fn test_intent_with_deadline() {
        let i = make_intent("i1").with_deadline("2099-01-01T00:00:00Z");
        assert!(!i.is_expired(&chrono::Utc::now().to_rfc3339()));
    }

    #[test]
    fn test_intent_expired() {
        let i = make_intent("i1").with_deadline("2020-01-01T00:00:00Z");
        assert!(i.is_expired(&chrono::Utc::now().to_rfc3339()));
    }

    #[test]
    fn test_intent_cancel() {
        let mut i = make_intent("i1");
        i.cancel().unwrap();
        assert_eq!(i.status, IntentStatus::Cancelled);
    }

    #[test]
    fn test_intent_double_cancel() {
        let mut i = make_intent("i1");
        i.cancel().unwrap();
        assert!(i.cancel().is_err());
    }

    #[test]
    fn test_intent_resolve() {
        let mut i = make_intent("i1");
        i.resolve("sol1").unwrap();
        assert_eq!(i.status, IntentStatus::Solved);
        assert_eq!(i.solution_id, Some("sol1".to_string()));
    }

    #[test]
    fn test_intent_resolve_already_resolved() {
        let mut i = make_intent("i1");
        i.mark_completed();
        assert!(i.resolve("sol1").is_err());
    }

    #[test]
    fn test_constraint_eq() {
        let c = Constraint::new("type", ConstraintOp::Eq, "transfer");
        assert!(c.check("transfer"));
        assert!(!c.check("swap"));
    }

    #[test]
    fn test_constraint_lt() {
        let c = Constraint::new("gas", ConstraintOp::Lt, "50000");
        assert!(c.check("21000"));
        assert!(!c.check("60000"));
    }

    #[test]
    fn test_constraint_gte() {
        let c = Constraint::new("amount", ConstraintOp::Gte, "100");
        assert!(c.check("100"));
        assert!(c.check("200"));
        assert!(!c.check("50"));
    }

    #[test]
    fn test_constraint_contains() {
        let c = Constraint::new("desc", ConstraintOp::Contains, "swap");
        assert!(c.check("token swap for ETH"));
        assert!(!c.check("transfer only"));
    }

    #[test]
    fn test_solution_new() {
        let mut sol = Solution::new("s1", "i1", "solver1");
        sol.add_step("transfer", "evap1bob", 21000);
        assert_eq!(sol.step_count(), 1);
        assert_eq!(sol.estimated_gas, 21000);
    }

    #[test]
    fn test_solution_multi_step() {
        let mut sol = Solution::new("s1", "i1", "solver1");
        sol.add_step("approve", "evap1dex", 10000);
        sol.add_step("swap", "evap1dex", 50000);
        sol.add_step("transfer", "evap1bob", 21000);
        assert_eq!(sol.step_count(), 3);
        assert_eq!(sol.estimated_gas, 81000);
    }

    #[test]
    fn test_solver_new() {
        let s = Solver::new(
            "s1",
            "Fast Solver",
            vec![IntentType::Transfer, IntentType::Swap],
        );
        assert!(s.supports(&IntentType::Transfer));
        assert!(!s.supports(&IntentType::Bridge));
    }

    #[test]
    fn test_solver_reliability() {
        let mut s = Solver::new("s1", "S", vec![IntentType::Transfer]);
        s.record_success(5.0);
        s.record_success(10.0);
        s.record_failure();
        assert!((s.reliability() - 0.666).abs() < 0.01);
    }

    #[test]
    fn test_engine_submit_intent() {
        let mut e = IntentEngine::new();
        e.submit_intent(make_intent("i1")).unwrap();
        assert_eq!(e.list_intents().len(), 1);
        assert_eq!(e.open_intents().len(), 1);
    }

    #[test]
    fn test_engine_empty_id() {
        let mut e = IntentEngine::new();
        let i = Intent::new("", IntentType::Transfer, "a", "x");
        assert!(e.submit_intent(i).is_err());
    }

    #[test]
    fn test_engine_register_solver() {
        let mut e = IntentEngine::new();
        e.register_solver(Solver::new("s1", "S1", vec![IntentType::Transfer]));
        assert_eq!(e.list_solvers().len(), 1);
        assert_eq!(e.solvers_for(&IntentType::Transfer).len(), 1);
        assert_eq!(e.solvers_for(&IntentType::Swap).len(), 0);
    }

    #[test]
    fn test_engine_submit_solution() {
        let mut e = IntentEngine::new();
        e.submit_intent(make_intent("i1")).unwrap();
        let mut sol = Solution::new("s1", "i1", "solver1");
        sol.score = 95.0;
        e.submit_solution(sol).unwrap();
        assert_eq!(e.solutions_for("i1").len(), 1);
    }

    #[test]
    fn test_engine_submit_solution_no_intent() {
        let mut e = IntentEngine::new();
        let sol = Solution::new("s1", "nope", "solver1");
        assert!(e.submit_solution(sol).is_err());
    }

    #[test]
    fn test_engine_best_solution() {
        let mut e = IntentEngine::new();
        e.submit_intent(make_intent("i1")).unwrap();
        let mut sol1 = Solution::new("s1", "i1", "solver1");
        sol1.score = 80.0;
        let mut sol2 = Solution::new("s2", "i1", "solver2");
        sol2.score = 95.0;
        e.submit_solution(sol1).unwrap();
        e.submit_solution(sol2).unwrap();
        let best = e.best_solution("i1").unwrap();
        assert_eq!(best.id, "s2");
    }

    #[test]
    fn test_engine_stats() {
        let mut e = IntentEngine::new();
        e.submit_intent(make_intent("i1")).unwrap();
        let mut i2 = make_intent("i2");
        i2.mark_completed();
        e.submit_intent(i2).unwrap();
        let stats = e.stats();
        assert_eq!(stats.total_intents, 2);
        assert_eq!(stats.open, 1);
        assert_eq!(stats.completed, 1);
    }

    #[test]
    fn test_engine_save_load() {
        let path = std::env::temp_dir().join(format!("evap_intent_{}.json", std::process::id()));
        let mut e = IntentEngine::new();
        e.submit_intent(make_intent("i1")).unwrap();
        e.save(&path).unwrap();
        let loaded = IntentEngine::load(&path).unwrap();
        assert_eq!(loaded.list_intents().len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_intent_type_name() {
        assert_eq!(IntentType::Swap.name(), "swap");
        assert_eq!(IntentType::Bridge.name(), "bridge");
    }
}
