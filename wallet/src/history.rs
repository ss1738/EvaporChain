//! Transaction history persistence.
//!
//! Records submitted transactions locally so users can review past activity
//! without querying the node. Persisted to JSON with append-friendly design.

use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum HistoryError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ──────────────────────────── History Entry ───────────────────────────

/// Outcome of a submitted transaction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TxOutcome {
    /// Transaction accepted by the node (hash returned).
    Accepted,
    /// Transaction rejected by the node.
    Rejected,
    /// Confirmed on-chain.
    Confirmed,
    /// Unknown / pending.
    Pending,
}

/// A recorded transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Transaction hash (from node response), if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    /// Transaction type (e.g., "Transfer", "CreateObject").
    pub tx_type: String,
    /// Sender address (hex).
    pub from: String,
    /// Recipient or target address (hex), if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// Amount transferred, if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<u64>,
    /// Outcome of the submission.
    pub outcome: TxOutcome,
    /// Error message if rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Timestamp when submitted (ISO 8601).
    pub submitted_at: String,
    /// Optional note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

// ──────────────────────────── TxHistory ───────────────────────────────

/// Persistent transaction history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxHistory {
    pub version: u32,
    pub entries: Vec<HistoryEntry>,
}

impl TxHistory {
    /// Create a new empty history.
    pub fn new() -> Self {
        Self {
            version: 1,
            entries: Vec::new(),
        }
    }

    /// Load from file, or create empty if not found.
    pub fn load_or_empty<P: AsRef<Path>>(path: P) -> Result<Self, HistoryError> {
        let path = path.as_ref();
        if path.exists() {
            let data = std::fs::read_to_string(path)?;
            let history: TxHistory = serde_json::from_str(&data)?;
            Ok(history)
        } else {
            Ok(Self::new())
        }
    }

    /// Save to file.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), HistoryError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Record a successful transaction.
    pub fn record_success(
        &mut self,
        tx_type: &str,
        from: &str,
        to: Option<&str>,
        amount: Option<u64>,
        tx_hash: &str,
    ) {
        self.entries.push(HistoryEntry {
            tx_hash: Some(tx_hash.to_string()),
            tx_type: tx_type.to_string(),
            from: from.to_string(),
            to: to.map(|s| s.to_string()),
            amount,
            outcome: TxOutcome::Accepted,
            error: None,
            submitted_at: chrono::Utc::now().to_rfc3339(),
            note: None,
        });
    }

    /// Record a failed transaction.
    pub fn record_failure(&mut self, tx_type: &str, from: &str, error: &str) {
        self.entries.push(HistoryEntry {
            tx_hash: None,
            tx_type: tx_type.to_string(),
            from: from.to_string(),
            to: None,
            amount: None,
            outcome: TxOutcome::Rejected,
            error: Some(error.to_string()),
            submitted_at: chrono::Utc::now().to_rfc3339(),
            note: None,
        });
    }

    /// Get all entries for a given address.
    pub fn for_address(&self, address: &str) -> Vec<&HistoryEntry> {
        self.entries
            .iter()
            .filter(|e| e.from == address || e.to.as_deref() == Some(address))
            .collect()
    }

    /// Get the last N entries.
    pub fn recent(&self, n: usize) -> &[HistoryEntry] {
        let start = self.entries.len().saturating_sub(n);
        &self.entries[start..]
    }

    /// Total number of recorded transactions.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the history is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clear all history.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Export history as CSV string.
    pub fn to_csv(&self) -> String {
        let mut csv = String::from("timestamp,type,from,to,amount,status,tx_hash,error\n");
        for e in &self.entries {
            let status = match e.outcome {
                TxOutcome::Accepted => "accepted",
                TxOutcome::Confirmed => "confirmed",
                TxOutcome::Rejected => "rejected",
                TxOutcome::Pending => "pending",
            };
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{}\n",
                e.submitted_at,
                e.tx_type,
                e.from,
                e.to.as_deref().unwrap_or(""),
                e.amount.map(|a| a.to_string()).unwrap_or_default(),
                status,
                e.tx_hash.as_deref().unwrap_or(""),
                e.error.as_deref().unwrap_or(""),
            ));
        }
        csv
    }

    /// Export history as CSV to a file.
    pub fn export_csv<P: AsRef<std::path::Path>>(&self, path: P) -> Result<(), HistoryError> {
        std::fs::write(path, self.to_csv())?;
        Ok(())
    }
}

impl Default for TxHistory {
    fn default() -> Self {
        Self::new()
    }
}

