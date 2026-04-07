use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum BatchExecutorError {
    #[error("Batch not found: {0}")]
    BatchNotFound(String),
    #[error("Transaction not found: {0}")]
    TxNotFound(String),
    #[error("Batch not in Draft status: {0}")]
    NotDraft(String),
    #[error("Batch not executable: {0}")]
    NotExecutable(String),
    #[error("Dependency not found: tx {0} depends on {1}")]
    DependencyNotFound(String, String),
    #[error("Batch already executed: {0}")]
    AlreadyExecuted(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BatchStatus {
    Draft,
    Validating,
    Executing,
    Completed,
    PartialFailure,
    RolledBack,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TxStatus {
    Pending,
    Success,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RollbackPolicy {
    None,
    StopOnFailure,
    RollbackAll,
    ContinueOnFailure,
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchTx {
    pub id: String,
    pub description: String,
    pub tx_type: String,
    pub to: String,
    pub amount: u64,
    pub token: String,
    pub status: TxStatus,
    pub tx_hash: Option<String>,
    pub error: Option<String>,
    pub gas_used: Option<u64>,
    pub order: u32,
    pub depends_on: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchJob {
    pub id: String,
    pub name: String,
    pub transactions: Vec<BatchTx>,
    pub status: BatchStatus,
    pub rollback_policy: RollbackPolicy,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub total_gas: u64,
    pub success_count: u32,
    pub failure_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResult {
    pub batch_id: String,
    pub status: BatchStatus,
    pub completed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub total_gas: u64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchStats {
    pub total_batches: usize,
    pub completed_batches: usize,
    pub failed_batches: usize,
    pub total_transactions: usize,
    pub total_gas_used: u64,
    pub avg_batch_size: f64,
    pub success_rate: f64,
}

// ---------------------------------------------------------------------------
// Main Store
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BatchExecutor {
    pub jobs: HashMap<String, BatchJob>,
    pub results: Vec<BatchResult>,
}

impl BatchExecutor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new Draft batch and return its ID.
    pub fn create_batch(&mut self, name: &str, policy: RollbackPolicy) -> String {
        let id = format!("batch-{}", self.jobs.len() + 1);
        let job = BatchJob {
            id: id.clone(),
            name: name.to_string(),
            transactions: Vec::new(),
            status: BatchStatus::Draft,
            rollback_policy: policy,
            created_at: chrono::Utc::now().to_rfc3339(),
            started_at: None,
            completed_at: None,
            total_gas: 0,
            success_count: 0,
            failure_count: 0,
        };
        self.jobs.insert(id.clone(), job);
        id
    }

    /// Add a transaction to a Draft batch.
    pub fn add_tx(&mut self, batch_id: &str, tx: BatchTx) -> Result<(), BatchExecutorError> {
        let job = self
            .jobs
            .get_mut(batch_id)
            .ok_or_else(|| BatchExecutorError::BatchNotFound(batch_id.to_string()))?;
        if job.status != BatchStatus::Draft {
            return Err(BatchExecutorError::NotDraft(batch_id.to_string()));
        }
        job.transactions.push(tx);
        Ok(())
    }

    /// Remove a transaction from a Draft batch, returning it.
    pub fn remove_tx(
        &mut self,
        batch_id: &str,
        tx_id: &str,
    ) -> Result<BatchTx, BatchExecutorError> {
        let job = self
            .jobs
            .get_mut(batch_id)
            .ok_or_else(|| BatchExecutorError::BatchNotFound(batch_id.to_string()))?;
        if job.status != BatchStatus::Draft {
            return Err(BatchExecutorError::NotDraft(batch_id.to_string()));
        }
        let idx = job
            .transactions
            .iter()
            .position(|t| t.id == tx_id)
            .ok_or_else(|| BatchExecutorError::TxNotFound(tx_id.to_string()))?;
        Ok(job.transactions.remove(idx))
    }

    /// Reorder a transaction within a Draft batch.
    pub fn reorder_tx(
        &mut self,
        batch_id: &str,
        tx_id: &str,
        new_order: u32,
    ) -> Result<(), BatchExecutorError> {
        let job = self
            .jobs
            .get_mut(batch_id)
            .ok_or_else(|| BatchExecutorError::BatchNotFound(batch_id.to_string()))?;
        if job.status != BatchStatus::Draft {
            return Err(BatchExecutorError::NotDraft(batch_id.to_string()));
        }
        let tx = job
            .transactions
            .iter_mut()
            .find(|t| t.id == tx_id)
            .ok_or_else(|| BatchExecutorError::TxNotFound(tx_id.to_string()))?;
        tx.order = new_order;
        Ok(())
    }

    /// Validate a batch: check dependencies exist, return warnings, set Validating.
    pub fn validate_batch(
        &mut self,
        batch_id: &str,
    ) -> Result<Vec<String>, BatchExecutorError> {
        let job = self
            .jobs
            .get(batch_id)
            .ok_or_else(|| BatchExecutorError::BatchNotFound(batch_id.to_string()))?;
        if job.status != BatchStatus::Draft {
            return Err(BatchExecutorError::NotDraft(batch_id.to_string()));
        }

        let tx_ids: Vec<String> = job.transactions.iter().map(|t| t.id.clone()).collect();
        let mut warnings = Vec::new();

        for tx in &job.transactions {
            if let Some(ref dep) = tx.depends_on {
                if !tx_ids.contains(dep) {
                    return Err(BatchExecutorError::DependencyNotFound(
                        tx.id.clone(),
                        dep.clone(),
                    ));
                }
            }
            if tx.amount == 0 {
                warnings.push(format!("Transaction {} has zero amount", tx.id));
            }
        }

        if job.transactions.is_empty() {
            warnings.push("Batch has no transactions".to_string());
        }

        let job = self.jobs.get_mut(batch_id).unwrap();
        job.status = BatchStatus::Validating;

        Ok(warnings)
    }

    /// Simulate execution of a batch.
    pub fn execute_batch(
        &mut self,
        batch_id: &str,
    ) -> Result<BatchResult, BatchExecutorError> {
        let job = self
            .jobs
            .get(batch_id)
            .ok_or_else(|| BatchExecutorError::BatchNotFound(batch_id.to_string()))?;
        if job.status != BatchStatus::Draft && job.status != BatchStatus::Validating {
            return Err(BatchExecutorError::NotExecutable(batch_id.to_string()));
        }

        let start = std::time::Instant::now();
        let job = self.jobs.get_mut(batch_id).unwrap();
        job.status = BatchStatus::Executing;
        job.started_at = Some(chrono::Utc::now().to_rfc3339());

        // Sort transactions by order for execution
        job.transactions.sort_by_key(|t| t.order);

        let mut success_count: u32 = 0;
        let mut failure_count: u32 = 0;
        let mut skipped_count: usize = 0;
        let mut total_gas: u64 = 0;
        let mut failed_ids: Vec<String> = Vec::new();
        let mut stop = false;

        let policy = job.rollback_policy.clone();
        let tx_count = job.transactions.len();

        for i in 0..tx_count {
            if stop {
                job.transactions[i].status = TxStatus::Skipped;
                skipped_count += 1;
                continue;
            }

            // Check if dependency failed
            let dep_failed = job.transactions[i]
                .depends_on
                .as_ref()
                .map(|dep_id| failed_ids.contains(dep_id))
                .unwrap_or(false);
            if dep_failed {
                let dep_id = job.transactions[i].depends_on.clone().unwrap();
                job.transactions[i].status = TxStatus::Skipped;
                job.transactions[i].error =
                    Some(format!("Dependency {} failed", dep_id));
                skipped_count += 1;
                continue;
            }

            // Simple rule: amount > 0 means success
            let gas = 21000_u64;
            if job.transactions[i].amount > 0 {
                job.transactions[i].status = TxStatus::Success;
                job.transactions[i].tx_hash =
                    Some(format!("0xhash_{}", job.transactions[i].id));
                job.transactions[i].gas_used = Some(gas);
                total_gas += gas;
                success_count += 1;
            } else {
                job.transactions[i].status = TxStatus::Failed;
                job.transactions[i].error = Some("Zero amount".to_string());
                failure_count += 1;
                failed_ids.push(job.transactions[i].id.clone());

                match policy {
                    RollbackPolicy::StopOnFailure => {
                        stop = true;
                    }
                    RollbackPolicy::RollbackAll => {
                        // Mark all previously successful txs as Skipped
                        for j in 0..i {
                            if job.transactions[j].status == TxStatus::Success {
                                job.transactions[j].status = TxStatus::Skipped;
                                success_count -= 1;
                                skipped_count += 1;
                            }
                        }
                        stop = true;
                    }
                    RollbackPolicy::ContinueOnFailure | RollbackPolicy::None => {}
                }
            }
        }

        job.total_gas = total_gas;
        job.success_count = success_count;
        job.failure_count = failure_count;
        job.completed_at = Some(chrono::Utc::now().to_rfc3339());

        if failure_count == 0 {
            job.status = BatchStatus::Completed;
        } else if success_count > 0 {
            job.status = BatchStatus::PartialFailure;
        } else if policy == RollbackPolicy::RollbackAll {
            job.status = BatchStatus::RolledBack;
        } else {
            job.status = BatchStatus::Failed;
        }

        let result = BatchResult {
            batch_id: batch_id.to_string(),
            status: job.status.clone(),
            completed: success_count as usize,
            failed: failure_count as usize,
            skipped: skipped_count,
            total_gas,
            duration_ms: start.elapsed().as_millis() as u64,
        };

        self.results.push(result.clone());
        Ok(result)
    }

    /// Rollback a batch: set RolledBack, mark all txs Skipped.
    pub fn rollback_batch(&mut self, batch_id: &str) -> Result<(), BatchExecutorError> {
        let job = self
            .jobs
            .get_mut(batch_id)
            .ok_or_else(|| BatchExecutorError::BatchNotFound(batch_id.to_string()))?;
        job.status = BatchStatus::RolledBack;
        for tx in &mut job.transactions {
            tx.status = TxStatus::Skipped;
        }
        Ok(())
    }

    pub fn get_batch(&self, id: &str) -> Option<&BatchJob> {
        self.jobs.get(id)
    }

    pub fn get_batch_mut(&mut self, id: &str) -> Option<&mut BatchJob> {
        self.jobs.get_mut(id)
    }

    /// Return all batches sorted by created_at descending.
    pub fn batch_history(&self) -> Vec<&BatchJob> {
        let mut jobs: Vec<&BatchJob> = self.jobs.values().collect();
        jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        jobs
    }

    /// Return batches that are Draft or Validating.
    pub fn pending_batches(&self) -> Vec<&BatchJob> {
        self.jobs
            .values()
            .filter(|j| j.status == BatchStatus::Draft || j.status == BatchStatus::Validating)
            .collect()
    }

    /// Estimate gas for a batch (21000 per tx base).
    pub fn estimate_gas(&self, batch_id: &str) -> Result<u64, BatchExecutorError> {
        let job = self
            .jobs
            .get(batch_id)
            .ok_or_else(|| BatchExecutorError::BatchNotFound(batch_id.to_string()))?;
        Ok(job.transactions.len() as u64 * 21000)
    }

    /// Compute aggregate stats.
    pub fn stats(&self) -> BatchStats {
        let total_batches = self.jobs.len();
        let completed_batches = self
            .jobs
            .values()
            .filter(|j| j.status == BatchStatus::Completed)
            .count();
        let failed_batches = self
            .jobs
            .values()
            .filter(|j| {
                j.status == BatchStatus::Failed
                    || j.status == BatchStatus::PartialFailure
                    || j.status == BatchStatus::RolledBack
            })
            .count();
        let total_transactions: usize =
            self.jobs.values().map(|j| j.transactions.len()).sum();
        let total_gas_used: u64 = self.jobs.values().map(|j| j.total_gas).sum();
        let avg_batch_size = if total_batches > 0 {
            total_transactions as f64 / total_batches as f64
        } else {
            0.0
        };
        let success_rate = if total_batches > 0 {
            completed_batches as f64 / total_batches as f64
        } else {
            0.0
        };

        BatchStats {
            total_batches,
            completed_batches,
            failed_batches,
            total_transactions,
            total_gas_used,
            avg_batch_size,
            success_rate,
        }
    }

    // -- Persistence --------------------------------------------------------

    pub fn load(path: &Path) -> Result<Self, BatchExecutorError> {
        let data = std::fs::read_to_string(path)?;
        let store: Self = serde_json::from_str(&data)?;
        Ok(store)
    }

    pub fn save(&self, path: &Path) -> Result<(), BatchExecutorError> {
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(path, data)?;
        Ok(())
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
    use std::path::PathBuf;

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("evaporchain_batch_executor_test_{}_{}", std::process::id(), name))
    }

    fn sample_tx(id: &str, amount: u64, order: u32) -> BatchTx {
        BatchTx {
            id: id.to_string(),
            description: format!("Test tx {}", id),
            tx_type: "transfer".to_string(),
            to: "0xRecipient".to_string(),
            amount,
            token: "EVAP".to_string(),
            status: TxStatus::Pending,
            tx_hash: None,
            error: None,
            gas_used: None,
            order,
            depends_on: None,
        }
    }

    #[test]
    fn test_create_batch() {
        let mut exec = BatchExecutor::new();
        let id = exec.create_batch("Test Batch", RollbackPolicy::None);
        assert!(!id.is_empty());
        let job = exec.get_batch(&id).unwrap();
        assert_eq!(job.name, "Test Batch");
        assert_eq!(job.status, BatchStatus::Draft);
    }

    #[test]
    fn test_add_tx() {
        let mut exec = BatchExecutor::new();
        let id = exec.create_batch("B1", RollbackPolicy::None);
        let tx = sample_tx("tx1", 100, 0);
        assert!(exec.add_tx(&id, tx).is_ok());
        assert_eq!(exec.get_batch(&id).unwrap().transactions.len(), 1);
    }

    #[test]
    fn test_add_tx_not_draft() {
        let mut exec = BatchExecutor::new();
        let id = exec.create_batch("B1", RollbackPolicy::None);
        exec.add_tx(&id, sample_tx("tx1", 100, 0)).unwrap();
        exec.validate_batch(&id).unwrap();
        let result = exec.add_tx(&id, sample_tx("tx2", 200, 1));
        assert!(result.is_err());
    }

    #[test]
    fn test_add_tx_batch_not_found() {
        let mut exec = BatchExecutor::new();
        let result = exec.add_tx("nonexistent", sample_tx("tx1", 100, 0));
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_tx() {
        let mut exec = BatchExecutor::new();
        let id = exec.create_batch("B1", RollbackPolicy::None);
        exec.add_tx(&id, sample_tx("tx1", 100, 0)).unwrap();
        exec.add_tx(&id, sample_tx("tx2", 200, 1)).unwrap();
        let removed = exec.remove_tx(&id, "tx1").unwrap();
        assert_eq!(removed.id, "tx1");
        assert_eq!(exec.get_batch(&id).unwrap().transactions.len(), 1);
    }

    #[test]
    fn test_remove_tx_not_found() {
        let mut exec = BatchExecutor::new();
        let id = exec.create_batch("B1", RollbackPolicy::None);
        let result = exec.remove_tx(&id, "nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_reorder_tx() {
        let mut exec = BatchExecutor::new();
        let id = exec.create_batch("B1", RollbackPolicy::None);
        exec.add_tx(&id, sample_tx("tx1", 100, 0)).unwrap();
        exec.reorder_tx(&id, "tx1", 5).unwrap();
        let job = exec.get_batch(&id).unwrap();
        assert_eq!(job.transactions[0].order, 5);
    }

    #[test]
    fn test_validate_batch_ok() {
        let mut exec = BatchExecutor::new();
        let id = exec.create_batch("B1", RollbackPolicy::None);
        exec.add_tx(&id, sample_tx("tx1", 100, 0)).unwrap();
        let warnings = exec.validate_batch(&id).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(exec.get_batch(&id).unwrap().status, BatchStatus::Validating);
    }

    #[test]
    fn test_validate_batch_zero_amount_warning() {
        let mut exec = BatchExecutor::new();
        let id = exec.create_batch("B1", RollbackPolicy::None);
        exec.add_tx(&id, sample_tx("tx1", 0, 0)).unwrap();
        let warnings = exec.validate_batch(&id).unwrap();
        assert!(!warnings.is_empty());
    }

    #[test]
    fn test_validate_empty_batch_warning() {
        let mut exec = BatchExecutor::new();
        let id = exec.create_batch("B1", RollbackPolicy::None);
        let warnings = exec.validate_batch(&id).unwrap();
        assert!(warnings.iter().any(|w| w.contains("no transactions")));
    }

    #[test]
    fn test_validate_dependency_missing() {
        let mut exec = BatchExecutor::new();
        let id = exec.create_batch("B1", RollbackPolicy::None);
        let mut tx = sample_tx("tx1", 100, 0);
        tx.depends_on = Some("nonexistent".to_string());
        exec.add_tx(&id, tx).unwrap();
        let result = exec.validate_batch(&id);
        assert!(result.is_err());
    }

    #[test]
    fn test_execute_batch_all_success() {
        let mut exec = BatchExecutor::new();
        let id = exec.create_batch("B1", RollbackPolicy::None);
        exec.add_tx(&id, sample_tx("tx1", 100, 0)).unwrap();
        exec.add_tx(&id, sample_tx("tx2", 200, 1)).unwrap();
        let result = exec.execute_batch(&id).unwrap();
        assert_eq!(result.status, BatchStatus::Completed);
        assert_eq!(result.completed, 2);
        assert_eq!(result.failed, 0);
    }

    #[test]
    fn test_execute_batch_partial_failure() {
        let mut exec = BatchExecutor::new();
        let id = exec.create_batch("B1", RollbackPolicy::ContinueOnFailure);
        exec.add_tx(&id, sample_tx("tx1", 100, 0)).unwrap();
        exec.add_tx(&id, sample_tx("tx2", 0, 1)).unwrap(); // will fail
        let result = exec.execute_batch(&id).unwrap();
        assert_eq!(result.status, BatchStatus::PartialFailure);
        assert_eq!(result.completed, 1);
        assert_eq!(result.failed, 1);
    }

    #[test]
    fn test_execute_stop_on_failure() {
        let mut exec = BatchExecutor::new();
        let id = exec.create_batch("B1", RollbackPolicy::StopOnFailure);
        exec.add_tx(&id, sample_tx("tx1", 0, 0)).unwrap(); // fail
        exec.add_tx(&id, sample_tx("tx2", 100, 1)).unwrap(); // should be skipped
        let result = exec.execute_batch(&id).unwrap();
        assert_eq!(result.failed, 1);
        assert_eq!(result.skipped, 1);
    }

    #[test]
    fn test_execute_rollback_all() {
        let mut exec = BatchExecutor::new();
        let id = exec.create_batch("B1", RollbackPolicy::RollbackAll);
        exec.add_tx(&id, sample_tx("tx1", 100, 0)).unwrap(); // success then rolled back
        exec.add_tx(&id, sample_tx("tx2", 0, 1)).unwrap(); // fail
        let result = exec.execute_batch(&id).unwrap();
        assert_eq!(result.status, BatchStatus::RolledBack);
        assert_eq!(result.completed, 0);
    }

    #[test]
    fn test_execute_dependency_skip() {
        let mut exec = BatchExecutor::new();
        let id = exec.create_batch("B1", RollbackPolicy::ContinueOnFailure);
        exec.add_tx(&id, sample_tx("tx1", 0, 0)).unwrap(); // fail
        let mut tx2 = sample_tx("tx2", 100, 1);
        tx2.depends_on = Some("tx1".to_string());
        exec.add_tx(&id, tx2).unwrap();
        let result = exec.execute_batch(&id).unwrap();
        assert_eq!(result.skipped, 1);
    }

    #[test]
    fn test_rollback_batch() {
        let mut exec = BatchExecutor::new();
        let id = exec.create_batch("B1", RollbackPolicy::None);
        exec.add_tx(&id, sample_tx("tx1", 100, 0)).unwrap();
        exec.execute_batch(&id).unwrap();
        exec.rollback_batch(&id).unwrap();
        let job = exec.get_batch(&id).unwrap();
        assert_eq!(job.status, BatchStatus::RolledBack);
        assert!(job.transactions.iter().all(|t| t.status == TxStatus::Skipped));
    }

    #[test]
    fn test_batch_history() {
        let mut exec = BatchExecutor::new();
        exec.create_batch("First", RollbackPolicy::None);
        exec.create_batch("Second", RollbackPolicy::None);
        let history = exec.batch_history();
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn test_pending_batches() {
        let mut exec = BatchExecutor::new();
        let id1 = exec.create_batch("Draft", RollbackPolicy::None);
        let id2 = exec.create_batch("Executed", RollbackPolicy::None);
        exec.add_tx(&id2, sample_tx("tx1", 100, 0)).unwrap();
        exec.execute_batch(&id2).unwrap();
        let pending = exec.pending_batches();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, id1);
    }

    #[test]
    fn test_estimate_gas() {
        let mut exec = BatchExecutor::new();
        let id = exec.create_batch("B1", RollbackPolicy::None);
        exec.add_tx(&id, sample_tx("tx1", 100, 0)).unwrap();
        exec.add_tx(&id, sample_tx("tx2", 200, 1)).unwrap();
        let gas = exec.estimate_gas(&id).unwrap();
        assert_eq!(gas, 42000);
    }

    #[test]
    fn test_stats() {
        let mut exec = BatchExecutor::new();
        let id = exec.create_batch("B1", RollbackPolicy::None);
        exec.add_tx(&id, sample_tx("tx1", 100, 0)).unwrap();
        exec.execute_batch(&id).unwrap();
        let stats = exec.stats();
        assert_eq!(stats.total_batches, 1);
        assert_eq!(stats.completed_batches, 1);
        assert_eq!(stats.total_transactions, 1);
        assert!(stats.success_rate > 0.0);
    }

    #[test]
    fn test_save_and_load() {
        let path = tmp_path("save_load.json");
        let mut exec = BatchExecutor::new();
        let id = exec.create_batch("B1", RollbackPolicy::None);
        exec.add_tx(&id, sample_tx("tx1", 100, 0)).unwrap();
        exec.save(&path).unwrap();

        let loaded = BatchExecutor::load(&path).unwrap();
        assert_eq!(loaded.jobs.len(), 1);
        assert!(loaded.get_batch(&id).is_some());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing() {
        let path = tmp_path("nonexistent.json");
        let exec = BatchExecutor::load_or_default(&path);
        assert!(exec.jobs.is_empty());
    }

    #[test]
    fn test_get_batch_mut() {
        let mut exec = BatchExecutor::new();
        let id = exec.create_batch("B1", RollbackPolicy::None);
        let job = exec.get_batch_mut(&id).unwrap();
        job.name = "Renamed".to_string();
        assert_eq!(exec.get_batch(&id).unwrap().name, "Renamed");
    }
}
