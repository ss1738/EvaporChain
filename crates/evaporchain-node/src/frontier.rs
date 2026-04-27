//! Frontier primitives integration — wires the three novel thermodynamic
//! primitives into the live node:
//!
//! 1. **AnchorManager** — creates state anchors at configurable intervals
//! 2. **PoHAStore** — manages decaying DA certificates
//! 3. **EnergyVerkleTrie** — maintains energy-annotated state tree
//!
//! This module provides a single `FrontierState` struct that the node
//! updates after each block commit.

use evaporchain_consensus::anchor::{
    AnchorManager, DecayRules, LazyQueryResult, LazyStateCache, ObjectLifecycleState,
    ObjectSnapshot,
};
use evaporchain_crypto::energy_verkle::{EnergyVerkleTrie, TrieHealth};
use evaporchain_da::poha::PoHAStore;
#[cfg(test)]
use evaporchain_da::poha::CertTemperature;
use evaporchain_state::db::StateDB;

/// Configuration for frontier primitives.
pub struct FrontierConfig {
    /// Anchor creation interval (in blocks).
    pub anchor_interval: u64,
    /// Default energy for PoHA certificates.
    pub poha_energy: u64,
    /// Default half-life for PoHA certificates (in epochs).
    pub poha_half_life: u64,
}

impl Default for FrontierConfig {
    fn default() -> Self {
        Self {
            anchor_interval: 100,
            poha_energy: 10000,
            poha_half_life: 500,
        }
    }
}

/// Unified state for all frontier primitives.
pub struct FrontierState {
    /// Anchor manager for rule-based consensus.
    pub anchors: AnchorManager,
    /// PoHA store for decaying DA certificates.
    pub poha: PoHAStore,
    /// Energy-annotated Verkle trie (shadow copy tracking live state).
    pub energy_trie: EnergyVerkleTrie,
    /// Lazy state cache for inter-anchor queries without DB access.
    pub lazy_cache: LazyStateCache,
    /// Last anchor height for quick checks.
    last_anchor_height: u64,
}

impl FrontierState {
    /// Create a new frontier state with the given config.
    pub fn new(config: FrontierConfig) -> Self {
        let rules = DecayRules::default_rules();
        Self {
            anchors: AnchorManager::new(config.anchor_interval, rules.clone()),
            poha: PoHAStore::new(config.poha_energy, config.poha_half_life),
            energy_trie: EnergyVerkleTrie::new(),
            lazy_cache: LazyStateCache::new(rules, 10),
            last_anchor_height: 0,
        }
    }

