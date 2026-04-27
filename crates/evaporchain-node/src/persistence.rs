//! Persistence layer for non-state chain data (blocks, DeFi stores, consensus metadata).

use crate::api::{
    BlockRecord, ChainStats, DAOStore, EventRecord, NftStore, StakingStore, TokenStore,
};
use evaporchain_da::block_da::BlockDAPackage;
use evaporchain_types::{Block, Transaction};
use rocksdb::{ColumnFamilyDescriptor, Options, DB};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::path::Path;

const CF_BLOCKS: &str = "blocks";
const CF_META: &str = "chain_meta";
const CF_STORES: &str = "stores";
/// Full serialized Block objects for sync protocol (separate from BlockRecord summaries).
const CF_FULL_BLOCKS: &str = "full_blocks";
/// DA shard packages keyed by block number.
const CF_DA_SHARDS: &str = "da_shards";
/// PoHA certificate store (serialized PoHAStore state).
const CF_POHA: &str = "poha";
/// Transaction index: tx_hash (32 bytes) → TxReceipt JSON.
const CF_TX_INDEX: &str = "tx_index";
/// Address transaction history: address_hex:block_number:tx_index → tx_hash.
const CF_ADDR_HISTORY: &str = "addr_history";
/// Contract event logs: contract_id:block_number:log_index → ContractEventLog JSON.
const CF_CONTRACT_EVENTS: &str = "contract_events";
/// EvaporScript contracts keyed by ID.
const CF_SCRIPT_CONTRACTS: &str = "script_contracts";
/// Template-based ContractInstance objects keyed by ID.
const CF_TEMPLATE_CONTRACTS: &str = "template_contracts";

/// Persistent storage for chain data beyond the state DB.
pub struct ChainStore {
    db: DB,
}

impl ChainStore {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let cf_descriptors = vec![
            ColumnFamilyDescriptor::new(CF_BLOCKS, Options::default()),
            ColumnFamilyDescriptor::new(CF_META, Options::default()),
            ColumnFamilyDescriptor::new(CF_STORES, Options::default()),
            ColumnFamilyDescriptor::new(CF_FULL_BLOCKS, Options::default()),
            ColumnFamilyDescriptor::new(CF_DA_SHARDS, Options::default()),
            ColumnFamilyDescriptor::new(CF_POHA, Options::default()),
            ColumnFamilyDescriptor::new(CF_TX_INDEX, Options::default()),
            ColumnFamilyDescriptor::new(CF_ADDR_HISTORY, Options::default()),
            ColumnFamilyDescriptor::new(CF_CONTRACT_EVENTS, Options::default()),
            ColumnFamilyDescriptor::new(CF_SCRIPT_CONTRACTS, Options::default()),
            ColumnFamilyDescriptor::new(CF_TEMPLATE_CONTRACTS, Options::default()),
        ];

        let db = DB::open_cf_descriptors(&opts, path, cf_descriptors)
            .map_err(|e| format!("Failed to open ChainStore: {}", e))?;

