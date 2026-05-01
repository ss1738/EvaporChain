//! Local chain event indexer for the EvaporChain wallet.
//!
//! Indexes blocks, events, and transaction receipts locally for fast querying.
//! Supports filtering by event type, address, block range, and more.
//! Data is persisted as JSON for offline access.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum IndexerError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ──────────────────────────── Enums ────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventType {
    Transfer,
    ObjectCreated,
    ObjectRefreshed,
    ObjectEvaporated,
    ContractCall,
    NftMint,
    NftTransfer,
    TokenTransfer,
    StakeDeposit,
    StakeWithdraw,
    GovernanceVote,
    Custom(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReceiptStatus {
    Success,
    Failed,
    Pending,
}

// ──────────────────────────── IndexedEvent ─────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedEvent {
    pub id: String,
    pub event_type: EventType,
    pub block_height: u64,
    pub tx_hash: String,
    pub from_address: String,
    pub to_address: Option<String>,
    pub amount: Option<u64>,
    pub data: HashMap<String, String>,
    pub timestamp: String,
}

impl IndexedEvent {
    pub fn new(
        id: impl Into<String>,
        event_type: EventType,
        block_height: u64,
        tx_hash: impl Into<String>,
        from_address: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            event_type,
            block_height,
            tx_hash: tx_hash.into(),
            from_address: from_address.into(),
            to_address: None,
            amount: None,
            data: HashMap::new(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub fn with_to(mut self, addr: &str) -> Self {
        self.to_address = Some(addr.to_string());
        self
    }

    pub fn with_amount(mut self, amount: u64) -> Self {
        self.amount = Some(amount);
        self
    }

    pub fn with_data(mut self, key: &str, value: &str) -> Self {
        self.data.insert(key.to_string(), value.to_string());
        self
    }

    pub fn matches_filter(&self, filter: &EventFilter) -> bool {
        filter.matches(self)
    }
}

// ──────────────────────────── EventFilter ──────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventFilter {
    pub event_types: Option<Vec<EventType>>,
    pub from_address: Option<String>,
    pub to_address: Option<String>,
    pub min_block: Option<u64>,
    pub max_block: Option<u64>,
    pub tx_hash: Option<String>,
    pub min_amount: Option<u64>,
}

impl EventFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_type(mut self, et: EventType) -> Self {
        let types = self.event_types.get_or_insert_with(Vec::new);
        types.push(et);
        self
    }

    pub fn with_from(mut self, addr: &str) -> Self {
        self.from_address = Some(addr.to_string());
        self
    }

    pub fn with_to(mut self, addr: &str) -> Self {
        self.to_address = Some(addr.to_string());
        self
    }

    pub fn with_block_range(mut self, min: u64, max: u64) -> Self {
        self.min_block = Some(min);
        self.max_block = Some(max);
        self
    }

    pub fn with_tx(mut self, hash: &str) -> Self {
        self.tx_hash = Some(hash.to_string());
        self
    }

    pub fn with_min_amount(mut self, amount: u64) -> Self {
        self.min_amount = Some(amount);
        self
    }

    /// All set filters must match (AND logic).
    pub fn matches(&self, event: &IndexedEvent) -> bool {
        if let Some(ref types) = self.event_types {
            if !types.contains(&event.event_type) {
                return false;
            }
        }
        if let Some(ref addr) = self.from_address {
            if event.from_address != *addr {
                return false;
            }
        }
        if let Some(ref addr) = self.to_address {
            if event.to_address.as_deref() != Some(addr.as_str()) {
                return false;
            }
        }
        if let Some(min) = self.min_block {
            if event.block_height < min {
                return false;
            }
        }
        if let Some(max) = self.max_block {
            if event.block_height > max {
                return false;
            }
        }
        if let Some(ref hash) = self.tx_hash {
            if event.tx_hash != *hash {
                return false;
            }
        }
        if let Some(min_amt) = self.min_amount {
            match event.amount {
                Some(amt) if amt >= min_amt => {}
                _ => return false,
            }
        }
        true
    }
}

