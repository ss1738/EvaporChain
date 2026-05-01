use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum TxSimulatorError {
    #[error("simulation not found: {0}")]
    SimulationNotFound(String),
    #[error("fork not found: {0}")]
    ForkNotFound(String),
    #[error("scenario not found: {0}")]
    ScenarioNotFound(String),
    #[error("duplicate simulation: {0}")]
    DuplicateSimulation(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SimStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Reverted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RevertReason {
    OutOfGas,
    InsufficientBalance,
    InvalidNonce,
    ContractError(String),
    EnergyDecayed,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ForkSource {
    LatestBlock,
    SpecificBlock(u64),
    Snapshot(String),
}

// ---------------------------------------------------------------------------
// Data structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SimulatedTx {
    pub id: String,
    pub from: String,
    pub to: String,
    pub amount: u64,
    pub gas_limit: u64,
    pub data: Option<String>,
    pub nonce: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationResult2 {
    pub id: String,
    pub tx: SimulatedTx,
    pub status: SimStatus,
    pub gas_used: u64,
    pub gas_refund: u64,
    pub return_data: Option<String>,
    pub revert_reason: Option<RevertReason>,
    pub state_changes: Vec<StateChange>,
    pub logs: Vec<SimLog>,
    pub executed_at: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StateChange {
    pub address: String,
    pub field: String,
    pub old_value: String,
    pub new_value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SimLog {
    pub index: u32,
    pub address: String,
    pub topic: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateFork {
    pub id: String,
    pub source: ForkSource,
    pub block_number: u64,
    pub created_at: String,
    pub state_snapshot: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatIfScenario {
    pub id: String,
    pub name: String,
    pub base_fork: String,
    pub transactions: Vec<SimulatedTx>,
    pub results: Vec<SimulationResult2>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SimulatorStats {
    pub total_simulations: usize,
    pub successful: usize,
    pub failed: usize,
    pub reverted: usize,
    pub total_forks: usize,
    pub total_scenarios: usize,
    pub avg_gas_used: f64,
    pub total_gas_simulated: u64,
}

// ---------------------------------------------------------------------------
// Main struct
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TxSimulator {
    pub simulations: HashMap<String, SimulationResult2>,
    pub forks: HashMap<String, StateFork>,
    pub scenarios: HashMap<String, WhatIfScenario>,
}

impl TxSimulator {
    pub fn new() -> Self {
        Self::default()
    }

    // -- Fork management ----------------------------------------------------

    pub fn create_fork(
        &mut self,
        id: &str,
        source: ForkSource,
        block_number: u64,
    ) -> Result<(), TxSimulatorError> {
        if self.forks.contains_key(id) {
            return Err(TxSimulatorError::DuplicateSimulation(id.to_string()));
        }
        let fork = StateFork {
            id: id.to_string(),
            source,
            block_number,
            created_at: Utc::now().to_rfc3339(),
            state_snapshot: HashMap::new(),
        };
        self.forks.insert(id.to_string(), fork);
        Ok(())
    }

    pub fn remove_fork(&mut self, id: &str) -> Result<StateFork, TxSimulatorError> {
        self.forks
            .remove(id)
            .ok_or_else(|| TxSimulatorError::ForkNotFound(id.to_string()))
    }

    // -- Transaction simulation ---------------------------------------------

    pub fn simulate_tx(&self, tx: SimulatedTx) -> SimulationResult2 {
        let now = Utc::now();

        if tx.gas_limit < 21000 {
            return SimulationResult2 {
                id: tx.id.clone(),
                tx,
                status: SimStatus::Reverted,
                gas_used: 0,
                gas_refund: 0,
                return_data: None,
                revert_reason: Some(RevertReason::OutOfGas),
                state_changes: Vec::new(),
                logs: Vec::new(),
                executed_at: now.to_rfc3339(),
                duration_ms: 1,
            };
        }

        let gas_used = 21000 + (tx.amount % 1000);
        let state_changes = vec![StateChange {
            address: tx.from.clone(),
            field: "balance".to_string(),
            old_value: format!("{}", tx.amount + 1000),
            new_value: format!("{}", 1000),
        }];

        SimulationResult2 {
            id: tx.id.clone(),
            tx,
            status: SimStatus::Completed,
            gas_used,
            gas_refund: 0,
            return_data: None,
            revert_reason: None,
            state_changes,
            logs: Vec::new(),
            executed_at: now.to_rfc3339(),
            duration_ms: 5,
        }
    }

    pub fn store_result(&mut self, result: SimulationResult2) -> Result<(), TxSimulatorError> {
        if self.simulations.contains_key(&result.id) {
            return Err(TxSimulatorError::DuplicateSimulation(result.id.clone()));
        }
        self.simulations.insert(result.id.clone(), result);
        Ok(())
    }

    pub fn get_result(&self, id: &str) -> Option<&SimulationResult2> {
        self.simulations.get(id)
    }

    // -- Scenario management ------------------------------------------------

    pub fn create_scenario(
        &mut self,
        id: &str,
        name: &str,
        fork_id: &str,
    ) -> Result<(), TxSimulatorError> {
        if !self.forks.contains_key(fork_id) {
            return Err(TxSimulatorError::ForkNotFound(fork_id.to_string()));
        }
        if self.scenarios.contains_key(id) {
            return Err(TxSimulatorError::DuplicateSimulation(id.to_string()));
        }
        let scenario = WhatIfScenario {
            id: id.to_string(),
            name: name.to_string(),
            base_fork: fork_id.to_string(),
            transactions: Vec::new(),
            results: Vec::new(),
            created_at: Utc::now().to_rfc3339(),
        };
        self.scenarios.insert(id.to_string(), scenario);
        Ok(())
    }

    pub fn add_tx_to_scenario(
        &mut self,
        scenario_id: &str,
        tx: SimulatedTx,
    ) -> Result<(), TxSimulatorError> {
        let scenario = self
            .scenarios
            .get_mut(scenario_id)
            .ok_or_else(|| TxSimulatorError::ScenarioNotFound(scenario_id.to_string()))?;
        scenario.transactions.push(tx);
        Ok(())
    }

    pub fn run_scenario(
        &mut self,
        scenario_id: &str,
    ) -> Result<Vec<SimulationResult2>, TxSimulatorError> {
        let scenario = self
            .scenarios
            .get(scenario_id)
            .ok_or_else(|| TxSimulatorError::ScenarioNotFound(scenario_id.to_string()))?;

        let txs: Vec<SimulatedTx> = scenario.transactions.clone();
        let mut results = Vec::new();
        for tx in txs {
            let result = self.simulate_tx(tx);
            results.push(result);
        }

        let scenario = self.scenarios.get_mut(scenario_id).unwrap();
        scenario.results = results.clone();
        Ok(results)
    }

    // -- Analysis -----------------------------------------------------------

    pub fn revert_analysis(&self, id: &str) -> Result<Option<&RevertReason>, TxSimulatorError> {
        let result = self
            .simulations
            .get(id)
            .ok_or_else(|| TxSimulatorError::SimulationNotFound(id.to_string()))?;
        Ok(result.revert_reason.as_ref())
    }

    pub fn results_by_status(&self, status: &SimStatus) -> Vec<&SimulationResult2> {
        self.simulations
            .values()
            .filter(|r| r.status == *status)
            .collect()
    }

    pub fn recent_simulations(&self, n: usize) -> Vec<&SimulationResult2> {
        let mut results: Vec<&SimulationResult2> = self.simulations.values().collect();
        results.sort_by(|a, b| b.executed_at.cmp(&a.executed_at));
        results.truncate(n);
        results
    }

    pub fn stats(&self) -> SimulatorStats {
        let total_simulations = self.simulations.len();
        let successful = self
            .simulations
            .values()
            .filter(|r| r.status == SimStatus::Completed)
            .count();
        let failed = self
            .simulations
            .values()
            .filter(|r| r.status == SimStatus::Failed)
            .count();
        let reverted = self
            .simulations
            .values()
            .filter(|r| r.status == SimStatus::Reverted)
            .count();
        let total_gas_simulated: u64 = self.simulations.values().map(|r| r.gas_used).sum();
        let avg_gas_used = if total_simulations > 0 {
            total_gas_simulated as f64 / total_simulations as f64
        } else {
            0.0
        };

        SimulatorStats {
            total_simulations,
            successful,
            failed,
            reverted,
            total_forks: self.forks.len(),
            total_scenarios: self.scenarios.len(),
            avg_gas_used,
            total_gas_simulated,
        }
    }

    // -- Persistence --------------------------------------------------------

    pub fn save(&self, path: &Path) -> Result<(), TxSimulatorError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, TxSimulatorError> {
        let data = std::fs::read_to_string(path)?;
        let simulator: Self = serde_json::from_str(&data)?;
        Ok(simulator)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;
    use std::process;

    fn test_path(name: &str) -> std::path::PathBuf {
        temp_dir().join(format!("tx_simulator_test_{}_{}.json", process::id(), name))
    }

    fn sample_tx(id: &str, amount: u64, gas_limit: u64) -> SimulatedTx {
        SimulatedTx {
            id: id.to_string(),
            from: "0xAlice".to_string(),
            to: "0xBob".to_string(),
            amount,
            gas_limit,
            data: None,
            nonce: 1,
        }
    }

    #[test]
    fn test_create_fork() {
        let mut sim = TxSimulator::new();
        assert!(sim.create_fork("f1", ForkSource::LatestBlock, 100).is_ok());
        assert!(sim.forks.contains_key("f1"));
    }

    #[test]
    fn test_create_fork_duplicate() {
        let mut sim = TxSimulator::new();
        sim.create_fork("f1", ForkSource::LatestBlock, 100).unwrap();
        let err = sim
            .create_fork("f1", ForkSource::SpecificBlock(50), 50)
            .unwrap_err();
        assert!(matches!(err, TxSimulatorError::DuplicateSimulation(_)));
    }

    #[test]
    fn test_remove_fork() {
        let mut sim = TxSimulator::new();
        sim.create_fork("f1", ForkSource::LatestBlock, 100).unwrap();
        let fork = sim.remove_fork("f1").unwrap();
        assert_eq!(fork.id, "f1");
        assert!(!sim.forks.contains_key("f1"));
    }

    #[test]
    fn test_remove_fork_not_found() {
        let mut sim = TxSimulator::new();
        let err = sim.remove_fork("nope").unwrap_err();
        assert!(matches!(err, TxSimulatorError::ForkNotFound(_)));
    }

    #[test]
    fn test_simulate_tx_success() {
        let sim = TxSimulator::new();
        let tx = sample_tx("tx1", 500, 50000);
        let result = sim.simulate_tx(tx);
        assert_eq!(result.status, SimStatus::Completed);
        assert_eq!(result.gas_used, 21000 + (500 % 1000));
        assert!(result.revert_reason.is_none());
        assert!(!result.state_changes.is_empty());
    }

    #[test]
    fn test_simulate_tx_revert_out_of_gas() {
        let sim = TxSimulator::new();
        let tx = sample_tx("tx2", 100, 5000);
        let result = sim.simulate_tx(tx);
        assert_eq!(result.status, SimStatus::Reverted);
        assert_eq!(result.revert_reason, Some(RevertReason::OutOfGas));
        assert_eq!(result.gas_used, 0);
    }

    #[test]
    fn test_store_and_get_result() {
        let mut sim = TxSimulator::new();
        let tx = sample_tx("tx1", 500, 50000);
        let result = sim.simulate_tx(tx);
        sim.store_result(result).unwrap();
        let fetched = sim.get_result("tx1");
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().id, "tx1");
    }

    #[test]
    fn test_store_result_duplicate() {
        let mut sim = TxSimulator::new();
        let tx1 = sample_tx("tx1", 500, 50000);
        let r1 = sim.simulate_tx(tx1);
        sim.store_result(r1).unwrap();

        let tx2 = sample_tx("tx1", 200, 50000);
        let r2 = sim.simulate_tx(tx2);
        let err = sim.store_result(r2).unwrap_err();
        assert!(matches!(err, TxSimulatorError::DuplicateSimulation(_)));
    }

    #[test]
    fn test_get_result_not_found() {
        let sim = TxSimulator::new();
        assert!(sim.get_result("missing").is_none());
    }

    #[test]
    fn test_create_scenario() {
        let mut sim = TxSimulator::new();
        sim.create_fork("f1", ForkSource::LatestBlock, 100).unwrap();
        assert!(sim.create_scenario("s1", "Test Scenario", "f1").is_ok());
        assert!(sim.scenarios.contains_key("s1"));
    }

    #[test]
    fn test_create_scenario_fork_not_found() {
        let mut sim = TxSimulator::new();
        let err = sim
            .create_scenario("s1", "Test Scenario", "missing_fork")
            .unwrap_err();
        assert!(matches!(err, TxSimulatorError::ForkNotFound(_)));
    }

    #[test]
    fn test_add_tx_to_scenario() {
        let mut sim = TxSimulator::new();
        sim.create_fork("f1", ForkSource::LatestBlock, 100).unwrap();
        sim.create_scenario("s1", "Test", "f1").unwrap();
        let tx = sample_tx("tx1", 500, 50000);
        sim.add_tx_to_scenario("s1", tx).unwrap();
        assert_eq!(sim.scenarios["s1"].transactions.len(), 1);
    }

    #[test]
    fn test_add_tx_to_scenario_not_found() {
        let mut sim = TxSimulator::new();
        let tx = sample_tx("tx1", 500, 50000);
        let err = sim.add_tx_to_scenario("missing", tx).unwrap_err();
        assert!(matches!(err, TxSimulatorError::ScenarioNotFound(_)));
    }

    #[test]
    fn test_run_scenario() {
        let mut sim = TxSimulator::new();
        sim.create_fork("f1", ForkSource::LatestBlock, 100).unwrap();
        sim.create_scenario("s1", "Test", "f1").unwrap();
        sim.add_tx_to_scenario("s1", sample_tx("tx1", 100, 50000))
            .unwrap();
        sim.add_tx_to_scenario("s1", sample_tx("tx2", 200, 50000))
            .unwrap();

        let results = sim.run_scenario("s1").unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].status, SimStatus::Completed);
        assert_eq!(results[1].status, SimStatus::Completed);
        assert_eq!(sim.scenarios["s1"].results.len(), 2);
    }

    #[test]
    fn test_run_scenario_not_found() {
        let mut sim = TxSimulator::new();
        let err = sim.run_scenario("missing").unwrap_err();
        assert!(matches!(err, TxSimulatorError::ScenarioNotFound(_)));
    }

    #[test]
    fn test_revert_analysis() {
        let mut sim = TxSimulator::new();
        let tx = sample_tx("tx_rev", 100, 5000);
        let result = sim.simulate_tx(tx);
        sim.store_result(result).unwrap();

        let reason = sim.revert_analysis("tx_rev").unwrap();
        assert_eq!(reason, Some(&RevertReason::OutOfGas));
    }

    #[test]
    fn test_revert_analysis_not_found() {
        let sim = TxSimulator::new();
        let err = sim.revert_analysis("missing").unwrap_err();
        assert!(matches!(err, TxSimulatorError::SimulationNotFound(_)));
    }

    #[test]
    fn test_results_by_status() {
        let mut sim = TxSimulator::new();
        let r1 = sim.simulate_tx(sample_tx("a", 100, 50000));
        let r2 = sim.simulate_tx(sample_tx("b", 200, 5000)); // reverted
        let r3 = sim.simulate_tx(sample_tx("c", 300, 50000));
        sim.store_result(r1).unwrap();
        sim.store_result(r2).unwrap();
        sim.store_result(r3).unwrap();

        let completed = sim.results_by_status(&SimStatus::Completed);
        assert_eq!(completed.len(), 2);
        let reverted = sim.results_by_status(&SimStatus::Reverted);
        assert_eq!(reverted.len(), 1);
    }

    #[test]
    fn test_recent_simulations() {
        let mut sim = TxSimulator::new();
        for i in 0..5 {
            let r = sim.simulate_tx(sample_tx(&format!("t{}", i), i * 100, 50000));
            sim.store_result(r).unwrap();
        }
        let recent = sim.recent_simulations(3);
        assert_eq!(recent.len(), 3);
    }

    #[test]
    fn test_stats() {
        let mut sim = TxSimulator::new();
        sim.create_fork("f1", ForkSource::LatestBlock, 100).unwrap();
        sim.create_scenario("s1", "Test", "f1").unwrap();

        let r1 = sim.simulate_tx(sample_tx("a", 500, 50000));
        let r2 = sim.simulate_tx(sample_tx("b", 100, 5000)); // reverted
        sim.store_result(r1).unwrap();
        sim.store_result(r2).unwrap();

        let stats = sim.stats();
        assert_eq!(stats.total_simulations, 2);
        assert_eq!(stats.successful, 1);
        assert_eq!(stats.reverted, 1);
        assert_eq!(stats.failed, 0);
        assert_eq!(stats.total_forks, 1);
        assert_eq!(stats.total_scenarios, 1);
        assert!(stats.avg_gas_used > 0.0);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path("roundtrip");
        let mut sim = TxSimulator::new();
        sim.create_fork("f1", ForkSource::SpecificBlock(42), 42)
            .unwrap();
        let r = sim.simulate_tx(sample_tx("tx1", 500, 50000));
        sim.store_result(r).unwrap();
        sim.save(&path).unwrap();

        let loaded = TxSimulator::load(&path).unwrap();
        assert!(loaded.simulations.contains_key("tx1"));
        assert!(loaded.forks.contains_key("f1"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = test_path("nonexistent");
        let _ = std::fs::remove_file(&path);
        let sim = TxSimulator::load_or_default(&path);
        assert!(sim.simulations.is_empty());
    }
}