// ──────────────────────────── Tests ──────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_success() {
        let mut h = TxHistory::new();
        h.record_success("Transfer", "0xaaa", Some("0xbbb"), Some(1000), "0xhash1");
        assert_eq!(h.len(), 1);
        assert_eq!(h.entries[0].tx_type, "Transfer");
        assert_eq!(h.entries[0].outcome, TxOutcome::Accepted);
        assert_eq!(h.entries[0].tx_hash.as_deref(), Some("0xhash1"));
    }

    #[test]
    fn test_record_failure() {
        let mut h = TxHistory::new();
        h.record_failure("CreateObject", "0xaaa", "insufficient balance");
        assert_eq!(h.len(), 1);
        assert_eq!(h.entries[0].outcome, TxOutcome::Rejected);
        assert_eq!(h.entries[0].error.as_deref(), Some("insufficient balance"));
    }

    #[test]
    fn test_for_address() {
        let mut h = TxHistory::new();
        h.record_success("Transfer", "0xaaa", Some("0xbbb"), Some(100), "0xh1");
        h.record_success("Transfer", "0xbbb", Some("0xccc"), Some(200), "0xh2");
        h.record_success("Transfer", "0xccc", Some("0xaaa"), Some(300), "0xh3");

        let aaa = h.for_address("0xaaa");
        assert_eq!(aaa.len(), 2); // sent and received
    }

    #[test]
    fn test_recent() {
        let mut h = TxHistory::new();
        for i in 0..10 {
            h.record_success("Transfer", "0xaaa", None, Some(i), &format!("0xh{}", i));
        }
        let last3 = h.recent(3);
        assert_eq!(last3.len(), 3);
        assert_eq!(last3[0].amount, Some(7));
    }

    #[test]
    fn test_clear() {
        let mut h = TxHistory::new();
        h.record_success("Transfer", "0xaaa", None, None, "0xh1");
        assert_eq!(h.len(), 1);
        h.clear();
        assert_eq!(h.len(), 0);
    }

    #[test]
    fn test_json_roundtrip() {
        let mut h = TxHistory::new();
        h.record_success("Transfer", "0xaaa", Some("0xbbb"), Some(500), "0xh1");
        h.record_failure("Refresh", "0xaaa", "object not found");

        let json = serde_json::to_string_pretty(&h).unwrap();
        let loaded: TxHistory = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.entries[0].outcome, TxOutcome::Accepted);
        assert_eq!(loaded.entries[1].outcome, TxOutcome::Rejected);
    }

    #[test]
    fn test_file_save_and_load() {
        let dir = std::env::temp_dir().join("evaporchain_history_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.json");

        let mut h = TxHistory::new();
        h.record_success("Transfer", "0xaaa", Some("0xbbb"), Some(100), "0xh1");
        h.save(&path).unwrap();

        let loaded = TxHistory::load_or_empty(&path).unwrap();
        assert_eq!(loaded.len(), 1);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_load_missing_file_returns_empty() {
        let path = std::env::temp_dir().join("nonexistent_history_xyz.json");
        let h = TxHistory::load_or_empty(&path).unwrap();
        assert_eq!(h.len(), 0);
    }

    // ─── Additional coverage tests (session 62) ───────────────────────────────

    #[test]
    fn test_is_empty_reflects_entries() {
        let mut h = TxHistory::new();
        assert!(h.is_empty());
        h.record_success("Transfer", "0xaaa", None, None, "0xh1");
        assert!(!h.is_empty());
    }

    #[test]
    fn test_default_creates_empty_history() {
        let h = TxHistory::default();
        assert_eq!(h.len(), 0);
        assert_eq!(h.version, 1);
    }

    #[test]
    fn test_to_csv_header_and_all_outcome_variants() {
        let mut h = TxHistory::new();
        h.record_success("Transfer", "0xaaa", Some("0xbbb"), Some(100), "0xh1");
        h.record_failure("Refresh", "0xaaa", "object not found");
        h.entries.push(HistoryEntry {
            tx_hash: Some("0xh3".into()),
            tx_type: "Transfer".into(),
            from: "0xccc".into(),
            to: None,
            amount: None,
            outcome: TxOutcome::Confirmed,
            error: None,
            submitted_at: "2026-01-01T00:00:00Z".into(),
            note: None,
        });
        h.entries.push(HistoryEntry {
            tx_hash: None,
            tx_type: "Transfer".into(),
            from: "0xddd".into(),
            to: None,
            amount: None,
            outcome: TxOutcome::Pending,
            error: None,
            submitted_at: "2026-01-01T00:00:00Z".into(),
            note: None,
        });

        let csv = h.to_csv();
        assert!(csv.starts_with("timestamp,type,from,to,amount,status,tx_hash,error\n"));
        assert!(csv.contains("accepted"));
        assert!(csv.contains("rejected"));
        assert!(csv.contains("confirmed"));
        assert!(csv.contains("pending"));
        assert_eq!(csv.lines().count(), 5);
    }

    #[test]
    fn test_to_csv_empty_history_has_only_header() {
        let h = TxHistory::new();
        let csv = h.to_csv();
        assert_eq!(csv.trim(), "timestamp,type,from,to,amount,status,tx_hash,error");
    }

    #[test]
    fn test_export_csv_writes_file() {
        let path = std::env::temp_dir().join("evaporchain_history_export_test.csv");
        let mut h = TxHistory::new();
        h.record_success("Transfer", "0xaaa", Some("0xbbb"), Some(50), "0xh1");
        h.export_csv(&path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("accepted"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_save_creates_nested_parent_dirs() {
        let base = std::env::temp_dir().join("ev_hist_nested_test").join("a").join("b");
        let path = base.join("history.json");
        let mut h = TxHistory::new();
        h.record_success("Transfer", "0xaaa", None, None, "0xh1");
        h.save(&path).unwrap();
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join("ev_hist_nested_test"));
    }
}
