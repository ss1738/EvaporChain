//! Persistence layer for non-state chain data (blocks, DeFi stores, consensus metadata).

use crate::api::{
    BlockRecord, ChainStats, DAOStore, EventRecord, NftStore, StakingStore, TokenStore,
};
use rocksdb::{ColumnFamilyDescriptor, Options, DB};
use std::collections::VecDeque;
use std::path::Path;

const CF_BLOCKS: &str = "blocks";
const CF_META: &str = "chain_meta";
const CF_STORES: &str = "stores";

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
        ];

        let db = DB::open_cf_descriptors(&opts, path, cf_descriptors)
            .map_err(|e| format!("Failed to open ChainStore: {}", e))?;

        Ok(Self { db })
    }

    // ─── Consensus metadata ───

    pub fn save_consensus_meta(&self, block_number: u64, epoch: u64, parent_hash: [u8; 32]) {
        let cf = self.db.cf_handle(CF_META).unwrap();
        self.db.put_cf(cf, b"block_number", block_number.to_le_bytes()).unwrap();
        self.db.put_cf(cf, b"epoch", epoch.to_le_bytes()).unwrap();
        self.db.put_cf(cf, b"parent_hash", parent_hash).unwrap();
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

    pub fn save_block(&self, record: &BlockRecord) {
        let cf = self.db.cf_handle(CF_BLOCKS).unwrap();
        let key = record.number.to_be_bytes();
        let value = serde_json::to_vec(record).expect("serialize block record");
        self.db.put_cf(cf, key, value).unwrap();
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
        for item in iter {
            if let Ok((key, _)) = item {
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
    }

    // ─── Chain stats ───

    pub fn save_chain_stats(&self, stats: &ChainStats) {
        let cf = self.db.cf_handle(CF_META).unwrap();
        let value = serde_json::to_vec(stats).expect("serialize chain stats");
        self.db.put_cf(cf, b"chain_stats", value).unwrap();
    }

    pub fn load_chain_stats(&self) -> Option<ChainStats> {
        let cf = self.db.cf_handle(CF_META).unwrap();
        let data = self.db.get_cf(cf, b"chain_stats").ok()??;
        serde_json::from_slice(&data).ok()
    }

    // ─── DeFi stores (serialized as complete JSON blobs) ───

    pub fn save_nft_store(&self, store: &NftStore) {
        let cf = self.db.cf_handle(CF_STORES).unwrap();
        let value = serde_json::to_vec(store).expect("serialize nft store");
        self.db.put_cf(cf, b"nft_store", value).unwrap();
    }

    pub fn load_nft_store(&self) -> Option<NftStore> {
        let cf = self.db.cf_handle(CF_STORES).unwrap();
        let data = self.db.get_cf(cf, b"nft_store").ok()??;
        serde_json::from_slice(&data).ok()
    }

    pub fn save_token_store(&self, store: &TokenStore) {
        let cf = self.db.cf_handle(CF_STORES).unwrap();
        let value = serde_json::to_vec(store).expect("serialize token store");
        self.db.put_cf(cf, b"token_store", value).unwrap();
    }

    pub fn load_token_store(&self) -> Option<TokenStore> {
        let cf = self.db.cf_handle(CF_STORES).unwrap();
        let data = self.db.get_cf(cf, b"token_store").ok()??;
        serde_json::from_slice(&data).ok()
    }

    pub fn save_staking_store(&self, store: &StakingStore) {
        let cf = self.db.cf_handle(CF_STORES).unwrap();
        let value = serde_json::to_vec(store).expect("serialize staking store");
        self.db.put_cf(cf, b"staking_store", value).unwrap();
    }

    pub fn load_staking_store(&self) -> Option<StakingStore> {
        let cf = self.db.cf_handle(CF_STORES).unwrap();
        let data = self.db.get_cf(cf, b"staking_store").ok()??;
        serde_json::from_slice(&data).ok()
    }

    pub fn save_dao_store(&self, store: &DAOStore) {
        let cf = self.db.cf_handle(CF_STORES).unwrap();
        let value = serde_json::to_vec(store).expect("serialize dao store");
        self.db.put_cf(cf, b"dao_store", value).unwrap();
    }

    pub fn load_dao_store(&self) -> Option<DAOStore> {
        let cf = self.db.cf_handle(CF_STORES).unwrap();
        let data = self.db.get_cf(cf, b"dao_store").ok()??;
        serde_json::from_slice(&data).ok()
    }

    // ─── State snapshots ───

    pub fn save_snapshot(&self, height: u64, data: &[u8], state_root: [u8; 32]) {
        let cf = self.db.cf_handle(CF_META).unwrap();
        // Store the snapshot data keyed by height
        let key = format!("snapshot:{}", height);
        self.db.put_cf(cf, key.as_bytes(), data).unwrap();
        // Store metadata: latest snapshot height + state root
        self.db.put_cf(cf, b"snapshot_latest_height", height.to_le_bytes()).unwrap();
        self.db.put_cf(cf, b"snapshot_latest_root", state_root).unwrap();
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

    pub fn save_mempool(&self, txs: &[evaporchain_types::Transaction]) {
        let cf = self.db.cf_handle(CF_META).unwrap();
        let value = serde_json::to_vec(txs).expect("serialize mempool");
        self.db.put_cf(cf, b"mempool", value).unwrap();
    }

    pub fn load_mempool(&self) -> Vec<evaporchain_types::Transaction> {
        let cf = self.db.cf_handle(CF_META).unwrap();
        self.db.get_cf(cf, b"mempool").ok()
            .flatten()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default()
    }

    // ─── Events ───

    pub fn save_events(&self, events: &VecDeque<EventRecord>) {
        let cf = self.db.cf_handle(CF_META).unwrap();
        let value = serde_json::to_vec(events).expect("serialize events");
        self.db.put_cf(cf, b"events", value).unwrap();
    }

    pub fn load_events(&self) -> VecDeque<EventRecord> {
        let cf = self.db.cf_handle(CF_META).unwrap();
        self.db.get_cf(cf, b"events").ok()
            .flatten()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default()
    }
}