// ──────────────────────────── TxReceipt ────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxReceipt {
    pub tx_hash: String,
    pub block_height: u64,
    pub status: ReceiptStatus,
    pub gas_used: u64,
    pub fee_paid: u64,
    pub events: Vec<String>,
    pub error_message: Option<String>,
    pub timestamp: String,
}

impl TxReceipt {
    pub fn new(
        tx_hash: impl Into<String>,
        block_height: u64,
        status: ReceiptStatus,
        gas_used: u64,
        fee_paid: u64,
    ) -> Self {
        Self {
            tx_hash: tx_hash.into(),
            block_height,
            status,
            gas_used,
            fee_paid,
            events: Vec::new(),
            error_message: None,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub fn with_error(mut self, msg: &str) -> Self {
        self.error_message = Some(msg.to_string());
        self
    }

    pub fn is_success(&self) -> bool {
        self.status == ReceiptStatus::Success
    }
}

// ──────────────────────────── IndexedBlock ──────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedBlock {
    pub height: u64,
    pub hash: String,
    pub parent_hash: String,
    pub tx_count: usize,
    pub event_count: usize,
    pub timestamp: String,
}

// ──────────────────────────── IndexerStats ──────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct IndexerStats {
    pub total_events: usize,
    pub total_receipts: usize,
    pub total_blocks: usize,
    pub last_indexed_block: u64,
    pub success_receipts: usize,
    pub failed_receipts: usize,
}

// ──────────────────────────── ChainIndexer ──────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct ChainIndexer {
    pub events: Vec<IndexedEvent>,
    pub receipts: HashMap<String, TxReceipt>,
    pub blocks: Vec<IndexedBlock>,
    pub last_indexed_block: u64,
    pub max_events: usize,
    pub max_blocks: usize,
}

impl Default for ChainIndexer {
    fn default() -> Self {
        Self {
            events: Vec::new(),
            receipts: HashMap::new(),
            blocks: Vec::new(),
            last_indexed_block: 0,
            max_events: 10_000,
            max_blocks: 1_000,
        }
    }
}