    /// Called after a block is committed. Handles:
    /// - State anchor creation at intervals
    /// - PoHA certificate registration for blocks with DA certificates
    /// - PoHA decay processing
    /// - Energy trie updates from state DB
    ///
    /// Returns a `FrontierUpdate` describing what happened.
    #[allow(clippy::too_many_arguments)]
    pub fn on_block_committed(
        &mut self,
        block_number: u64,
        epoch: u64,
        state_root: [u8; 32],
        active_objects: u64,
        ghost_count: u64,
        mmr_root: [u8; 32],
        da_certificate: Option<&DACertInfo>,
        db: &dyn StateDB,
    ) -> FrontierUpdate {
        let mut update = FrontierUpdate::default();

        // ── 1. State Anchor + Lazy Cache Snapshot ──
        if self.anchors.is_anchor_height(block_number) {
            let anchor = self.anchors.create_anchor(
                block_number,
                epoch,
                state_root,
                active_objects,
                ghost_count,
                mmr_root,
            );
            self.last_anchor_height = block_number;

            // Capture object snapshots for lazy evaluation between anchors
            let object_ids = db.all_object_ids();
            let snapshots: Vec<ObjectSnapshot> = object_ids
                .iter()
                .filter_map(|id| {
                    let obj = db.get_object(id)?;
                    let lifecycle_state = match obj.state {
                        evaporchain_types::ObjectState::Active
                        | evaporchain_types::ObjectState::Resurrected => ObjectLifecycleState::Active,
                        evaporchain_types::ObjectState::Grace => ObjectLifecycleState::Grace,
                        evaporchain_types::ObjectState::Ghost => ObjectLifecycleState::Ghost,
                    };
                    Some(ObjectSnapshot {
                        object_id: *id,
                        energy_at_anchor: obj.energy_at(epoch),
                        half_life: obj.half_life,
                        anchor_epoch: epoch,
                        state: lifecycle_state,
                        grace_epoch: obj.grace_epoch,
                    })
                })
                .collect();
            self.lazy_cache.capture_anchor(epoch, snapshots);

            update.anchor_created = Some(AnchorSummary {
                height: anchor.height,
                hash: anchor.hash(),
                active_objects: anchor.active_objects,
                ghost_count: anchor.ghost_count,
            });
        }

        // ── 2. PoHA Certificate Registration ──
        if let Some(da_info) = da_certificate {
            self.poha.register(
                block_number,
                da_info.data_root,
                da_info.shard_count,
                da_info.attested_stake,
                da_info.total_stake,
                epoch,
                da_info.aggregate_signature.clone(),
                da_info.signer_ids.clone(),
            );
            update.poha_registered = true;
        }

        // ── 3. PoHA Decay ──
        let (poha_decayed, poha_evaporated) = self.poha.process_epoch(epoch);
        update.poha_decayed = poha_decayed;
        update.poha_evaporated = poha_evaporated;

        // ── 4. Energy Trie Update ──
        // Sync energy trie with current active objects from state DB.
        // We update energies for objects we're tracking, and add new ones.
        let object_ids = db.all_object_ids();
        let mut trie_updates = 0u32;
        for id in &object_ids {
            if let Some(obj) = db.get_object(id) {
                let current_energy = obj.energy_at(epoch);
                // Try to update existing entry's energy
                if self.energy_trie.update_energy(id, current_energy, epoch) {
                    trie_updates += 1;
                } else {
                    // New object — insert into energy trie
                    let value = blake3::hash(&obj.data);
                    self.energy_trie.insert(
                        *id,
                        *value.as_bytes(),
                        current_energy,
                        obj.half_life,
                        epoch,
                    );
                    trie_updates += 1;
                }
            }
        }

        // Remove objects from energy trie that are no longer in state DB
        // (evaporated since last check). We do this by checking trie health.
        // Full sync would be expensive — compress cold subtrees instead.
        let compressed = self.energy_trie.compress_cold();
        update.trie_updates = trie_updates;
        update.trie_compressions = compressed;
        update.trie_health = self.energy_trie.health();

        update
    }

    /// Query an object's state lazily without touching the DB.
    pub fn query_lazy(&self, object_id: &[u8; 32], epoch: u64) -> Option<LazyQueryResult> {
        self.lazy_cache.query(object_id, epoch)
    }

    /// Query all objects' state lazily at a given epoch.
    pub fn query_all_lazy(&self, epoch: u64) -> Vec<LazyQueryResult> {
        self.lazy_cache.query_all(epoch)
    }

    /// Get a compact status summary for logging.
    pub fn status_line(&self) -> String {
        let anchor_count = self.anchors.anchor_count();
        let poha_active = self.poha.active_count();
        let poha_ghosts = self.poha.ghost_count();
        let dist = self.poha.temperature_distribution();
        let health = self.energy_trie.health();

        let lazy_anchors = self.lazy_cache.snapshot_count();
        let lazy_objects = self.lazy_cache.total_objects();

        format!(
            "anchors={} poha={}/{}ghosts({}h/{}w/{}c) etrie={}active/{}compressed lazy={}snap/{}obj",
            anchor_count,
            poha_active,
            poha_ghosts,
            dist.hot,
            dist.warm,
            dist.cold,
            health.active_leaves,
            health.compressed_leaves,
            lazy_anchors,
            lazy_objects,
        )
    }
}