        Ok(Self { db })
    }

    // ─── Consensus metadata ───

    pub fn save_consensus_meta(&self, block_number: u64, epoch: u64, parent_hash: [u8; 32]) -> Result<(), String> {
        let cf = self.db.cf_handle(CF_META).unwrap();
        self.db.put_cf(cf, b"block_number", block_number.to_le_bytes()).map_err(|e| e.to_string())?;
        self.db.put_cf(cf, b"epoch", epoch.to_le_bytes()).map_err(|e| e.to_string())?;
        self.db.put_cf(cf, b"parent_hash", parent_hash).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn load_consensus_meta(&self) -> Option<(u64, u64, [u8; 32])> {
        let cf = self.db.cf_handle(CF_META).unwrap();
        let bn = self.db.get_cf(cf, b"block_number").ok()??;
        let ep = self.db.get_cf(cf, b"epoch").ok()??;
        let ph = self.db.get_cf(cf, b"parent_hash").ok()??;

        if bn.len() < 8 || ep.len() < 8 || ph.len() < 32 {
            return None;
        }

        let block_number = u64::from_le_bytes(bn[..8].try_into().ok()?);
        let epoch = u64::from_le_bytes(ep[..8].try_into().ok()?);
        let mut parent_hash = [0u8; 32];
        parent_hash.copy_from_slice(&ph[..32]);

        Some((block_number, epoch, parent_hash))
    }

    // ─── Block history ───

    pub fn save_block(&self, record: &BlockRecord) -> Result<(), String> {
        let cf = self.db.cf_handle(CF_BLOCKS).unwrap();
        let key = record.number.to_be_bytes();
        let value = serde_json::to_vec(record).map_err(|e| e.to_string())?;
        self.db.put_cf(cf, key, value).map_err(|e| e.to_string())
    }

    pub fn load_block_history(&self, limit: usize) -> VecDeque<BlockRecord> {
        let cf = self.db.cf_handle(CF_BLOCKS).unwrap();
        let mut blocks = VecDeque::new();

        // Iterate in reverse (latest blocks first) to get the most recent `limit`
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::End);
        for item in iter {
            if blocks.len() >= limit {
                break;
            }
            if let Ok((_, value)) = item {
                if let Ok(record) = serde_json::from_slice::<BlockRecord>(&value) {
                    blocks.push_front(record);
                }
            }
        }
        blocks
    }

    /// Prune block records older than `current_height - retain`.
    /// Returns the number of blocks pruned.
    pub fn prune_blocks(&self, current_height: u64, retain: u64) -> usize {
        if current_height <= retain {
            return 0;
        }
        let cutoff = current_height - retain;
        let cf = self.db.cf_handle(CF_BLOCKS).unwrap();
        let mut pruned = 0;
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);
        for item in iter {
            if let Ok((key, _)) = item {
                if key.len() >= 8 {
                    let block_num = u64::from_be_bytes(key[..8].try_into().unwrap());
                    if block_num >= cutoff {
                        break;
                    }
                    let _ = self.db.delete_cf(cf, &key);
                    pruned += 1;
                }
            } else {
                break;
            }
        }
        pruned
    }

    /// Prune old snapshots, keeping only the latest.
    pub fn prune_old_snapshots(&self, current_height: u64, retain: u64) {
        let cf = self.db.cf_handle(CF_META).unwrap();
        if current_height <= retain {
            return;
        }
        let cutoff = current_height - retain;
        // Scan for snapshot:N keys where N < cutoff
        let prefix = b"snapshot:";
        let iter = self.db.prefix_iterator_cf(cf, prefix);
        for (key, _) in iter.flatten() {
            if let Ok(key_str) = std::str::from_utf8(&key) {
                if let Some(height_str) = key_str.strip_prefix("snapshot:") {
                    if let Ok(h) = height_str.parse::<u64>() {
                        if h < cutoff {
                            let _ = self.db.delete_cf(cf, &key);
                        }
                    }
                }
            }
        }
    }

    // ─── Chain stats ───

    pub fn save_chain_stats(&self, stats: &ChainStats) -> Result<(), String> {
        let cf = self.db.cf_handle(CF_META).unwrap();
        let value = serde_json::to_vec(stats).map_err(|e| e.to_string())?;
        self.db.put_cf(cf, b"chain_stats", value).map_err(|e| e.to_string())
    }

    pub fn load_chain_stats(&self) -> Option<ChainStats> {
        let cf = self.db.cf_handle(CF_META).unwrap();
        let data = self.db.get_cf(cf, b"chain_stats").ok()??;
        serde_json::from_slice(&data).ok()
    }

    // ─── DeFi stores (serialized as complete JSON blobs) ───

    pub fn save_nft_store(&self, store: &NftStore) -> Result<(), String> {
        let cf = self.db.cf_handle(CF_STORES).unwrap();
        let value = serde_json::to_vec(store).map_err(|e| e.to_string())?;
        self.db.put_cf(cf, b"nft_store", value).map_err(|e| e.to_string())
    }

    pub fn load_nft_store(&self) -> Option<NftStore> {
        let cf = self.db.cf_handle(CF_STORES).unwrap();
        let data = self.db.get_cf(cf, b"nft_store").ok()??;
        serde_json::from_slice(&data).ok()
    }

    pub fn save_token_store(&self, store: &TokenStore) -> Result<(), String> {
        let cf = self.db.cf_handle(CF_STORES).unwrap();
        let value = serde_json::to_vec(store).map_err(|e| e.to_string())?;
        self.db.put_cf(cf, b"token_store", value).map_err(|e| e.to_string())
    }

    pub fn load_token_store(&self) -> Option<TokenStore> {
        let cf = self.db.cf_handle(CF_STORES).unwrap();
        let data = self.db.get_cf(cf, b"token_store").ok()??;
        serde_json::from_slice(&data).ok()
    }

    pub fn save_staking_store(&self, store: &StakingStore) -> Result<(), String> {
        let cf = self.db.cf_handle(CF_STORES).unwrap();
        let value = serde_json::to_vec(store).map_err(|e| e.to_string())?;
        self.db.put_cf(cf, b"staking_store", value).map_err(|e| e.to_string())
    }

    pub fn load_staking_store(&self) -> Option<StakingStore> {
        let cf = self.db.cf_handle(CF_STORES).unwrap();
        let data = self.db.get_cf(cf, b"staking_store").ok()??;
        serde_json::from_slice(&data).ok()
    }

    pub fn save_dao_store(&self, store: &DAOStore) -> Result<(), String> {
        let cf = self.db.cf_handle(CF_STORES).unwrap();
        let value = serde_json::to_vec(store).map_err(|e| e.to_string())?;
        self.db.put_cf(cf, b"dao_store", value).map_err(|e| e.to_string())
    }

    pub fn load_dao_store(&self) -> Option<DAOStore> {
        let cf = self.db.cf_handle(CF_STORES).unwrap();
        let data = self.db.get_cf(cf, b"dao_store").ok()??;
        serde_json::from_slice(&data).ok()
    }

    // ─── State snapshots ───

    pub fn save_snapshot(&self, height: u64, data: &[u8], state_root: [u8; 32]) -> Result<(), String> {
        let cf = self.db.cf_handle(CF_META).unwrap();
        let key = format!("snapshot:{}", height);
        self.db.put_cf(cf, key.as_bytes(), data).map_err(|e| e.to_string())?;
        self.db.put_cf(cf, b"snapshot_latest_height", height.to_le_bytes()).map_err(|e| e.to_string())?;
        self.db.put_cf(cf, b"snapshot_latest_root", state_root).map_err(|e| e.to_string())
    }

    pub fn load_latest_snapshot(&self) -> Option<(u64, [u8; 32], Vec<u8>)> {
        let cf = self.db.cf_handle(CF_META).unwrap();
        let height_bytes = self.db.get_cf(cf, b"snapshot_latest_height").ok()??;
        let root_bytes = self.db.get_cf(cf, b"snapshot_latest_root").ok()??;
        if height_bytes.len() < 8 || root_bytes.len() < 32 {
            return None;
        }
        let height = u64::from_le_bytes(height_bytes[..8].try_into().ok()?);
        let mut state_root = [0u8; 32];
        state_root.copy_from_slice(&root_bytes[..32]);
        let key = format!("snapshot:{}", height);
        let data = self.db.get_cf(cf, key.as_bytes()).ok()??;
        Some((height, state_root, data))
    }

    // ─── Mempool persistence ───

    pub fn save_mempool(&self, txs: &[evaporchain_types::Transaction]) -> Result<(), String> {
        let cf = self.db.cf_handle(CF_META).unwrap();
        let value = serde_json::to_vec(txs).map_err(|e| e.to_string())?;
        self.db.put_cf(cf, b"mempool", value).map_err(|e| e.to_string())
    }

    pub fn load_mempool(&self) -> Vec<evaporchain_types::Transaction> {
        let cf = self.db.cf_handle(CF_META).unwrap();
        self.db.get_cf(cf, b"mempool").ok()
            .flatten()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default()
    }

    // ─── Events ───

    pub fn save_events(&self, events: &VecDeque<EventRecord>) -> Result<(), String> {
        let cf = self.db.cf_handle(CF_META).unwrap();
        let value = serde_json::to_vec(events).map_err(|e| e.to_string())?;
        self.db.put_cf(cf, b"events", value).map_err(|e| e.to_string())
    }

    pub fn load_events(&self) -> VecDeque<EventRecord> {
        let cf = self.db.cf_handle(CF_META).unwrap();
        self.db.get_cf(cf, b"events").ok()
            .flatten()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default()
    }

    // ─── Full block storage (for sync protocol) ───

    /// Store a complete Block object for serving sync requests after restart.
    pub fn save_full_block(&self, block: &Block) -> Result<(), String> {
        let cf = self.db.cf_handle(CF_FULL_BLOCKS).unwrap();
        let key = block.number.to_be_bytes();
        let value = serde_json::to_vec(block).map_err(|e| e.to_string())?;
        self.db.put_cf(cf, key, value).map_err(|e| e.to_string())
    }

    /// Load a single full block by height.
    pub fn load_full_block(&self, height: u64) -> Option<Block> {
        let cf = self.db.cf_handle(CF_FULL_BLOCKS).unwrap();
        let key = height.to_be_bytes();
        let data = self.db.get_cf(cf, key).ok()??;
        serde_json::from_slice(&data).ok()
    }

    /// Load the most recent `limit` full blocks (for populating BlockCache on startup).
    pub fn load_recent_full_blocks(&self, limit: usize) -> Vec<Block> {
        let cf = self.db.cf_handle(CF_FULL_BLOCKS).unwrap();
        let mut blocks = Vec::new();
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::End);
        for item in iter {
            if blocks.len() >= limit {
                break;
            }
            if let Ok((_, value)) = item {
                if let Ok(block) = serde_json::from_slice::<Block>(&value) {
                    blocks.push(block);
                }
            }
        }
        blocks.reverse(); // Return in ascending order
        blocks
    }

    /// Prune full blocks older than `current_height - retain`.
    pub fn prune_full_blocks(&self, current_height: u64, retain: u64) -> usize {
        if current_height <= retain {
            return 0;
        }
        let cutoff = current_height - retain;
        let cf = self.db.cf_handle(CF_FULL_BLOCKS).unwrap();
        let mut pruned = 0;
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);
        for item in iter {
            if let Ok((key, _)) = item {
                if key.len() >= 8 {
                    let block_num = u64::from_be_bytes(key[..8].try_into().unwrap());
                    if block_num >= cutoff {
                        break;
                    }
                    let _ = self.db.delete_cf(cf, &key);
                    pruned += 1;
                }
            } else {
                break;
            }
        }
        pruned
    }

    // ─── DA shard persistence ───

    pub fn save_da_package(&self, block_number: u64, package: &BlockDAPackage) -> Result<(), String> {
        let cf = self.db.cf_handle(CF_DA_SHARDS).unwrap();
        let key = block_number.to_be_bytes();
        let value = serde_json::to_vec(package).map_err(|e| e.to_string())?;
        self.db.put_cf(cf, key, value).map_err(|e| e.to_string())
    }

    #[allow(dead_code)]
    pub fn load_da_package(&self, block_number: u64) -> Option<BlockDAPackage> {
        let cf = self.db.cf_handle(CF_DA_SHARDS).unwrap();
        let key = block_number.to_be_bytes();
        let data = self.db.get_cf(cf, key).ok()??;
        serde_json::from_slice(&data).ok()
    }

    pub fn load_recent_da_packages(&self, limit: usize) -> BTreeMap<u64, BlockDAPackage> {
        let cf = self.db.cf_handle(CF_DA_SHARDS).unwrap();
        let mut packages = BTreeMap::new();
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::End);
        for item in iter {
            if packages.len() >= limit {
                break;
            }
            if let Ok((key, value)) = item {
                if key.len() >= 8 {
                    let bn = u64::from_be_bytes(key[..8].try_into().unwrap());
                    if let Ok(pkg) = serde_json::from_slice::<BlockDAPackage>(&value) {
                        packages.insert(bn, pkg);
                    }
                }
            }
        }
        packages
    }

    // ─── PoHA persistence ───

    pub fn save_poha_state(&self, store: &evaporchain_da::poha::PoHAStore) -> Result<(), String> {
        let cf = self.db.cf_handle(CF_POHA).unwrap();
        // Save each active certificate keyed by block number
        for (&block_number, cert) in store.all_active() {
            let key = block_number.to_be_bytes();
            let value = serde_json::to_vec(cert).map_err(|e| e.to_string())?;
            self.db.put_cf(cf, key, value).map_err(|e| e.to_string())?;
        }
        // Save ghost records with a prefix
        for (&block_number, ghost) in store.all_ghosts() {
            let mut key = Vec::with_capacity(9);
            key.push(b'G');
            key.extend_from_slice(&block_number.to_be_bytes());
            let value = serde_json::to_vec(ghost).map_err(|e| e.to_string())?;
            self.db.put_cf(cf, key, value).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn load_poha_state(&self, store: &mut evaporchain_da::poha::PoHAStore) -> usize {
        let cf = self.db.cf_handle(CF_POHA).unwrap();
        let mut loaded = 0usize;
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);
        for (key, value) in iter.flatten() {
                if key.starts_with(b"G") && key.len() >= 9 {
                    // Ghost record
                    if let Ok(ghost) = serde_json::from_slice::<evaporchain_da::poha::CertGhost>(&value) {
                        store.insert_ghost(ghost);
                        loaded += 1;
                    }
                } else if key.len() >= 8 {
                    // Active certificate
                    if let Ok(cert) = serde_json::from_slice::<evaporchain_da::poha::PoHACertificate>(&value) {
                        store.insert_certificate(cert);
                        loaded += 1;
                    }
                }
        }
        loaded
    }

    // ─── Transaction receipt indexing ───

    /// Compute a deterministic blake3 hash for a transaction.
    pub fn compute_tx_hash(tx: &evaporchain_types::Transaction) -> [u8; 32] {
        let serialized = serde_json::to_vec(tx).unwrap_or_default();
        *blake3::hash(&serialized).as_bytes()
    }

    /// Index all transactions in a block. Call after save_full_block().
    pub fn index_block_transactions(&self, block: &Block) -> Result<usize, String> {
        let tx_cf = self.db.cf_handle(CF_TX_INDEX).unwrap();
        let addr_cf = self.db.cf_handle(CF_ADDR_HISTORY).unwrap();
        let mut indexed = 0;

        for (tx_idx, tx) in block.transactions.iter().enumerate() {
            let tx_hash = Self::compute_tx_hash(tx);
            let receipt = TxReceipt {
                tx_hash: hex::encode(tx_hash),
                block_number: block.number,
                tx_index: tx_idx as u32,
                epoch: block.epoch,
                timestamp: block.timestamp,
                tx_type: tx_type_name(tx).to_string(),
                from: tx_sender_hex(tx),
                to: tx_receiver_hex(tx),
                status: "confirmed".to_string(),
                gas_used: crate::api::estimate_tx_gas_pub(tx),
                revert_reason: None,
                log_count: 0,
            };
            let value = serde_json::to_vec(&receipt).map_err(|e| e.to_string())?;
            self.db.put_cf(tx_cf, tx_hash, &value).map_err(|e| e.to_string())?;

            // Index by sender address
            if let Some(ref addr) = receipt.from {
                let addr_key = format!("{}:{:016x}:{:04x}", addr, block.number, tx_idx);
                self.db.put_cf(addr_cf, addr_key.as_bytes(), tx_hash).map_err(|e| e.to_string())?;
            }
            // Index by receiver address
            if let Some(ref addr) = receipt.to {
                if receipt.from.as_deref() != Some(addr) {
                    let addr_key = format!("{}:{:016x}:{:04x}", addr, block.number, tx_idx);
                    self.db.put_cf(addr_cf, addr_key.as_bytes(), tx_hash).map_err(|e| e.to_string())?;
                }
            }

            indexed += 1;
        }
        Ok(indexed)
    }

    /// Look up a transaction receipt by its blake3 hash (hex string or raw bytes).
    pub fn get_tx_receipt(&self, hash_hex: &str) -> Option<TxReceipt> {
        let hash_bytes = hex::decode(hash_hex).ok()?;
        if hash_bytes.len() != 32 { return None; }
        let cf = self.db.cf_handle(CF_TX_INDEX).unwrap();
        let data = self.db.get_cf(cf, &hash_bytes).ok()??;
        serde_json::from_slice(&data).ok()
    }

    /// Get transaction history for an address (hex prefix, e.g. "0x01000000").
    /// Returns receipts in reverse chronological order, up to `limit`.
    pub fn get_address_transactions(&self, addr_hex: &str, limit: usize) -> Vec<TxReceipt> {
        let addr_cf = self.db.cf_handle(CF_ADDR_HISTORY).unwrap();
        let tx_cf = self.db.cf_handle(CF_TX_INDEX).unwrap();
        let prefix = format!("{}:", addr_hex);
        let mut receipts = Vec::new();

        // Scan in reverse to get most recent first
        let mut iter = self.db.prefix_iterator_cf(addr_cf, prefix.as_bytes());
        let mut all_entries: Vec<Vec<u8>> = Vec::new();
        while let Some(Ok((key, value))) = iter.next() {
            if !key.starts_with(prefix.as_bytes()) { break; }
            all_entries.push(value.to_vec());
        }

        // Reverse for chronological order (newest first)
        for tx_hash in all_entries.into_iter().rev().take(limit) {
            if let Ok(Some(data)) = self.db.get_cf(tx_cf, &tx_hash) {
                if let Ok(receipt) = serde_json::from_slice::<TxReceipt>(&data) {
                    receipts.push(receipt);
                }
            }
        }
        receipts
    }

    /// Count total indexed transactions.
    pub fn tx_index_count(&self) -> usize {
        let cf = self.db.cf_handle(CF_TX_INDEX).unwrap();
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);
        iter.count()
    }

    // ─── Contract event log indexing ───

    /// Store contract events emitted during a block.
    pub fn index_contract_events(
        &self,
        block_number: u64,
        epoch: u64,
        timestamp: u64,
        contract_id: u64,
        tx_hash: &str,
        events: &[evaporchain_script::ContractEvent],
    ) -> Result<usize, String> {
        let cf = self.db.cf_handle(CF_CONTRACT_EVENTS).unwrap();
        let mut indexed = 0;

        for (log_idx, event) in events.iter().enumerate() {
            let log = ContractEventLog {
                contract_id,
                block_number,
                log_index: log_idx as u32,
                epoch,
                timestamp,
                tx_hash: tx_hash.to_string(),
                event_name: event.name.clone(),
                topics: event.topics.iter().map(|v| format!("{v}")).collect(),
                data: event.data.iter().map(|v| format!("{v}")).collect(),
            };
            // Key format: contract_id(8 bytes) + block_number(8 bytes) + log_index(4 bytes)
            let mut key = Vec::with_capacity(20);
            key.extend_from_slice(&contract_id.to_be_bytes());
            key.extend_from_slice(&block_number.to_be_bytes());
            key.extend_from_slice(&(log_idx as u32).to_be_bytes());
            let value = serde_json::to_vec(&log).map_err(|e| e.to_string())?;
            self.db.put_cf(cf, &key, &value).map_err(|e| e.to_string())?;
            indexed += 1;
        }
        Ok(indexed)
    }

    /// Query contract events by contract ID, with optional event name filter and block range.
    pub fn get_contract_events(
        &self,
        contract_id: u64,
        event_name: Option<&str>,
        from_block: Option<u64>,
        to_block: Option<u64>,
        limit: usize,
    ) -> Vec<ContractEventLog> {
        let cf = self.db.cf_handle(CF_CONTRACT_EVENTS).unwrap();
        let prefix = contract_id.to_be_bytes();
        let iter = self.db.prefix_iterator_cf(cf, prefix);
        let mut results = Vec::new();

        for item in iter {
            if results.len() >= limit {
                break;
            }
            if let Ok((key, value)) = item {
                if !key.starts_with(&prefix) {
                    break;
                }
                if let Ok(log) = serde_json::from_slice::<ContractEventLog>(&value) {
                    if let Some(from) = from_block {
                        if log.block_number < from {
                            continue;
                        }
                    }
                    if let Some(to) = to_block {
                        if log.block_number > to {
                            continue;
                        }
                    }
                    if let Some(name) = event_name {
                        if log.event_name != name {
                            continue;
                        }
                    }
                    results.push(log);
                }
            } else {
                break;
            }
        }
        results
    }

    /// Get all events in a specific block (across all contracts).
    pub fn get_block_events(&self, block_number: u64, limit: usize) -> Vec<ContractEventLog> {
        let cf = self.db.cf_handle(CF_CONTRACT_EVENTS).unwrap();
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);
        let mut results = Vec::new();

        for item in iter {
            if results.len() >= limit {
                break;
            }
            if let Ok((_key, value)) = item {
                if let Ok(log) = serde_json::from_slice::<ContractEventLog>(&value) {
                    if log.block_number == block_number {
                        results.push(log);
                    } else if log.block_number > block_number {
                        break;
                    }
                }
            } else {
                break;
            }
        }
        results
    }

    /// Count total indexed contract events.
    pub fn contract_event_count(&self) -> usize {
        let cf = self.db.cf_handle(CF_CONTRACT_EVENTS).unwrap();
        self.db.iterator_cf(cf, rocksdb::IteratorMode::Start).count()
    }

    pub fn prune_da_packages(&self, current_height: u64, retain: u64) -> usize {
        if current_height <= retain {
            return 0;
        }
        let cutoff = current_height - retain;
        let cf = self.db.cf_handle(CF_DA_SHARDS).unwrap();
        let mut pruned = 0;
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);
        for item in iter {
            if let Ok((key, _)) = item {
                if key.len() >= 8 {
                    let block_num = u64::from_be_bytes(key[..8].try_into().unwrap());
                    if block_num >= cutoff {
                        break;
                    }
                    let _ = self.db.delete_cf(cf, &key);
                    pruned += 1;
                }
            } else {
                break;
            }
        }
        pruned
    }

    // ─── Script contract persistence ───

    pub fn save_script_contracts(&self, contracts: &[&evaporchain_script::ScriptContract]) -> Result<(), String> {
        let cf = self.db.cf_handle(CF_SCRIPT_CONTRACTS).ok_or("missing CF")?;
        for c in contracts {
            let data = serde_json::to_vec(c).map_err(|e| e.to_string())?;
            self.db.put_cf(cf, c.id.to_le_bytes(), data).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn load_all_script_contracts(&self) -> Vec<evaporchain_script::ScriptContract> {
        let cf = match self.db.cf_handle(CF_SCRIPT_CONTRACTS) {
            Some(cf) => cf,
            None => return vec![],
        };
        let mut contracts = vec![];
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);
        for (_key, value) in iter.flatten() {
                if let Ok(c) = serde_json::from_slice(&value) {
                    contracts.push(c);
                }
        }
        contracts
    }

    // ─── Template contract persistence ───

    pub fn save_template_contracts(&self, contracts: &[&evaporchain_contracts::ContractInstance]) -> Result<(), String> {
        let cf = self.db.cf_handle(CF_TEMPLATE_CONTRACTS).ok_or("missing CF")?;
        for c in contracts {
            let data = serde_json::to_vec(c).map_err(|e| e.to_string())?;
            self.db.put_cf(cf, c.id.to_le_bytes(), data).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn load_all_template_contracts(&self) -> Vec<evaporchain_contracts::ContractInstance> {
        let cf = match self.db.cf_handle(CF_TEMPLATE_CONTRACTS) {
            Some(cf) => cf,
            None => return vec![],
        };
        let mut contracts = vec![];
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);
        for (_key, value) in iter.flatten() {
                if let Ok(c) = serde_json::from_slice(&value) {
                    contracts.push(c);
                }
        }
        contracts
    }
}

// ─── Transaction Receipt ───

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxReceipt {
    pub tx_hash: String,
    pub block_number: u64,
    pub tx_index: u32,
    pub epoch: u64,
    pub timestamp: u64,
    pub tx_type: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub status: String,
    #[serde(default)]
    pub gas_used: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revert_reason: Option<String>,
    #[serde(default)]
    pub log_count: u32,
}

/// Persisted contract event log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractEventLog {
    pub contract_id: u64,
    pub block_number: u64,
    pub log_index: u32,
    pub epoch: u64,
    pub timestamp: u64,
    pub tx_hash: String,
    pub event_name: String,
    pub topics: Vec<String>,
    pub data: Vec<String>,
}

