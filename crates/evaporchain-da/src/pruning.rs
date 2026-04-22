//! Temperature-based shard pruning for PoHA certificates.
//!
//! Shards are pruned based on certificate temperature:
//! - Hot: keep all shards (full reconstruction possible)
//! - Warm: keep data shards only (drop parity)
//! - Cold: drop all shards (commitment root + hash only)
//! - Evaporated: nothing survives (ghost record in PoHA store)

use std::collections::BTreeMap;

use crate::block_da::BlockDAPackage;
use crate::poha::{CertTemperature, PoHAStore};

/// Result of a pruning pass.
#[derive(Debug, Default)]
pub struct PruneResult {
    pub shards_pruned: usize,
    pub blocks_fully_pruned: usize,
    pub blocks_parity_pruned: usize,
    pub blocks_retained: usize,
}

/// Prune DA shards based on PoHA certificate temperatures.
///
/// - Hot certificates: keep all shards
/// - Warm certificates: keep only data shards (drop parity indices >= data_shards)
/// - Cold/Evaporated certificates: remove the entire package
///
/// Returns (blocks to remove entirely, blocks needing parity-only prune).
pub fn prune_by_temperature(
    da_store: &mut BTreeMap<u64, BlockDAPackage>,
    poha: &PoHAStore,
) -> PruneResult {
    let mut result = PruneResult::default();
    let mut to_remove: Vec<u64> = Vec::new();
    let mut to_prune_parity: Vec<u64> = Vec::new();

    for (&block_number, package) in da_store.iter() {
        match poha.get(block_number) {
            Some(cert) => {
                match cert.temperature() {
                    CertTemperature::Hot => {
                        result.blocks_retained += 1;
                    }
                    CertTemperature::Warm => {
                        let parity_count = package.shards.len()
                            .saturating_sub(package.header.erasure_config.data_shards);
                        if parity_count > 0 {
                            to_prune_parity.push(block_number);
                            result.shards_pruned += parity_count;
                        } else {
                            result.blocks_retained += 1;
                        }
                    }
                    CertTemperature::Cold | CertTemperature::Evaporated => {
                        result.shards_pruned += package.shards.len();
                        to_remove.push(block_number);
                    }
                }
            }
            None => {
                // No PoHA certificate — block predates PoHA or cert already evaporated
                // Check ghost record
                if poha.get_ghost(block_number).is_some() {
                    result.shards_pruned += package.shards.len();
                    to_remove.push(block_number);
                } else {
                    result.blocks_retained += 1;
                }
            }
        }
    }

    // Remove cold/evaporated blocks entirely
    for bn in &to_remove {
        da_store.remove(bn);
    }
    result.blocks_fully_pruned = to_remove.len();

    // Prune parity shards from warm blocks
    for bn in &to_prune_parity {
        if let Some(package) = da_store.get_mut(bn) {
            let data_shards = package.header.erasure_config.data_shards;
            package.shards.retain(|s| s.index < data_shards);
        }
    }
    result.blocks_parity_pruned = to_prune_parity.len();

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_da::BlockDA;
    use crate::poha::PoHAStore;

    fn make_da_package(block_data: &[u8]) -> BlockDAPackage {
        let da = BlockDA::new().unwrap();
        da.encode_block(block_data).unwrap()
    }

    fn register_cert(store: &mut PoHAStore, block: u64, epoch: u64) {
        store.register(block, [block as u8; 32], 8, 3000, 4000, epoch, vec![], vec![]);
    }

    #[test]
    fn test_hot_blocks_retained() {
        let mut da_store = BTreeMap::new();
        let mut poha = PoHAStore::new(1000, 100);

        da_store.insert(1, make_da_package(b"block 1 data"));
        register_cert(&mut poha, 1, 0);

        let result = prune_by_temperature(&mut da_store, &poha);
        assert_eq!(result.blocks_retained, 1);
        assert_eq!(result.blocks_fully_pruned, 0);
        assert!(da_store.contains_key(&1));
    }

    #[test]
    fn test_warm_blocks_lose_parity() {
        let mut da_store = BTreeMap::new();
        let mut poha = PoHAStore::new(1000, 100);

        da_store.insert(1, make_da_package(b"block 1 warm data"));
        register_cert(&mut poha, 1, 0);
        poha.process_epoch(200); // energy = 250/1000 = 25% → Warm

        let initial_shards = da_store.get(&1).unwrap().shards.len();
        assert_eq!(initial_shards, 8); // 4 data + 4 parity

        let result = prune_by_temperature(&mut da_store, &poha);
        assert_eq!(result.blocks_parity_pruned, 1);
        assert_eq!(result.shards_pruned, 4); // 4 parity shards removed

        let remaining = da_store.get(&1).unwrap().shards.len();
        assert_eq!(remaining, 4); // only data shards
    }

    #[test]
    fn test_cold_blocks_fully_pruned() {
        let mut da_store = BTreeMap::new();
        let mut poha = PoHAStore::new(1000, 100);

        da_store.insert(1, make_da_package(b"block 1 cold data"));
        register_cert(&mut poha, 1, 0);
        poha.process_epoch(300); // energy = 125/1000 = 12.5% → Cold

        let result = prune_by_temperature(&mut da_store, &poha);
        assert_eq!(result.blocks_fully_pruned, 1);
        assert!(!da_store.contains_key(&1));
    }

    #[test]
    fn test_evaporated_blocks_fully_pruned() {
        let mut da_store = BTreeMap::new();
        let mut poha = PoHAStore::new(1000, 100);

        da_store.insert(1, make_da_package(b"block 1 evaporated"));
        register_cert(&mut poha, 1, 0);
        poha.process_epoch(1000); // energy = 0 → Evaporated (moved to ghost)

        // Block 1 is now a ghost
        assert!(poha.get_ghost(1).is_some());
        assert!(poha.get(1).is_none());

        let result = prune_by_temperature(&mut da_store, &poha);
        assert_eq!(result.blocks_fully_pruned, 1);
        assert!(!da_store.contains_key(&1));
    }

    #[test]
    fn test_mixed_temperatures() {
        let mut da_store = BTreeMap::new();
        let mut poha = PoHAStore::new(1000, 100);

        // Block 1: created at epoch 0, will be Warm at epoch 200
        da_store.insert(1, make_da_package(b"block-1"));
        register_cert(&mut poha, 1, 0);

        // Block 2: created at epoch 195, will be Hot at epoch 200
        da_store.insert(2, make_da_package(b"block-2"));
        register_cert(&mut poha, 2, 195);

        // Block 3: created at epoch 0, manually decay further → Cold
        da_store.insert(3, make_da_package(b"block-3"));
        register_cert(&mut poha, 3, 0);

        poha.process_epoch(200);
        // Block 3 at epoch 200: energy = 1000 >> (200/100) = 250 → 25% → Warm
        // Need to decay more for Cold
        poha.process_epoch(300);
        // Block 1: 1000 >> (300/100) = 1000 >> 3 = 125 → 12.5% → Cold
        // Block 2: 1000 >> (105/100) = 1000 >> 1 = 500 → 50% → Hot
        // Block 3: same as block 1 → Cold

        let result = prune_by_temperature(&mut da_store, &poha);
        // Blocks 1 and 3 are Cold → fully pruned
        assert_eq!(result.blocks_fully_pruned, 2);
        // Block 2 is Hot → retained
        assert_eq!(result.blocks_retained, 1);
        assert!(da_store.contains_key(&2));
        assert!(!da_store.contains_key(&1));
        assert!(!da_store.contains_key(&3));
    }
}