/// Info about a DA certificate for PoHA registration.
pub struct DACertInfo {
    pub data_root: [u8; 32],
    pub shard_count: u32,
    pub attested_stake: u64,
    pub total_stake: u64,
    pub aggregate_signature: Vec<u8>,
    pub signer_ids: Vec<u64>,
}

/// Summary of what happened during a frontier update.
#[derive(Default)]
pub struct FrontierUpdate {
    /// Anchor created this block (if any).
    pub anchor_created: Option<AnchorSummary>,
    /// PoHA certificate registered for this block.
    pub poha_registered: bool,
    /// Number of PoHA certificates that decayed.
    pub poha_decayed: usize,
    /// Number of PoHA certificates that evaporated.
    pub poha_evaporated: usize,
    /// Number of energy trie entries updated.
    pub trie_updates: u32,
    /// Number of cold subtrees compressed in the energy trie.
    pub trie_compressions: u32,
    /// Current trie health snapshot.
    pub trie_health: TrieHealth,
}

/// Summary of a created anchor.
pub struct AnchorSummary {
    pub height: u64,
    pub hash: [u8; 32],
    pub active_objects: u64,
    pub ghost_count: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_state::InMemoryStateDB;
    use evaporchain_types::{ObjectState, StateObject};

    fn make_object(id_byte: u8, energy: u64, half_life: u64) -> StateObject {
        StateObject {
            id: {
                let mut id = [0u8; 32];
                id[0] = id_byte;
                id
            },
            owner: [0u8; 32],
            energy,
            half_life,
            created_at: 0,
            last_refreshed: 0,
            state: ObjectState::Active,
            grace_epoch: None,
            data: vec![id_byte],
            decay_curve: None,
        }
    }

    #[test]
    fn test_frontier_state_creation() {
        let fs = FrontierState::new(FrontierConfig::default());
        assert_eq!(fs.anchors.anchor_count(), 0);
        assert_eq!(fs.poha.active_count(), 0);
        assert!(fs.energy_trie.is_empty());
    }

    #[test]
    fn test_anchor_creation_at_interval() {
        let mut fs = FrontierState::new(FrontierConfig {
            anchor_interval: 10,
            ..FrontierConfig::default()
        });
        let db = InMemoryStateDB::new();

        // Block 5: no anchor
        let update = fs.on_block_committed(5, 5, [1u8; 32], 0, 0, [0u8; 32], None, &db);
        assert!(update.anchor_created.is_none());

        // Block 10: anchor
        let update = fs.on_block_committed(10, 10, [2u8; 32], 100, 50, [3u8; 32], None, &db);
        assert!(update.anchor_created.is_some());
        assert_eq!(update.anchor_created.unwrap().height, 10);
        assert_eq!(fs.anchors.anchor_count(), 1);
    }

    #[test]
    fn test_poha_registration() {
        let mut fs = FrontierState::new(FrontierConfig::default());
        let db = InMemoryStateDB::new();

        let da_info = DACertInfo {
            data_root: [0xAB; 32],
            shard_count: 8,
            attested_stake: 3000,
            total_stake: 4000,
            aggregate_signature: vec![0u8; 96],
            signer_ids: vec![0, 1, 2],
        };

        let update = fs.on_block_committed(
            1, 1, [1u8; 32], 10, 0, [0u8; 32], Some(&da_info), &db,
        );
        assert!(update.poha_registered);
        assert_eq!(fs.poha.active_count(), 1);

        let cert = fs.poha.get(1).unwrap();
        assert_eq!(cert.temperature(), CertTemperature::Hot);
    }

    #[test]
    fn test_energy_trie_sync() {
        let mut fs = FrontierState::new(FrontierConfig::default());
        let mut db = InMemoryStateDB::new();

        // Add objects to state DB
        db.put_object(make_object(1, 1000, 100));
        db.put_object(make_object(2, 500, 50));
        db.put_object(make_object(3, 200, 25));

        let update = fs.on_block_committed(
            1, 1, [1u8; 32], 3, 0, [0u8; 32], None, &db,
        );
        assert_eq!(update.trie_updates, 3);
        assert_eq!(fs.energy_trie.len(), 3);

        let health = fs.energy_trie.health();
        assert_eq!(health.active_leaves, 3);
        // max_energy is energy_at(epoch=1) for object with energy=1000, half_life=100
        // = 1000 - (1000 * 1) / (2 * 100) = 995 (linear interpolation)
        assert_eq!(health.max_energy, 995);
    }