/// Merkle tree over transaction hashes in a block for inclusion proofs.
pub fn compute_tx_merkle_root(txs: &[Transaction]) -> [u8; 32] {
    if txs.is_empty() {
        return [0u8; 32];
    }
    let mut layer: Vec<[u8; 32]> = txs.iter().map(ChainStore::compute_tx_hash).collect();
    while layer.len() > 1 {
        let mut next = Vec::with_capacity(layer.len().div_ceil(2));
        for chunk in layer.chunks(2) {
            if chunk.len() == 2 {
                let mut hasher = blake3::Hasher::new();
                hasher.update(&chunk[0]);
                hasher.update(&chunk[1]);
                next.push(*hasher.finalize().as_bytes());
            } else {
                next.push(chunk[0]);
            }
        }
        layer = next;
    }
    layer[0]
}

/// Transaction inclusion proof: Merkle siblings from leaf to root.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxInclusionProof {
    pub tx_hash: String,
    pub tx_index: usize,
    pub block_number: u64,
    pub merkle_root: String,
    pub siblings: Vec<TxMerkleSibling>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxMerkleSibling {
    pub hash: String,
    pub position: String, // "left" or "right"
}

/// Generate a Merkle inclusion proof for a tx at `tx_index` in the block's txs.
pub fn prove_tx_inclusion(txs: &[Transaction], tx_index: usize, block_number: u64) -> Option<TxInclusionProof> {
    if tx_index >= txs.len() {
        return None;
    }
    let hashes: Vec<[u8; 32]> = txs.iter().map(ChainStore::compute_tx_hash).collect();
    let tx_hash = hashes[tx_index];
    let mut layer = hashes;
    let mut siblings = Vec::new();
    let mut idx = tx_index;

    while layer.len() > 1 {
        let sibling_idx = if idx.is_multiple_of(2) { idx + 1 } else { idx - 1 };
        if sibling_idx < layer.len() {
            siblings.push(TxMerkleSibling {
                hash: hex::encode(layer[sibling_idx]),
                position: if idx.is_multiple_of(2) { "right".into() } else { "left".into() },
            });
        }
        let mut next = Vec::with_capacity(layer.len().div_ceil(2));
        for chunk in layer.chunks(2) {
            if chunk.len() == 2 {
                let mut hasher = blake3::Hasher::new();
                hasher.update(&chunk[0]);
                hasher.update(&chunk[1]);
                next.push(*hasher.finalize().as_bytes());
            } else {
                next.push(chunk[0]);
            }
        }
        layer = next;
        idx /= 2;
    }

    Some(TxInclusionProof {
        tx_hash: hex::encode(tx_hash),
        tx_index,
        block_number,
        merkle_root: hex::encode(layer[0]),
        siblings,
    })
}

