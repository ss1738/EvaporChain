//! Transaction batching — execute multiple operations from a JSON file.
//!
//! Reads a batch file, validates all entries, shows a summary,
//! then executes sequentially with automatic nonce management.
//!
//! # Batch File Format
//!
//! ```json
//! {
//!   "operations": [
//!     { "type": "transfer", "to": "0xabc...", "amount": 1000 },
//!     { "type": "transfer", "to": "0xdef...", "amount": 2000 },
//!     { "type": "refresh", "object_id": "0x123...", "energy": 500 }
//!   ]
//! }
//! ```

use serde::{Deserialize, Serialize};
use std::path::Path;

// ──────────────────────────── Types ──────────────────────────────────────

/// A batch file containing multiple operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchFile {
    pub operations: Vec<BatchOperation>,
}

/// A single operation in a batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum BatchOperation {
    Transfer { to: String, amount: u64 },
    Refresh { object_id: String, energy: u64 },
}

/// Result of executing a single operation.
#[derive(Debug, Clone, Serialize)]
pub struct BatchResult {
    pub index: usize,
    pub operation: String,
    pub success: bool,
    pub tx_hash: Option<String>,
    pub error: Option<String>,
}

/// Summary of a batch execution.
#[derive(Debug, Clone, Serialize)]
pub struct BatchSummary {
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub results: Vec<BatchResult>,
}

// ──────────────────────────── Loading ────────────────────────────────────

impl BatchFile {
    /// Load a batch file from disk.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let data = std::fs::read_to_string(path.as_ref())
            .map_err(|e| format!("Cannot read batch file: {}", e))?;
        let batch: BatchFile =
            serde_json::from_str(&data).map_err(|e| format!("Invalid batch JSON: {}", e))?;

        if batch.operations.is_empty() {
            return Err("Batch file has no operations".to_string());
        }