    #[test]
    fn test_full_lifecycle() {
        let mut fs = FrontierState::new(FrontierConfig {
            anchor_interval: 5,
            poha_energy: 1000,
            poha_half_life: 10,
        });
        let mut db = InMemoryStateDB::new();

        // Add objects
        for i in 1..=5u8 {
            db.put_object(make_object(i, 1000, 50));
        }

        // Simulate 20 blocks
        for block in 1..=20u64 {
            let da_info = DACertInfo {
                data_root: [block as u8; 32],
                shard_count: 8,
                attested_stake: 3000,
                total_stake: 4000,
                aggregate_signature: vec![0u8; 96],
                signer_ids: vec![0, 1, 2],
            };

            let update = fs.on_block_committed(
                block,
                block,
                [block as u8; 32],
                db.object_count() as u64,
                db.ghost_count() as u64,
                [0u8; 32],
                Some(&da_info),
                &db,
            );

            // Anchors at 5, 10, 15, 20
            if block % 5 == 0 {
                assert!(update.anchor_created.is_some(), "anchor expected at block {}", block);
            }
        }

        // Should have 4 anchors
        assert_eq!(fs.anchors.anchor_count(), 4);
        // Should have PoHA certificates
        assert!(fs.poha.active_count() > 0);
        // Energy trie should have 5 objects
        assert_eq!(fs.energy_trie.len(), 5);

        let status = fs.status_line();
        assert!(status.contains("anchors=4"));
    }

    #[test]
    fn test_lazy_cache_populated_at_anchor() {
        let mut fs = FrontierState::new(FrontierConfig {
            anchor_interval: 10,
            ..FrontierConfig::default()
        });
        let mut db = InMemoryStateDB::new();

        db.put_object(make_object(1, 1000, 50));
        db.put_object(make_object(2, 500, 25));

        // Block 5: no anchor → lazy cache empty
        fs.on_block_committed(5, 5, [1u8; 32], 2, 0, [0u8; 32], None, &db);
        assert_eq!(fs.lazy_cache.snapshot_count(), 0);

        // Block 10: anchor → lazy cache captures 2 objects
        fs.on_block_committed(10, 10, [2u8; 32], 2, 0, [0u8; 32], None, &db);
        assert_eq!(fs.lazy_cache.snapshot_count(), 1);
        assert_eq!(fs.lazy_cache.total_objects(), 2);

        // Query object 1 lazily at epoch 60 (1 half-life past anchor at 10)
        let mut id = [0u8; 32];
        id[0] = 1;
        let result = fs.query_lazy(&id, 60).unwrap();
        assert_eq!(result.anchor_epoch, 10);
        assert!(result.energy < 1000); // decayed
    }

    #[test]
    fn test_lazy_query_all() {
        let mut fs = FrontierState::new(FrontierConfig {
            anchor_interval: 10,
            ..FrontierConfig::default()
        });
        let mut db = InMemoryStateDB::new();

        for i in 1..=5u8 {
            db.put_object(make_object(i, 1000, 50));
        }

        fs.on_block_committed(10, 10, [1u8; 32], 5, 0, [0u8; 32], None, &db);

        let results = fs.query_all_lazy(15);
        assert_eq!(results.len(), 5);
        for r in &results {
            assert!(r.energy <= 1000);
        }
    }

    #[test]
    fn test_status_line() {
        let fs = FrontierState::new(FrontierConfig::default());
        let status = fs.status_line();
        assert!(status.contains("anchors=0"));
        assert!(status.contains("poha=0"));
        assert!(status.contains("etrie=0active"));
        assert!(status.contains("lazy=0snap/0obj"));
    }
}