/// Verify a tx inclusion proof.
pub fn verify_tx_inclusion(proof: &TxInclusionProof) -> bool {
    let mut current = match hex::decode(&proof.tx_hash) {
        Ok(h) if h.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&h);
            arr
        }
        _ => return false,
    };

    for sibling in &proof.siblings {
        let sibling_hash = match hex::decode(&sibling.hash) {
            Ok(h) if h.len() == 32 => {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&h);
                arr
            }
            _ => return false,
        };
        let mut hasher = blake3::Hasher::new();
        if sibling.position == "left" {
            hasher.update(&sibling_hash);
            hasher.update(&current);
        } else {
            hasher.update(&current);
            hasher.update(&sibling_hash);
        }
        current = *hasher.finalize().as_bytes();
    }

    hex::encode(current) == proof.merkle_root
}

fn tx_type_name(tx: &Transaction) -> &'static str {
    match tx {
        Transaction::Transfer(_) => "transfer",
        Transaction::CreateObject(_) => "create_object",
        Transaction::Refresh(_) => "refresh",
        Transaction::DeployContract(_) => "deploy_contract",
        Transaction::CallContract(_) => "call_contract",
        Transaction::DeployScript(_) => "deploy_script",
        Transaction::CallScript(_) => "call_script",
        Transaction::ValidatorStake(_) => "validator_stake",
        Transaction::ValidatorExit(_) => "validator_exit",
        Transaction::ValidatorClaimStake(_) => "validator_claim_stake",
        Transaction::Shield(_) => "shield",
        Transaction::Unshield(_) => "unshield",
        Transaction::PrivateTransfer(_) => "private_transfer",
        Transaction::Deferred(_) => "deferred",
        Transaction::Blob(_) => "blob",
        Transaction::Governance(_) => "governance",
        Transaction::MultiSig(_) => "multisig",
        Transaction::UserOp(_) => "user_op",
        Transaction::UpgradeContract(_) => "upgrade_contract",
    }
}

