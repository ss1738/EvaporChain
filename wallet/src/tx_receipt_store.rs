use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum TxReceiptError {
    #[error("receipt already exists for tx_hash: {0}")]
    DuplicateReceipt(String),
    #[error("receipt not found for tx_hash: {0}")]
    NotFound(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TxReceiptStatus {
    Success,
    Failed,
    Pending,
    Dropped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TxType2 {
    Transfer,
    ContractDeploy,
    ContractCall,
    Refresh,
    Stake,
    Unstake,
    Governance,
    NFTMint,
    TokenTransfer,
    Bridge,
    Custom(String),
}

// ---------------------------------------------------------------------------
// Data structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxLog {
    pub index: u32,
    pub address: String,
    pub topics: Vec<String>,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxReceipt {
    pub tx_hash: String,
    pub block_number: Option<u64>,
    pub block_hash: Option<String>,
    pub from: String,
    pub to: String,
    pub amount: u64,
    pub token: String,
    pub tx_type: TxType2,
    pub status: TxReceiptStatus,
    pub gas_used: u64,
    pub gas_price: u64,
    pub fee: u64,
    pub nonce: u64,
    pub timestamp: String,
    pub confirmations: u32,
    pub logs: Vec<TxLog>,
    pub error_message: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxSummary {
    pub total_txs: usize,
    pub successful: usize,
    pub failed: usize,
    pub pending: usize,
    pub total_gas_spent: u64,
    pub total_fees_paid: u64,
    pub total_sent: u64,
    pub total_received: u64,
    pub unique_addresses: usize,
    pub date_range: Option<(String, String)>,
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TxReceiptStore {
    pub receipts: HashMap<String, TxReceipt>,
}

impl TxReceiptStore {
    pub fn new() -> Self {
        Self {
            receipts: HashMap::new(),
        }
    }

    // -- CRUD ---------------------------------------------------------------

    pub fn store_receipt(&mut self, receipt: TxReceipt) -> Result<(), TxReceiptError> {
        if self.receipts.contains_key(&receipt.tx_hash) {
            return Err(TxReceiptError::DuplicateReceipt(receipt.tx_hash.clone()));
        }
        self.receipts.insert(receipt.tx_hash.clone(), receipt);
        Ok(())
    }

    pub fn update_receipt(
        &mut self,
        tx_hash: &str,
        status: TxReceiptStatus,
        block_number: Option<u64>,
        confirmations: u32,
    ) -> Result<(), TxReceiptError> {
        let receipt = self
            .receipts
            .get_mut(tx_hash)
            .ok_or_else(|| TxReceiptError::NotFound(tx_hash.to_string()))?;
        receipt.status = status;
        receipt.block_number = block_number;
        receipt.confirmations = confirmations;
        Ok(())
    }

    pub fn get_receipt(&self, tx_hash: &str) -> Option<&TxReceipt> {
        self.receipts.get(tx_hash)
    }

    pub fn remove_receipt(&mut self, tx_hash: &str) -> Result<TxReceipt, TxReceiptError> {
        self.receipts
            .remove(tx_hash)
            .ok_or_else(|| TxReceiptError::NotFound(tx_hash.to_string()))
    }

    // -- Queries ------------------------------------------------------------

    pub fn receipts_by_address(&self, address: &str) -> Vec<&TxReceipt> {
        self.receipts
            .values()
            .filter(|r| r.from == address || r.to == address)
            .collect()
    }

    pub fn receipts_by_type(&self, tx_type: &TxType2) -> Vec<&TxReceipt> {
        self.receipts
            .values()
            .filter(|r| &r.tx_type == tx_type)
            .collect()
    }

    pub fn receipts_by_status(&self, status: &TxReceiptStatus) -> Vec<&TxReceipt> {
        self.receipts
            .values()
            .filter(|r| &r.status == status)
            .collect()
    }

    pub fn receipts_in_range(&self, start: &str, end: &str) -> Vec<&TxReceipt> {
        self.receipts
            .values()
            .filter(|r| r.timestamp.as_str() >= start && r.timestamp.as_str() <= end)
            .collect()
    }

    pub fn pending_receipts(&self) -> Vec<&TxReceipt> {
        self.receipts_by_status(&TxReceiptStatus::Pending)
    }

    pub fn failed_receipts(&self) -> Vec<&TxReceipt> {
        self.receipts_by_status(&TxReceiptStatus::Failed)
    }

    pub fn recent_receipts(&self, n: usize) -> Vec<&TxReceipt> {
        let mut sorted: Vec<&TxReceipt> = self.receipts.values().collect();
        sorted.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        sorted.truncate(n);
        sorted
    }

    pub fn add_note(&mut self, tx_hash: &str, note: &str) -> Result<(), TxReceiptError> {
        let receipt = self
            .receipts
            .get_mut(tx_hash)
            .ok_or_else(|| TxReceiptError::NotFound(tx_hash.to_string()))?;
        receipt.notes = Some(note.to_string());
        Ok(())
    }

    pub fn search(&self, query: &str) -> Vec<&TxReceipt> {
        let q = query.to_lowercase();
        self.receipts
            .values()
            .filter(|r| {
                r.tx_hash.to_lowercase().contains(&q)
                    || r.from.to_lowercase().contains(&q)
                    || r.to.to_lowercase().contains(&q)
                    || r.notes
                        .as_ref()
                        .map(|n| n.to_lowercase().contains(&q))
                        .unwrap_or(false)
            })
            .collect()
    }

    // -- Summaries ----------------------------------------------------------

    pub fn summary(&self) -> TxSummary {
        self.compute_summary(self.receipts.values().collect())
    }

    pub fn summary_for_address(&self, address: &str) -> TxSummary {
        let filtered = self.receipts_by_address(address);
        self.compute_summary(filtered)
    }

    fn compute_summary(&self, receipts: Vec<&TxReceipt>) -> TxSummary {
        let mut successful = 0usize;
        let mut failed = 0usize;
        let mut pending = 0usize;
        let mut total_gas_spent = 0u64;
        let mut total_fees_paid = 0u64;
        let mut total_sent = 0u64;
        let mut total_received = 0u64;
        let mut addresses = std::collections::HashSet::new();
        let mut min_ts: Option<String> = None;
        let mut max_ts: Option<String> = None;

        for r in &receipts {
            match r.status {
                TxReceiptStatus::Success => successful += 1,
                TxReceiptStatus::Failed => failed += 1,
                TxReceiptStatus::Pending => pending += 1,
                TxReceiptStatus::Dropped => {}
            }
            total_gas_spent += r.gas_used;
            total_fees_paid += r.fee;
            total_sent += r.amount;
            total_received += r.amount;
            addresses.insert(r.from.clone());
            addresses.insert(r.to.clone());

            match &min_ts {
                None => min_ts = Some(r.timestamp.clone()),
                Some(ts) if r.timestamp < *ts => min_ts = Some(r.timestamp.clone()),
                _ => {}
            }
            match &max_ts {
                None => max_ts = Some(r.timestamp.clone()),
                Some(ts) if r.timestamp > *ts => max_ts = Some(r.timestamp.clone()),
                _ => {}
            }
        }

        let date_range = match (min_ts, max_ts) {
            (Some(a), Some(b)) => Some((a, b)),
            _ => None,
        };

        TxSummary {
            total_txs: receipts.len(),
            successful,
            failed,
            pending,
            total_gas_spent,
            total_fees_paid,
            total_sent,
            total_received,
            unique_addresses: addresses.len(),
            date_range,
        }
    }

    // -- Persistence --------------------------------------------------------

    pub fn load(path: &Path) -> Result<Self, TxReceiptError> {
        let data = std::fs::read_to_string(path)?;
        let store: Self = serde_json::from_str(&data)?;
        Ok(store)
    }

    pub fn save(&self, path: &Path) -> Result<(), TxReceiptError> {
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Helper to build a receipt for tests
// ---------------------------------------------------------------------------

#[cfg(test)]
fn make_receipt(
    tx_hash: &str,
    from: &str,
    to: &str,
    amount: u64,
    status: TxReceiptStatus,
    tx_type: TxType2,
    timestamp: &str,
) -> TxReceipt {
    TxReceipt {
        tx_hash: tx_hash.to_string(),
        block_number: Some(100),
        block_hash: Some("blockhash_abc".to_string()),
        from: from.to_string(),
        to: to.to_string(),
        amount,
        token: "EVAP".to_string(),
        tx_type,
        status,
        gas_used: 21000,
        gas_price: 10,
        fee: 210000,
        nonce: 1,
        timestamp: timestamp.to_string(),
        confirmations: 6,
        logs: vec![],
        error_message: None,
        notes: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "evaporchain_test_{}_{}_{}",
            name,
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ))
    }

    fn sample_receipt(hash: &str) -> TxReceipt {
        make_receipt(
            hash,
            "alice",
            "bob",
            1000,
            TxReceiptStatus::Success,
            TxType2::Transfer,
            &chrono::Utc::now().to_rfc3339(),
        )
    }

    // 1
    #[test]
    fn test_new_store_is_empty() {
        let store = TxReceiptStore::new();
        assert!(store.receipts.is_empty());
    }

    // 2
    #[test]
    fn test_store_receipt() {
        let mut store = TxReceiptStore::new();
        assert!(store.store_receipt(sample_receipt("tx1")).is_ok());
        assert_eq!(store.receipts.len(), 1);
    }

    // 3
    #[test]
    fn test_store_duplicate_receipt() {
        let mut store = TxReceiptStore::new();
        store.store_receipt(sample_receipt("tx1")).unwrap();
        let result = store.store_receipt(sample_receipt("tx1"));
        assert!(result.is_err());
    }

    // 4
    #[test]
    fn test_get_receipt() {
        let mut store = TxReceiptStore::new();
        store.store_receipt(sample_receipt("tx1")).unwrap();
        let r = store.get_receipt("tx1");
        assert!(r.is_some());
        assert_eq!(r.unwrap().tx_hash, "tx1");
    }

    // 5
    #[test]
    fn test_get_receipt_not_found() {
        let store = TxReceiptStore::new();
        assert!(store.get_receipt("nope").is_none());
    }

    // 6
    #[test]
    fn test_remove_receipt() {
        let mut store = TxReceiptStore::new();
        store.store_receipt(sample_receipt("tx1")).unwrap();
        let removed = store.remove_receipt("tx1").unwrap();
        assert_eq!(removed.tx_hash, "tx1");
        assert!(store.receipts.is_empty());
    }

    // 7
    #[test]
    fn test_remove_receipt_not_found() {
        let mut store = TxReceiptStore::new();
        assert!(store.remove_receipt("nope").is_err());
    }

    // 8
    #[test]
    fn test_update_receipt() {
        let mut store = TxReceiptStore::new();
        let mut r = sample_receipt("tx1");
        r.status = TxReceiptStatus::Pending;
        store.store_receipt(r).unwrap();

        store
            .update_receipt("tx1", TxReceiptStatus::Success, Some(200), 12)
            .unwrap();

        let updated = store.get_receipt("tx1").unwrap();
        assert_eq!(updated.status, TxReceiptStatus::Success);
        assert_eq!(updated.block_number, Some(200));
        assert_eq!(updated.confirmations, 12);
    }

    // 9
    #[test]
    fn test_update_receipt_not_found() {
        let mut store = TxReceiptStore::new();
        let result = store.update_receipt("nope", TxReceiptStatus::Success, None, 0);
        assert!(result.is_err());
    }

    // 10
    #[test]
    fn test_receipts_by_address() {
        let mut store = TxReceiptStore::new();
        store
            .store_receipt(make_receipt(
                "tx1",
                "alice",
                "bob",
                100,
                TxReceiptStatus::Success,
                TxType2::Transfer,
                "2026-01-01T00:00:00Z",
            ))
            .unwrap();
        store
            .store_receipt(make_receipt(
                "tx2",
                "charlie",
                "alice",
                200,
                TxReceiptStatus::Success,
                TxType2::Transfer,
                "2026-01-02T00:00:00Z",
            ))
            .unwrap();
        store
            .store_receipt(make_receipt(
                "tx3",
                "dave",
                "eve",
                300,
                TxReceiptStatus::Success,
                TxType2::Transfer,
                "2026-01-03T00:00:00Z",
            ))
            .unwrap();

        let alice_txs = store.receipts_by_address("alice");
        assert_eq!(alice_txs.len(), 2);
    }

    // 11
    #[test]
    fn test_receipts_by_type() {
        let mut store = TxReceiptStore::new();
        store
            .store_receipt(make_receipt(
                "tx1",
                "a",
                "b",
                100,
                TxReceiptStatus::Success,
                TxType2::Transfer,
                "2026-01-01T00:00:00Z",
            ))
            .unwrap();
        store
            .store_receipt(make_receipt(
                "tx2",
                "a",
                "b",
                100,
                TxReceiptStatus::Success,
                TxType2::Stake,
                "2026-01-01T00:00:00Z",
            ))
            .unwrap();

        assert_eq!(store.receipts_by_type(&TxType2::Transfer).len(), 1);
        assert_eq!(store.receipts_by_type(&TxType2::Stake).len(), 1);
    }

    // 12
    #[test]
    fn test_receipts_by_status() {
        let mut store = TxReceiptStore::new();
        store
            .store_receipt(make_receipt(
                "tx1",
                "a",
                "b",
                100,
                TxReceiptStatus::Success,
                TxType2::Transfer,
                "2026-01-01T00:00:00Z",
            ))
            .unwrap();
        store
            .store_receipt(make_receipt(
                "tx2",
                "a",
                "b",
                100,
                TxReceiptStatus::Failed,
                TxType2::Transfer,
                "2026-01-01T00:00:00Z",
            ))
            .unwrap();

        assert_eq!(store.receipts_by_status(&TxReceiptStatus::Success).len(), 1);
        assert_eq!(store.receipts_by_status(&TxReceiptStatus::Failed).len(), 1);
    }

    // 13
    #[test]
    fn test_pending_receipts() {
        let mut store = TxReceiptStore::new();
        store
            .store_receipt(make_receipt(
                "tx1",
                "a",
                "b",
                100,
                TxReceiptStatus::Pending,
                TxType2::Transfer,
                "2026-01-01T00:00:00Z",
            ))
            .unwrap();
        store
            .store_receipt(make_receipt(
                "tx2",
                "a",
                "b",
                100,
                TxReceiptStatus::Success,
                TxType2::Transfer,
                "2026-01-01T00:00:00Z",
            ))
            .unwrap();

        assert_eq!(store.pending_receipts().len(), 1);
    }

    // 14
    #[test]
    fn test_failed_receipts() {
        let mut store = TxReceiptStore::new();
        store
            .store_receipt(make_receipt(
                "tx1",
                "a",
                "b",
                100,
                TxReceiptStatus::Failed,
                TxType2::Transfer,
                "2026-01-01T00:00:00Z",
            ))
            .unwrap();

        assert_eq!(store.failed_receipts().len(), 1);
    }

    // 15
    #[test]
    fn test_receipts_in_range() {
        let mut store = TxReceiptStore::new();
        store
            .store_receipt(make_receipt(
                "tx1",
                "a",
                "b",
                100,
                TxReceiptStatus::Success,
                TxType2::Transfer,
                "2026-01-01T00:00:00Z",
            ))
            .unwrap();
        store
            .store_receipt(make_receipt(
                "tx2",
                "a",
                "b",
                100,
                TxReceiptStatus::Success,
                TxType2::Transfer,
                "2026-06-15T00:00:00Z",
            ))
            .unwrap();
        store
            .store_receipt(make_receipt(
                "tx3",
                "a",
                "b",
                100,
                TxReceiptStatus::Success,
                TxType2::Transfer,
                "2026-12-31T00:00:00Z",
            ))
            .unwrap();

        let range = store.receipts_in_range("2026-01-01T00:00:00Z", "2026-07-01T00:00:00Z");
        assert_eq!(range.len(), 2);
    }

    // 16
    #[test]
    fn test_recent_receipts() {
        let mut store = TxReceiptStore::new();
        store
            .store_receipt(make_receipt(
                "tx1",
                "a",
                "b",
                100,
                TxReceiptStatus::Success,
                TxType2::Transfer,
                "2026-01-01T00:00:00Z",
            ))
            .unwrap();
        store
            .store_receipt(make_receipt(
                "tx2",
                "a",
                "b",
                100,
                TxReceiptStatus::Success,
                TxType2::Transfer,
                "2026-06-01T00:00:00Z",
            ))
            .unwrap();
        store
            .store_receipt(make_receipt(
                "tx3",
                "a",
                "b",
                100,
                TxReceiptStatus::Success,
                TxType2::Transfer,
                "2026-12-01T00:00:00Z",
            ))
            .unwrap();

        let recent = store.recent_receipts(2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].tx_hash, "tx3");
        assert_eq!(recent[1].tx_hash, "tx2");
    }

    // 17
    #[test]
    fn test_add_note() {
        let mut store = TxReceiptStore::new();
        store.store_receipt(sample_receipt("tx1")).unwrap();
        store.add_note("tx1", "important transaction").unwrap();
        assert_eq!(
            store.get_receipt("tx1").unwrap().notes.as_deref(),
            Some("important transaction")
        );
    }

    // 18
    #[test]
    fn test_add_note_not_found() {
        let mut store = TxReceiptStore::new();
        assert!(store.add_note("nope", "note").is_err());
    }

    // 19
    #[test]
    fn test_search() {
        let mut store = TxReceiptStore::new();
        store
            .store_receipt(make_receipt(
                "abc123",
                "alice",
                "bob",
                100,
                TxReceiptStatus::Success,
                TxType2::Transfer,
                "2026-01-01T00:00:00Z",
            ))
            .unwrap();
        store
            .store_receipt(make_receipt(
                "def456",
                "charlie",
                "dave",
                200,
                TxReceiptStatus::Success,
                TxType2::Transfer,
                "2026-01-01T00:00:00Z",
            ))
            .unwrap();

        assert_eq!(store.search("alice").len(), 1);
        assert_eq!(store.search("abc").len(), 1);
        assert_eq!(store.search("dave").len(), 1);
        assert_eq!(store.search("zzz").len(), 0);
    }

    // 20
    #[test]
    fn test_summary() {
        let mut store = TxReceiptStore::new();
        store
            .store_receipt(make_receipt(
                "tx1",
                "alice",
                "bob",
                100,
                TxReceiptStatus::Success,
                TxType2::Transfer,
                "2026-01-01T00:00:00Z",
            ))
            .unwrap();
        store
            .store_receipt(make_receipt(
                "tx2",
                "alice",
                "charlie",
                200,
                TxReceiptStatus::Failed,
                TxType2::Transfer,
                "2026-06-01T00:00:00Z",
            ))
            .unwrap();

        let s = store.summary();
        assert_eq!(s.total_txs, 2);
        assert_eq!(s.successful, 1);
        assert_eq!(s.failed, 1);
        assert_eq!(s.total_gas_spent, 42000);
        assert!(s.date_range.is_some());
    }

    // 21
    #[test]
    fn test_save_and_load() {
        let path = temp_path("save_load");
        let mut store = TxReceiptStore::new();
        store.store_receipt(sample_receipt("tx1")).unwrap();
        store.save(&path).unwrap();

        let loaded = TxReceiptStore::load(&path).unwrap();
        assert_eq!(loaded.receipts.len(), 1);
        assert!(loaded.get_receipt("tx1").is_some());

        let _ = std::fs::remove_file(&path);
    }

    // 22
    #[test]
    fn test_load_or_default_missing_file() {
        let path = temp_path("missing_file");
        let store = TxReceiptStore::load_or_default(&path);
        assert!(store.receipts.is_empty());
    }

    // 23
    #[test]
    fn test_summary_for_address() {
        let mut store = TxReceiptStore::new();
        store
            .store_receipt(make_receipt(
                "tx1",
                "alice",
                "bob",
                100,
                TxReceiptStatus::Success,
                TxType2::Transfer,
                "2026-01-01T00:00:00Z",
            ))
            .unwrap();
        store
            .store_receipt(make_receipt(
                "tx2",
                "charlie",
                "dave",
                500,
                TxReceiptStatus::Success,
                TxType2::Transfer,
                "2026-01-01T00:00:00Z",
            ))
            .unwrap();

        let s = store.summary_for_address("alice");
        assert_eq!(s.total_txs, 1);
    }

    // 24
    #[test]
    fn test_default_trait() {
        let store = TxReceiptStore::default();
        assert!(store.receipts.is_empty());
    }
}