impl ChainIndexer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_limits(mut self, max_events: usize, max_blocks: usize) -> Self {
        self.max_events = max_events;
        self.max_blocks = max_blocks;
        self
    }

    // ── Indexing ──────────────────────────────────────────────────────

    pub fn index_event(&mut self, event: IndexedEvent) {
        self.events.push(event);
        if self.events.len() > self.max_events {
            let excess = self.events.len() - self.max_events;
            self.events.drain(..excess);
        }
    }

    pub fn index_receipt(&mut self, receipt: TxReceipt) {
        self.receipts.insert(receipt.tx_hash.clone(), receipt);
    }

    pub fn index_block(&mut self, block: IndexedBlock) {
        if block.height > self.last_indexed_block {
            self.last_indexed_block = block.height;
        }
        self.blocks.push(block);
        if self.blocks.len() > self.max_blocks {
            let excess = self.blocks.len() - self.max_blocks;
            self.blocks.drain(..excess);
        }
    }

    // ── Queries ───────────────────────────────────────────────────────

    pub fn query_events(&self, filter: &EventFilter) -> Vec<&IndexedEvent> {
        self.events.iter().filter(|e| filter.matches(e)).collect()
    }

    pub fn get_receipt(&self, tx_hash: &str) -> Option<&TxReceipt> {
        self.receipts.get(tx_hash)
    }

    pub fn get_block(&self, height: u64) -> Option<&IndexedBlock> {
        self.blocks.iter().find(|b| b.height == height)
    }

    pub fn latest_block(&self) -> Option<&IndexedBlock> {
        self.blocks.last()
    }

    pub fn events_for_address(&self, address: &str) -> Vec<&IndexedEvent> {
        self.events
            .iter()
            .filter(|e| e.from_address == address || e.to_address.as_deref() == Some(address))
            .collect()
    }

    pub fn events_for_tx(&self, tx_hash: &str) -> Vec<&IndexedEvent> {
        self.events
            .iter()
            .filter(|e| e.tx_hash == tx_hash)
            .collect()
    }

    pub fn events_in_block(&self, height: u64) -> Vec<&IndexedEvent> {
        self.events
            .iter()
            .filter(|e| e.block_height == height)
            .collect()
    }

    pub fn event_count_by_type(&self) -> HashMap<String, usize> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for event in &self.events {
            let key = format!("{:?}", event.event_type);
            *counts.entry(key).or_insert(0) += 1;
        }
        counts
    }

    pub fn receipts_by_status(&self, status: &ReceiptStatus) -> Vec<&TxReceipt> {
        self.receipts
            .values()
            .filter(|r| r.status == *status)
            .collect()
    }

    pub fn failed_receipts(&self) -> Vec<&TxReceipt> {
        self.receipts_by_status(&ReceiptStatus::Failed)
    }

    pub fn stats(&self) -> IndexerStats {
        let success_receipts = self
            .receipts
            .values()
            .filter(|r| r.status == ReceiptStatus::Success)
            .count();
        let failed_receipts = self
            .receipts
            .values()
            .filter(|r| r.status == ReceiptStatus::Failed)
            .count();
        IndexerStats {
            total_events: self.events.len(),
            total_receipts: self.receipts.len(),
            total_blocks: self.blocks.len(),
            last_indexed_block: self.last_indexed_block,
            success_receipts,
            failed_receipts,
        }
    }

    /// Remove events and blocks before the given block height.
    /// Returns the total number of items removed.
    pub fn clear_before_block(&mut self, height: u64) -> usize {
        let events_before = self.events.len();
        self.events.retain(|e| e.block_height >= height);
        let events_removed = events_before - self.events.len();

        let blocks_before = self.blocks.len();
        self.blocks.retain(|b| b.height >= height);
        let blocks_removed = blocks_before - self.blocks.len();

        events_removed + blocks_removed
    }

    // ── Persistence ───────────────────────────────────────────────────

    pub fn save(&self, path: &Path) -> Result<(), IndexerError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, IndexerError> {
        let data = std::fs::read_to_string(path)?;
        let indexer: Self = serde_json::from_str(&data)?;
        Ok(indexer)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ──────────────────────────── Tests ────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("chain_idx_test_{}", std::process::id()))
    }

    fn make_event(
        id: &str,
        event_type: EventType,
        block: u64,
        tx: &str,
        from: &str,
    ) -> IndexedEvent {
        IndexedEvent::new(id, event_type, block, tx, from)
    }

    fn make_block(height: u64) -> IndexedBlock {
        IndexedBlock {
            height,
            hash: format!("hash_{}", height),
            parent_hash: format!("hash_{}", height.saturating_sub(1)),
            tx_count: 5,
            event_count: 3,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }

    #[test]
    fn test_index_event() {
        let mut indexer = ChainIndexer::new();
        let event = make_event("e1", EventType::Transfer, 1, "tx1", "alice");
        indexer.index_event(event);
        assert_eq!(indexer.events.len(), 1);
        assert_eq!(indexer.events[0].id, "e1");
    }

    #[test]
    fn test_index_multiple_events() {
        let mut indexer = ChainIndexer::new();
        indexer.index_event(make_event("e1", EventType::Transfer, 1, "tx1", "alice"));
        indexer.index_event(make_event("e2", EventType::NftMint, 2, "tx2", "bob"));
        indexer.index_event(make_event("e3", EventType::StakeDeposit, 3, "tx3", "carol"));
        assert_eq!(indexer.events.len(), 3);
    }

    #[test]
    fn test_index_receipt() {
        let mut indexer = ChainIndexer::new();
        let receipt = TxReceipt::new("tx1", 1, ReceiptStatus::Success, 21000, 100);
        indexer.index_receipt(receipt);
        assert_eq!(indexer.receipts.len(), 1);
        assert!(indexer.receipts.contains_key("tx1"));
    }

    #[test]
    fn test_index_block() {
        let mut indexer = ChainIndexer::new();
        indexer.index_block(make_block(1));
        indexer.index_block(make_block(2));
        assert_eq!(indexer.blocks.len(), 2);
        assert_eq!(indexer.last_indexed_block, 2);
    }

    #[test]
    fn test_query_events_by_type() {
        let mut indexer = ChainIndexer::new();
        indexer.index_event(make_event("e1", EventType::Transfer, 1, "tx1", "alice"));
        indexer.index_event(make_event("e2", EventType::NftMint, 2, "tx2", "bob"));
        indexer.index_event(make_event("e3", EventType::Transfer, 3, "tx3", "carol"));

        let filter = EventFilter::new().with_type(EventType::Transfer);
        let results = indexer.query_events(&filter);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_query_events_by_from_address() {
        let mut indexer = ChainIndexer::new();
        indexer.index_event(make_event("e1", EventType::Transfer, 1, "tx1", "alice"));
        indexer.index_event(make_event("e2", EventType::Transfer, 2, "tx2", "bob"));

        let filter = EventFilter::new().with_from("alice");
        let results = indexer.query_events(&filter);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].from_address, "alice");
    }

    #[test]
    fn test_query_events_by_to_address() {
        let mut indexer = ChainIndexer::new();
        indexer
            .index_event(make_event("e1", EventType::Transfer, 1, "tx1", "alice").with_to("bob"));
        indexer
            .index_event(make_event("e2", EventType::Transfer, 2, "tx2", "alice").with_to("carol"));

        let filter = EventFilter::new().with_to("bob");
        let results = indexer.query_events(&filter);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].to_address.as_deref(), Some("bob"));
    }

    #[test]
    fn test_query_events_by_block_range() {
        let mut indexer = ChainIndexer::new();
        indexer.index_event(make_event("e1", EventType::Transfer, 5, "tx1", "alice"));
        indexer.index_event(make_event("e2", EventType::Transfer, 10, "tx2", "bob"));
        indexer.index_event(make_event("e3", EventType::Transfer, 15, "tx3", "carol"));

        let filter = EventFilter::new().with_block_range(6, 12);
        let results = indexer.query_events(&filter);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].block_height, 10);
    }

    #[test]
    fn test_query_events_by_tx_hash() {
        let mut indexer = ChainIndexer::new();
        indexer.index_event(make_event("e1", EventType::Transfer, 1, "tx_abc", "alice"));
        indexer.index_event(make_event("e2", EventType::Transfer, 2, "tx_def", "bob"));

        let filter = EventFilter::new().with_tx("tx_abc");
        let results = indexer.query_events(&filter);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].tx_hash, "tx_abc");
    }

    #[test]
    fn test_query_events_by_min_amount() {
        let mut indexer = ChainIndexer::new();
        indexer
            .index_event(make_event("e1", EventType::Transfer, 1, "tx1", "alice").with_amount(100));
        indexer
            .index_event(make_event("e2", EventType::Transfer, 2, "tx2", "bob").with_amount(500));
        indexer.index_event(make_event("e3", EventType::Transfer, 3, "tx3", "carol"));

        let filter = EventFilter::new().with_min_amount(200);
        let results = indexer.query_events(&filter);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].amount, Some(500));
    }

    #[test]
    fn test_query_events_combined_filter() {
        let mut indexer = ChainIndexer::new();
        indexer.index_event(
            make_event("e1", EventType::Transfer, 10, "tx1", "alice")
                .with_to("bob")
                .with_amount(500),
        );
        indexer.index_event(
            make_event("e2", EventType::Transfer, 20, "tx2", "alice")
                .with_to("carol")
                .with_amount(100),
        );
        indexer.index_event(
            make_event("e3", EventType::NftMint, 15, "tx3", "alice")
                .with_to("bob")
                .with_amount(300),
        );

        let filter = EventFilter::new()
            .with_type(EventType::Transfer)
            .with_from("alice")
            .with_min_amount(200);
        let results = indexer.query_events(&filter);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "e1");
    }

    #[test]
    fn test_events_for_address() {
        let mut indexer = ChainIndexer::new();
        indexer.index_event(make_event("e1", EventType::Transfer, 1, "tx1", "alice"));
        indexer
            .index_event(make_event("e2", EventType::Transfer, 2, "tx2", "bob").with_to("alice"));
        indexer.index_event(make_event("e3", EventType::Transfer, 3, "tx3", "carol"));

        let results = indexer.events_for_address("alice");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_events_for_tx() {
        let mut indexer = ChainIndexer::new();
        indexer.index_event(make_event("e1", EventType::Transfer, 1, "tx_same", "alice"));
        indexer.index_event(make_event("e2", EventType::NftMint, 1, "tx_same", "alice"));
        indexer.index_event(make_event("e3", EventType::Transfer, 2, "tx_other", "bob"));

        let results = indexer.events_for_tx("tx_same");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_events_in_block() {
        let mut indexer = ChainIndexer::new();
        indexer.index_event(make_event("e1", EventType::Transfer, 5, "tx1", "alice"));
        indexer.index_event(make_event("e2", EventType::NftMint, 5, "tx2", "bob"));
        indexer.index_event(make_event("e3", EventType::Transfer, 6, "tx3", "carol"));

        let results = indexer.events_in_block(5);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_event_count_by_type() {
        let mut indexer = ChainIndexer::new();
        indexer.index_event(make_event("e1", EventType::Transfer, 1, "tx1", "alice"));
        indexer.index_event(make_event("e2", EventType::Transfer, 2, "tx2", "bob"));
        indexer.index_event(make_event("e3", EventType::NftMint, 3, "tx3", "carol"));
        indexer.index_event(make_event("e4", EventType::StakeDeposit, 4, "tx4", "dave"));

        let counts = indexer.event_count_by_type();
        assert_eq!(counts.get("Transfer"), Some(&2));
        assert_eq!(counts.get("NftMint"), Some(&1));
        assert_eq!(counts.get("StakeDeposit"), Some(&1));
    }

    #[test]
    fn test_get_receipt() {
        let mut indexer = ChainIndexer::new();
        let receipt = TxReceipt::new("tx_abc", 10, ReceiptStatus::Success, 21000, 100);
        indexer.index_receipt(receipt);

        let r = indexer.get_receipt("tx_abc").unwrap();
        assert!(r.is_success());
        assert_eq!(r.gas_used, 21000);

        assert!(indexer.get_receipt("nonexistent").is_none());
    }

    #[test]
    fn test_failed_receipts() {
        let mut indexer = ChainIndexer::new();
        indexer.index_receipt(TxReceipt::new("tx1", 1, ReceiptStatus::Success, 21000, 100));
        indexer.index_receipt(
            TxReceipt::new("tx2", 2, ReceiptStatus::Failed, 21000, 50).with_error("out of gas"),
        );
        indexer.index_receipt(
            TxReceipt::new("tx3", 3, ReceiptStatus::Failed, 15000, 30).with_error("reverted"),
        );

        let failed = indexer.failed_receipts();
        assert_eq!(failed.len(), 2);
        for r in &failed {
            assert_eq!(r.status, ReceiptStatus::Failed);
            assert!(r.error_message.is_some());
        }
    }

    #[test]
    fn test_latest_block() {
        let mut indexer = ChainIndexer::new();
        assert!(indexer.latest_block().is_none());

        indexer.index_block(make_block(1));
        indexer.index_block(make_block(2));
        indexer.index_block(make_block(3));

        let latest = indexer.latest_block().unwrap();
        assert_eq!(latest.height, 3);
    }

    #[test]
    fn test_clear_before_block() {
        let mut indexer = ChainIndexer::new();
        indexer.index_event(make_event("e1", EventType::Transfer, 5, "tx1", "alice"));
        indexer.index_event(make_event("e2", EventType::Transfer, 10, "tx2", "bob"));
        indexer.index_event(make_event("e3", EventType::Transfer, 15, "tx3", "carol"));
        indexer.index_block(make_block(5));
        indexer.index_block(make_block(10));
        indexer.index_block(make_block(15));

        let removed = indexer.clear_before_block(10);
        // 1 event (block 5) + 1 block (height 5) = 2
        assert_eq!(removed, 2);
        assert_eq!(indexer.events.len(), 2);
        assert_eq!(indexer.blocks.len(), 2);
    }

    #[test]
    fn test_auto_prune_events() {
        let mut indexer = ChainIndexer::new().with_limits(5, 3);

        for i in 0..8 {
            indexer.index_event(make_event(
                &format!("e{}", i),
                EventType::Transfer,
                i as u64,
                &format!("tx{}", i),
                "alice",
            ));
        }
        assert_eq!(indexer.events.len(), 5);
        // oldest events should have been pruned; remaining are e3..e7
        assert_eq!(indexer.events[0].id, "e3");

        for i in 0..5 {
            indexer.index_block(make_block(i));
        }
        assert_eq!(indexer.blocks.len(), 3);
        assert_eq!(indexer.blocks[0].height, 2);
    }

    #[test]
    fn test_stats() {
        let mut indexer = ChainIndexer::new();
        indexer.index_event(make_event("e1", EventType::Transfer, 1, "tx1", "alice"));
        indexer.index_event(make_event("e2", EventType::NftMint, 2, "tx2", "bob"));
        indexer.index_receipt(TxReceipt::new("tx1", 1, ReceiptStatus::Success, 21000, 100));
        indexer.index_receipt(TxReceipt::new("tx2", 2, ReceiptStatus::Failed, 15000, 50));
        indexer.index_receipt(TxReceipt::new("tx3", 3, ReceiptStatus::Pending, 0, 0));
        indexer.index_block(make_block(1));
        indexer.index_block(make_block(2));

        let stats = indexer.stats();
        assert_eq!(stats.total_events, 2);
        assert_eq!(stats.total_receipts, 3);
        assert_eq!(stats.total_blocks, 2);
        assert_eq!(stats.last_indexed_block, 2);
        assert_eq!(stats.success_receipts, 1);
        assert_eq!(stats.failed_receipts, 1);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path();

        let mut indexer = ChainIndexer::new();
        indexer.index_event(
            make_event("e1", EventType::Transfer, 1, "tx1", "alice")
                .with_to("bob")
                .with_amount(1000)
                .with_data("memo", "hello"),
        );
        indexer.index_receipt(TxReceipt::new("tx1", 1, ReceiptStatus::Success, 21000, 100));
        indexer.index_block(make_block(1));

        indexer.save(&path).unwrap();

        let loaded = ChainIndexer::load(&path).unwrap();
        assert_eq!(loaded.events.len(), 1);
        assert_eq!(loaded.events[0].id, "e1");
        assert_eq!(loaded.events[0].to_address.as_deref(), Some("bob"));
        assert_eq!(loaded.events[0].amount, Some(1000));
        assert_eq!(loaded.events[0].data.get("memo").unwrap(), "hello");
        assert_eq!(loaded.receipts.len(), 1);
        assert_eq!(loaded.blocks.len(), 1);
        assert_eq!(loaded.last_indexed_block, 1);

        // load_or_default on existing file
        let loaded2 = ChainIndexer::load_or_default(&path);
        assert_eq!(loaded2.events.len(), 1);

        // load_or_default on missing file
        let missing = std::env::temp_dir().join("nonexistent_chain_idx_file.json");
        let default = ChainIndexer::load_or_default(&missing);
        assert_eq!(default.events.len(), 0);
        assert_eq!(default.max_events, 10_000);

        // cleanup
        let _ = std::fs::remove_file(&path);
    }
}