fn addr_hex(addr: &[u8; 32]) -> String {
    format!("0x{}", hex::encode(&addr[..4]))
}

fn tx_sender_hex(tx: &Transaction) -> Option<String> {
    match tx {
        Transaction::Transfer(t) => Some(addr_hex(&t.from)),
        Transaction::CreateObject(t) => Some(addr_hex(&t.creator)),
        Transaction::Refresh(_) => None,
        Transaction::DeployContract(t) => Some(addr_hex(&t.deployer)),
        Transaction::CallContract(t) => Some(addr_hex(&t.caller)),
        Transaction::DeployScript(t) => Some(addr_hex(&t.deployer)),
        Transaction::CallScript(t) => Some(addr_hex(&t.caller)),
        Transaction::ValidatorStake(t) => Some(addr_hex(&t.validator_address)),
        Transaction::ValidatorExit(t) => Some(addr_hex(&t.validator_address)),
        Transaction::ValidatorClaimStake(t) => Some(addr_hex(&t.validator_address)),
        Transaction::Shield(_) | Transaction::Unshield(_) | Transaction::PrivateTransfer(_)
        | Transaction::Deferred(_) | Transaction::Blob(_) | Transaction::Governance(_)
        | Transaction::MultiSig(_) | Transaction::UserOp(_) | Transaction::UpgradeContract(_) => None,
    }
}