        Ok(batch)
    }

    /// Validate all operations in the batch.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        for (i, op) in self.operations.iter().enumerate() {
            match op {
                BatchOperation::Transfer { to, amount } => {
                    if let Err(e) = crate::validation::validate_recipient(to) {
                        errors.push(format!("Op #{}: {}", i + 1, e));
                    }
                    if let Err(e) = crate::validation::validate_amount(*amount) {
                        errors.push(format!("Op #{}: {}", i + 1, e));
                    }
                }
                BatchOperation::Refresh { object_id, energy } => {
                    if let Err(e) = crate::validation::validate_address(object_id) {
                        errors.push(format!("Op #{}: {}", i + 1, e));
                    }
                    if let Err(e) = crate::validation::validate_energy(*energy) {
                        errors.push(format!("Op #{}: {}", i + 1, e));
                    }
                }
            }
        }

        errors
    }

    /// Get a human-readable summary of what will be executed.
    pub fn summary(&self) -> String {
        let transfers = self
            .operations
            .iter()
            .filter(|op| matches!(op, BatchOperation::Transfer { .. }))
            .count();
        let refreshes = self
            .operations
            .iter()
            .filter(|op| matches!(op, BatchOperation::Refresh { .. }))
            .count();
        let total_amount: u64 = self
            .operations
            .iter()
            .filter_map(|op| {
                if let BatchOperation::Transfer { amount, .. } = op {
                    Some(*amount)
                } else {
                    None
                }
            })
            .sum();
        let total_energy: u64 = self
            .operations
            .iter()
            .filter_map(|op| {
                if let BatchOperation::Refresh { energy, .. } = op {
                    Some(*energy)
                } else {
                    None
                }
            })
            .sum();

        let mut parts = Vec::new();
        if transfers > 0 {
            parts.push(format!(
                "{} transfers (total {} EVAP)",
                transfers, total_amount
            ));
        }
        if refreshes > 0 {
            parts.push(format!(
                "{} refreshes (total {} energy)",
                refreshes, total_energy
            ));
        }
        format!("{} operations: {}", self.operations.len(), parts.join(", "))
    }
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_batch() -> BatchFile {
        BatchFile {
            operations: vec![
                BatchOperation::Transfer {
                    to: format!("0x{}", "ab".repeat(32)),
                    amount: 1000,
                },
                BatchOperation::Transfer {
                    to: format!("0x{}", "cd".repeat(32)),
                    amount: 2000,
                },
                BatchOperation::Refresh {
                    object_id: format!("0x{}", "ef".repeat(32)),
                    energy: 500,
                },
            ],
        }
    }

    #[test]
    fn test_json_roundtrip() {
        let batch = sample_batch();
        let json = serde_json::to_string_pretty(&batch).unwrap();
        let loaded: BatchFile = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.operations.len(), 3);
    }

    #[test]
    fn test_validate_valid_batch() {
        let batch = sample_batch();
        let errors = batch.validate();
        assert!(errors.is_empty(), "Expected no errors, got: {:?}", errors);
    }

    #[test]
    fn test_validate_bad_address() {
        let batch = BatchFile {
            operations: vec![BatchOperation::Transfer {
                to: "0xinvalid".to_string(),
                amount: 100,
            }],
        };
        let errors = batch.validate();
        assert!(!errors.is_empty());
    }

    #[test]
    fn test_validate_zero_amount() {
        let batch = BatchFile {
            operations: vec![BatchOperation::Transfer {
                to: format!("0x{}", "ab".repeat(32)),
                amount: 0,
            }],
        };
        let errors = batch.validate();
        assert!(!errors.is_empty());
    }

    #[test]
    fn test_validate_zero_energy() {
        let batch = BatchFile {
            operations: vec![BatchOperation::Refresh {
                object_id: format!("0x{}", "ab".repeat(32)),
                energy: 0,
            }],
        };
        let errors = batch.validate();
        assert!(!errors.is_empty());
    }

    #[test]
    fn test_summary() {
        let batch = sample_batch();
        let summary = batch.summary();
        assert!(summary.contains("3 operations"));
        assert!(summary.contains("2 transfers"));
        assert!(summary.contains("1 refreshes"));
        assert!(summary.contains("3000 EVAP"));
        assert!(summary.contains("500 energy"));
    }

    #[test]
    fn test_summary_transfers_only() {
        let batch = BatchFile {
            operations: vec![BatchOperation::Transfer {
                to: format!("0x{}", "ab".repeat(32)),
                amount: 100,
            }],
        };
        let summary = batch.summary();
        assert!(summary.contains("1 operations"));
        assert!(summary.contains("1 transfers"));
        assert!(!summary.contains("refreshes"));
    }

    #[test]
    fn test_file_save_and_load() {
        let batch = sample_batch();
        let dir = std::env::temp_dir().join("evaporchain_batch_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("batch.json");

        let json = serde_json::to_string_pretty(&batch).unwrap();
        std::fs::write(&path, &json).unwrap();

        let loaded = BatchFile::load(&path).unwrap();
        assert_eq!(loaded.operations.len(), 3);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_load_missing_file() {
        let result = BatchFile::load("/tmp/nonexistent_batch_xyz.json");
        assert!(result.is_err());
    }

    #[test]
    fn test_load_empty_operations() {
        let dir = std::env::temp_dir().join("evaporchain_batch_empty_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("empty.json");
        std::fs::write(&path, r#"{"operations":[]}"#).unwrap();

        let result = BatchFile::load(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no operations"));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_batch_result_serializable() {
        let r = BatchResult {
            index: 0,
            operation: "transfer".to_string(),
            success: true,
            tx_hash: Some("0xhash".to_string()),
            error: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"success\":true"));
    }

    #[test]
    fn test_batch_summary_serializable() {
        let s = BatchSummary {
            total: 2,
            succeeded: 1,
            failed: 1,
            results: vec![],
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"total\":2"));
    }
}