fn tx_receiver_hex(tx: &Transaction) -> Option<String> {
    match tx {
        Transaction::Transfer(t) => Some(addr_hex(&t.to)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_types::{TransferTx, RefreshTx};

    fn make_transfer(from: [u8; 32], to: [u8; 32], amount: u64) -> Transaction {
        Transaction::Transfer(TransferTx {
            from,
            to,
            amount,
            nonce: 0,
            signature: None,
            public_key: None,
        })
    }

    // ── addr_hex ──

    #[test]
    fn test_addr_hex_truncates_to_4_bytes() {
        let mut addr = [0u8; 32];
        addr[0] = 0xDE;
        addr[1] = 0xAD;
        addr[2] = 0xBE;
        addr[3] = 0xEF;
        assert_eq!(addr_hex(&addr), "0xdeadbeef");
    }

    #[test]
    fn test_addr_hex_all_zeros() {
        assert_eq!(addr_hex(&[0u8; 32]), "0x00000000");
    }

    // ── tx_type_name ──

    #[test]
    fn test_tx_type_name_transfer() {
        let tx = make_transfer([0; 32], [1; 32], 100);
        assert_eq!(tx_type_name(&tx), "transfer");
    }

    #[test]
    fn test_tx_type_name_refresh() {
        let tx = Transaction::Refresh(RefreshTx {
            object_id: [1u8; 32],
            energy_deposit: 100,
            signature: None,
            public_key: None,
        });
        assert_eq!(tx_type_name(&tx), "refresh");
    }

    // ── tx_sender_hex / tx_receiver_hex ──

    #[test]
    fn test_tx_sender_hex_transfer() {
        let mut from = [0u8; 32];
        from[0..4].copy_from_slice(&[0x11, 0x22, 0x33, 0x44]);
        let tx = make_transfer(from, [0; 32], 50);
        assert_eq!(tx_sender_hex(&tx), Some("0x11223344".into()));
    }

    #[test]
    fn test_tx_receiver_hex_transfer() {
        let mut to = [0u8; 32];
        to[0..4].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);
        let tx = make_transfer([0; 32], to, 50);
        assert_eq!(tx_receiver_hex(&tx), Some("0xaabbccdd".into()));
    }

    #[test]
    fn test_tx_sender_hex_refresh_is_none() {
        let tx = Transaction::Refresh(RefreshTx {
            object_id: [0u8; 32],
            energy_deposit: 0,
            signature: None,
            public_key: None,
        });
        assert_eq!(tx_sender_hex(&tx), None);
    }

    #[test]
    fn test_tx_receiver_hex_non_transfer_is_none() {
        let tx = Transaction::Refresh(RefreshTx {
            object_id: [0u8; 32],
            energy_deposit: 0,
            signature: None,
            public_key: None,
        });
        assert_eq!(tx_receiver_hex(&tx), None);
    }

    // ── compute_tx_hash ──

    #[test]
    fn test_compute_tx_hash_deterministic() {
        let tx = make_transfer([1; 32], [2; 32], 1000);
        let h1 = ChainStore::compute_tx_hash(&tx);
        let h2 = ChainStore::compute_tx_hash(&tx);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_compute_tx_hash_different_for_different_txs() {
        let tx1 = make_transfer([1; 32], [2; 32], 100);
        let tx2 = make_transfer([1; 32], [2; 32], 200);
        assert_ne!(ChainStore::compute_tx_hash(&tx1), ChainStore::compute_tx_hash(&tx2));
    }

    // ── compute_tx_merkle_root ──

    #[test]
    fn test_merkle_root_empty() {
        assert_eq!(compute_tx_merkle_root(&[]), [0u8; 32]);
    }

    #[test]
    fn test_merkle_root_single_tx() {
        let tx = make_transfer([1; 32], [2; 32], 100);
        let root = compute_tx_merkle_root(&[tx.clone()]);
        assert_eq!(root, ChainStore::compute_tx_hash(&tx));
    }

    #[test]
    fn test_merkle_root_two_txs() {
        let tx1 = make_transfer([1; 32], [2; 32], 100);
        let tx2 = make_transfer([3; 32], [4; 32], 200);
        let root = compute_tx_merkle_root(&[tx1.clone(), tx2.clone()]);
        let h1 = ChainStore::compute_tx_hash(&tx1);
        let h2 = ChainStore::compute_tx_hash(&tx2);
        let mut hasher = blake3::Hasher::new();
        hasher.update(&h1);
        hasher.update(&h2);
        assert_eq!(root, *hasher.finalize().as_bytes());
    }

    #[test]
    fn test_merkle_root_deterministic() {
        let txs: Vec<_> = (0..5)
            .map(|i| make_transfer([i; 32], [(i + 1); 32], (i as u64) * 100))
            .collect();
        let r1 = compute_tx_merkle_root(&txs);
        let r2 = compute_tx_merkle_root(&txs);
        assert_eq!(r1, r2);
    }

    // ── prove_tx_inclusion / verify_tx_inclusion ──

    #[test]
    fn test_prove_and_verify_inclusion() {
        let txs: Vec<_> = (0..4)
            .map(|i| make_transfer([i; 32], [(i + 10); 32], (i as u64) * 50))
            .collect();
        for idx in 0..txs.len() {
            let proof = prove_tx_inclusion(&txs, idx, 42).unwrap();
            assert!(verify_tx_inclusion(&proof), "failed for index {idx}");
            assert_eq!(proof.tx_index, idx);
            assert_eq!(proof.block_number, 42);
        }
    }

    #[test]
    fn test_prove_inclusion_single_tx() {
        let txs = vec![make_transfer([1; 32], [2; 32], 100)];
        let proof = prove_tx_inclusion(&txs, 0, 1).unwrap();
        assert!(verify_tx_inclusion(&proof));
        assert!(proof.siblings.is_empty());
    }

    #[test]
    fn test_prove_inclusion_out_of_bounds() {
        let txs = vec![make_transfer([1; 32], [2; 32], 100)];
        assert!(prove_tx_inclusion(&txs, 1, 1).is_none());
    }

    #[test]
    fn test_tampered_proof_fails() {
        let txs: Vec<_> = (0..4)
            .map(|i| make_transfer([i; 32], [(i + 10); 32], (i as u64) * 50))
            .collect();
        let mut proof = prove_tx_inclusion(&txs, 0, 42).unwrap();
        proof.tx_hash = hex::encode([0xFF; 32]);
        assert!(!verify_tx_inclusion(&proof));
    }

    #[test]
    fn test_merkle_root_matches_proof_root() {
        let txs: Vec<_> = (0..7)
            .map(|i| make_transfer([i; 32], [(i + 10); 32], (i as u64) * 10))
            .collect();
        let root = compute_tx_merkle_root(&txs);
        let proof = prove_tx_inclusion(&txs, 3, 1).unwrap();
        assert_eq!(hex::encode(root), proof.merkle_root);
    }

    // ── TxReceipt serialization ──

    #[test]
    fn test_tx_receipt_roundtrip() {
        let receipt = TxReceipt {
            tx_hash: "abc123".into(),
            block_number: 10,
            tx_index: 0,
            epoch: 1,
            timestamp: 1000,
            tx_type: "transfer".into(),
            from: Some("0xdead".into()),
            to: Some("0xbeef".into()),
            status: "success".into(),
            gas_used: 21000,
            revert_reason: None,
            log_count: 2,
        };
        let json = serde_json::to_vec(&receipt).unwrap();
        let back: TxReceipt = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.tx_hash, "abc123");
        assert_eq!(back.gas_used, 21000);
        assert_eq!(back.log_count, 2);
    }

    // ── ContractEventLog serialization ──

    #[test]
    fn test_event_log_roundtrip() {
        let log = ContractEventLog {
            contract_id: 42,
            block_number: 100,
            log_index: 0,
            epoch: 5,
            timestamp: 2000,
            tx_hash: "0xfeed".into(),
            event_name: "Transfer".into(),
            topics: vec!["topic1".into()],
            data: vec!["data1".into()],
        };
        let json = serde_json::to_vec(&log).unwrap();
        let back: ContractEventLog = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.contract_id, 42);
        assert_eq!(back.event_name, "Transfer");
    }

    // ── ChainStore integration (tempdir) ──

    #[test]
    fn test_chain_store_consensus_meta_roundtrip() {
        let dir = std::env::temp_dir().join(format!("evap_test_cs_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cs = ChainStore::open(&dir).unwrap();
        cs.save_consensus_meta(42, 5, [0xAA; 32]).unwrap();
        let (bn, ep, ph) = cs.load_consensus_meta().unwrap();
        assert_eq!(bn, 42);
        assert_eq!(ep, 5);
        assert_eq!(ph, [0xAA; 32]);
        drop(cs);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_chain_store_consensus_meta_none_when_empty() {
        let dir = std::env::temp_dir().join(format!("evap_test_cs_empty_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cs = ChainStore::open(&dir).unwrap();
        assert!(cs.load_consensus_meta().is_none());
        drop(cs);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_chain_store_mempool_roundtrip() {
        let dir = std::env::temp_dir().join(format!("evap_test_mp_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cs = ChainStore::open(&dir).unwrap();
        let txs = vec![make_transfer([1; 32], [2; 32], 100)];
        cs.save_mempool(&txs).unwrap();
        let loaded = cs.load_mempool();
        assert_eq!(loaded.len(), 1);
        drop(cs);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_chain_store_snapshot_roundtrip() {
        let dir = std::env::temp_dir().join(format!("evap_test_snap_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cs = ChainStore::open(&dir).unwrap();
        let data = b"snapshot data here";
        cs.save_snapshot(100, data, [0xBB; 32]).unwrap();
        let (h, root, d) = cs.load_latest_snapshot().unwrap();
        assert_eq!(h, 100);
        assert_eq!(root, [0xBB; 32]);
        assert_eq!(d, data);
        drop(cs);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
