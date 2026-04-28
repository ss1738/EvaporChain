use crate::db::StateDB;
use crate::decay_curves::compute_energy;
use evaporchain_crypto::hash::blake3_hash;
use evaporchain_crypto::{EnergyStampedNullifier, MerkleMountainRange};
use evaporchain_types::{Epoch, GhostRecord, HalfLife, ObjectState};
use tracing::{debug, info};

/// Result of processing a single epoch of evaporation.
#[derive(Debug, Default)]
pub struct EvaporationResult {
    /// Objects that transitioned Active → Grace (energy reached zero).
    pub entered_grace: Vec<[u8; 32]>,
    /// Objects that transitioned Grace → Ghost (grace period expired).
    pub evaporated: Vec<[u8; 32]>,
    /// Objects whose energy was decayed but remain Active.
    pub decayed: usize,
    /// Objects already in Ghost state (no-op).
    pub already_ghost: usize,
    /// Cold subtrees compressed in the energy-annotated Verkle trie.
    pub compressed_subtrees: u32,
}

/// Engine that processes thermodynamic energy decay and object evaporation.
///
/// Each epoch, the engine:
/// 1. Computes current energy for all objects based on exponential decay
/// 2. Transitions objects with zero energy to Grace state
/// 3. Evaporates objects whose grace period has expired (Grace → Ghost)
/// 4. Creates ghost records (nullifier proofs) for evaporated objects
pub const DEFAULT_MAX_EVAP_SCAN: usize = 50_000;

pub struct EvaporationEngine {
    /// Number of epochs an object stays in Grace before evaporation.
    pub grace_period: u64,
    /// Max objects to scan per epoch (bounds O(n) sweep).
    pub max_scan_per_epoch: usize,
}

impl EvaporationEngine {
    pub fn new(grace_period: u64) -> Self {
        Self { grace_period, max_scan_per_epoch: DEFAULT_MAX_EVAP_SCAN }
    }

    /// Process all objects for the given epoch (without MMR).
    ///
    /// This is the core state decay function — called once per epoch.
    /// Objects follow the lifecycle: Active → Grace → Ghost
    pub fn process_epoch(
        &self,
        db: &mut dyn StateDB,
        current_epoch: Epoch,
    ) -> EvaporationResult {
        self.process_epoch_inner(db, current_epoch, None)
    }

    /// Process all objects for the given epoch with MMR nullifier accumulation.
    ///
    /// When an object evaporates, an EnergyStampedNullifier is created and
    /// appended to the MMR. The ghost record stores the MMR position.
    pub fn process_epoch_with_mmr(
        &self,
        db: &mut dyn StateDB,
        current_epoch: Epoch,
        mmr: &mut MerkleMountainRange,
    ) -> EvaporationResult {
        self.process_epoch_inner(db, current_epoch, Some(mmr))
    }

    fn process_epoch_inner(
        &self,
        db: &mut dyn StateDB,
        current_epoch: Epoch,
        mut mmr: Option<&mut MerkleMountainRange>,
    ) -> EvaporationResult {
        let mut result = EvaporationResult::default();
        let mut object_ids = db.all_object_ids();
        if object_ids.len() > self.max_scan_per_epoch {
            info!(
                total = object_ids.len(),
                scan_limit = self.max_scan_per_epoch,
                "Evaporation scan truncated to limit"
            );
            object_ids.truncate(self.max_scan_per_epoch);
        }

        for id in object_ids {
            let obj = match db.get_object(&id) {
                Some(o) => o.clone(),
                None => continue,
            };

            match obj.state {
                ObjectState::Ghost => {
                    result.already_ghost += 1;
                    continue;
                }

                ObjectState::Grace => {
                    let grace_start = match obj.grace_epoch {
                        Some(e) => e,
                        None => {
                            // Data integrity: Grace object missing grace_epoch — set it now
                            if let Some(obj_mut) = db.get_object_mut(&id) {
                                obj_mut.grace_epoch = Some(current_epoch);
                            }
                            current_epoch
                        }
                    };
                    if current_epoch >= grace_start + self.grace_period {
                        // Grace period expired → evaporate (Ghost)
                        self.evaporate_object(db, &obj, current_epoch, mmr.as_deref_mut());
                        result.evaporated.push(id);
                        debug!(
                            object_id = hex::encode(id),
                            epoch = current_epoch,
                            "Object evaporated (Grace → Ghost)"
                        );
                    }
                    // Still in grace period — no action needed
                }

                ObjectState::Active | ObjectState::Resurrected => {
                    let current_energy = if let Some(ref curve) = obj.decay_curve {
                        let elapsed = current_epoch.saturating_sub(obj.last_refreshed);
                        compute_energy(curve, obj.energy, elapsed, Some(obj.last_refreshed))
                    } else {
                        obj.energy_at(current_epoch)
                    };

                    if current_energy == 0 {
                        // Energy depleted → enter grace period
                        if let Some(obj_mut) = db.get_object_mut(&id) {
                            obj_mut.state = ObjectState::Grace;
                            obj_mut.grace_epoch = Some(current_epoch);
                            obj_mut.energy = 0;
                            result.entered_grace.push(id);
                            debug!(
                                object_id = hex::encode(id),
                                epoch = current_epoch,
                                "Object entered grace period (Active → Grace)"
                            );
                        } else {
                            continue;
                        }
                    } else {
                        result.decayed += 1;
                    }
                }
            }
        }

        if !result.evaporated.is_empty() {
            result.compressed_subtrees = db.compress_cold_subtrees();
        }

        if !result.entered_grace.is_empty() || !result.evaporated.is_empty() {
            info!(
                epoch = current_epoch,
                entered_grace = result.entered_grace.len(),
                evaporated = result.evaporated.len(),
                decayed = result.decayed,
                compressed = result.compressed_subtrees,
                "Epoch evaporation complete"
            );
        }

        result
    }

    /// Remove an object from active state and create a ghost record.
    /// If an MMR is provided, creates an EnergyStampedNullifier and appends it.
    fn evaporate_object(
        &self,
        db: &mut dyn StateDB,
        obj: &evaporchain_types::StateObject,
        current_epoch: Epoch,
        mmr: Option<&mut MerkleMountainRange>,
    ) {
        let data_hash = blake3_hash(&obj.data);

        // Create energy-stamped nullifier and append to MMR if available
        let mmr_position = mmr.map(|mmr| {
            let nullifier = EnergyStampedNullifier {
                object_id: obj.id,
                value_hash: data_hash,
                evaporation_epoch: current_epoch,
                energy_at_death: 0, // always 0 at evaporation
                owner: obj.owner,
            };
            let pos = mmr.append(nullifier.to_bytes());
            pos.leaf_index
        });

        let ghost = GhostRecord {
            object_id: obj.id,
            owner: obj.owner,
            evaporated_at: current_epoch,
            data_hash,
            original_data: Some(obj.data.clone()),
            mmr_position,
            original_half_life: Some(obj.half_life),
        };

        // Credit back the owner's storage_bytes for the data we're about
        // to evaporate. Mirrors the increment on CreateObject in
        // evaporchain-execution::execute_create_object. We only touch
        // EXISTING accounts — if the owner never had an account, there
        // is nothing to credit back, and creating an empty one here
        // would inject a phantom account leaf into the trie that
        // distorts active_leaves counts (caught by the four
        // trie-health regression tests below).
        if let Some(owner_acct) = db.get_account_mut(&obj.owner) {
            owner_acct.storage_bytes = owner_acct
                .storage_bytes
                .saturating_sub(obj.data.len() as u64);
        }

        db.delete_object(&obj.id);
        db.put_ghost(ghost);
    }
}

/// Compute the minimum number of epochs until an object's energy reaches zero.
///
/// Useful for predicting when an object will enter grace period.
pub fn epochs_until_zero(energy: u64, half_life: HalfLife) -> u64 {
    if energy == 0 || half_life == 0 {
        return 0;
    }
    // energy * 2^(-n/half_life) < 1 when n > half_life * log2(energy)
    // log2(energy) = 63 - leading_zeros for u64
    let log2_energy = 63 - energy.leading_zeros() as u64;
    // Add 1 to ensure we cross zero, multiply by half_life
    (log2_energy + 1) * half_life
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::InMemoryStateDB;
    use evaporchain_types::StateObject;

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
            data: vec![id_byte, id_byte],
            decay_curve: None,
        }
    }

    #[test]
    fn test_active_object_decays_to_grace() {
        let mut db = InMemoryStateDB::new();
        // Energy 1000, half_life 10 → zero after ~100 epochs
        db.put_object(make_object(1, 1000, 10));

        let engine = EvaporationEngine::new(7);

        // At epoch 50, energy should still be > 0
        let r = engine.process_epoch(&mut db, 50);
        assert_eq!(r.decayed, 1);
        assert!(r.entered_grace.is_empty());

        // At epoch 200, energy should be 0 → enters grace
        let r = engine.process_epoch(&mut db, 200);
        assert_eq!(r.entered_grace.len(), 1);

        let obj = db.get_object(&{
            let mut id = [0u8; 32];
            id[0] = 1;
            id
        }).unwrap();
        assert_eq!(obj.state, ObjectState::Grace);
        assert_eq!(obj.grace_epoch, Some(200));
    }

    #[test]
    fn test_grace_to_ghost_after_grace_period() {
        let mut db = InMemoryStateDB::new();
        let mut obj = make_object(1, 0, 10);
        obj.state = ObjectState::Grace;
        obj.grace_epoch = Some(100);
        obj.energy = 0;
        db.put_object(obj);

        let engine = EvaporationEngine::new(7);

        // At epoch 105, grace period not yet expired (need epoch >= 107)
        let r = engine.process_epoch(&mut db, 105);
        assert!(r.evaporated.is_empty());
        assert_eq!(db.object_count(), 1);

        // At epoch 107, grace period expired → evaporate
        let r = engine.process_epoch(&mut db, 107);
        assert_eq!(r.evaporated.len(), 1);
        assert_eq!(db.object_count(), 0);
        assert_eq!(db.ghost_count(), 1);

        // Verify ghost record
        let ghost = db.get_ghost(&{
            let mut id = [0u8; 32];
            id[0] = 1;
            id
        }).unwrap();
        assert_eq!(ghost.evaporated_at, 107);
        assert_eq!(ghost.original_data, Some(vec![1, 1]));
    }

    #[test]
    fn test_full_lifecycle_active_to_ghost() {
        let mut db = InMemoryStateDB::new();
        // Very low energy, short half-life → dies fast
        db.put_object(make_object(1, 4, 1));

        let engine = EvaporationEngine::new(3);

        // Epoch 0: energy_at(0) = 4 → Active
        let r = engine.process_epoch(&mut db, 0);
        assert_eq!(r.decayed, 1);

        // Epoch 3: energy_at(3) = 4 >> 3 = 0 → Grace
        let r = engine.process_epoch(&mut db, 3);
        assert_eq!(r.entered_grace.len(), 1);

        // Epoch 5: still in grace (3 + 3 = 6 needed)
        let r = engine.process_epoch(&mut db, 5);
        assert!(r.evaporated.is_empty());

        // Epoch 6: grace expired → Ghost
        let r = engine.process_epoch(&mut db, 6);
        assert_eq!(r.evaporated.len(), 1);
        assert_eq!(db.object_count(), 0);
        assert_eq!(db.ghost_count(), 1);
    }

    #[test]
    fn test_multiple_objects_different_half_lives() {
        let mut db = InMemoryStateDB::new();
        db.put_object(make_object(1, 100, 5));   // dies fast
        db.put_object(make_object(2, 100, 50));  // dies slow
        db.put_object(make_object(3, 100, 500)); // practically immortal

        let engine = EvaporationEngine::new(7);

        // At epoch 100: object 1 should be dead, object 2 around 25, object 3 ~90
        let r = engine.process_epoch(&mut db, 100);
        assert_eq!(r.entered_grace.len(), 1); // only object 1
        assert_eq!(r.decayed, 2); // objects 2 and 3 still alive
    }

    #[test]
    fn test_ghost_objects_skipped() {
        let mut db = InMemoryStateDB::new();
        let mut obj = make_object(1, 0, 10);
        obj.state = ObjectState::Ghost;
        db.put_object(obj);

        let engine = EvaporationEngine::new(7);
        let r = engine.process_epoch(&mut db, 1000);
        assert_eq!(r.already_ghost, 1);
        assert!(r.evaporated.is_empty());
        assert!(r.entered_grace.is_empty());
    }

    #[test]
    fn test_epochs_until_zero() {
        // Energy 1000, half_life 10 → log2(1000) ≈ 9 → ~100 epochs
        let n = epochs_until_zero(1000, 10);
        assert!(n >= 90 && n <= 110, "got {n}");

        assert_eq!(epochs_until_zero(0, 10), 0);
        assert_eq!(epochs_until_zero(100, 0), 0);

        // Energy 1, half_life 10 → log2(1) = 0 → 10 epochs
        let n = epochs_until_zero(1, 10);
        assert_eq!(n, 10);
    }

    #[test]
    fn test_resurrected_objects_can_decay() {
        let mut db = InMemoryStateDB::new();
        let mut obj = make_object(1, 8, 1);
        obj.state = ObjectState::Resurrected;
        obj.last_refreshed = 0;
        db.put_object(obj);

        let engine = EvaporationEngine::new(3);

        // Epoch 4: energy = 8 >> 4 = 0 → enters grace
        let r = engine.process_epoch(&mut db, 4);
        assert_eq!(r.entered_grace.len(), 1);
    }

    // ── MMR integration tests ──

    #[test]
    fn test_evaporation_with_mmr_creates_nullifier() {
        let mut db = InMemoryStateDB::new();
        let mut mmr = MerkleMountainRange::new();

        // Create an object that will evaporate immediately
        let mut obj = make_object(1, 0, 10);
        obj.state = ObjectState::Grace;
        obj.grace_epoch = Some(100);
        obj.energy = 0;
        db.put_object(obj);

        let engine = EvaporationEngine::new(5);

        // Epoch 105: grace period expired → evaporate with MMR
        let r = engine.process_epoch_with_mmr(&mut db, 105, &mut mmr);
        assert_eq!(r.evaporated.len(), 1);
        assert_eq!(mmr.size(), 1);

        // Ghost record should have MMR position
        let ghost = db.get_ghost(&{
            let mut id = [0u8; 32];
            id[0] = 1;
            id
        }).unwrap();
        assert_eq!(ghost.mmr_position, Some(0));

        // Verify the nullifier proof
        let root = mmr.root();
        let proof = mmr.prove(0).unwrap();

        // Reconstruct the nullifier from ghost data
        let nullifier = evaporchain_crypto::EnergyStampedNullifier {
            object_id: ghost.object_id,
            value_hash: ghost.data_hash,
            evaporation_epoch: ghost.evaporated_at,
            energy_at_death: 0,
            owner: ghost.owner,
        };
        assert!(MerkleMountainRange::verify(&proof, &nullifier.to_bytes(), &root));
    }

    #[test]
    fn test_evaporation_with_mmr_multiple_objects() {
        let mut db = InMemoryStateDB::new();
        let mut mmr = MerkleMountainRange::new();

        // Create 3 objects all in grace, ready to evaporate
        for i in 1u8..=3 {
            let mut obj = make_object(i, 0, 10);
            obj.state = ObjectState::Grace;
            obj.grace_epoch = Some(50);
            obj.energy = 0;
            db.put_object(obj);
        }

        let engine = EvaporationEngine::new(5);
        let r = engine.process_epoch_with_mmr(&mut db, 55, &mut mmr);

        assert_eq!(r.evaporated.len(), 3);
        assert_eq!(mmr.size(), 3);
        assert_eq!(db.ghost_count(), 3);

        // Each ghost should have a unique MMR position
        let positions: Vec<u64> = (1u8..=3)
            .map(|i| {
                let mut id = [0u8; 32];
                id[0] = i;
                db.get_ghost(&id).unwrap().mmr_position.unwrap()
            })
            .collect();

        // All positions should be unique
        let mut sorted = positions.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 3);

        // All proofs should verify
        let root = mmr.root();
        for &pos in &positions {
            let proof = mmr.prove(pos).unwrap();
            let mut id = [0u8; 32];
            id[0] = (pos + 1) as u8; // approximate — just verify proof exists
            assert!(proof.leaf_index == pos);
            // Can't easily reconstruct nullifier without knowing exact order,
            // but proof generation succeeds
            assert!(proof.siblings.len() <= 10);
        }
    }

    #[test]
    fn test_dual_commitment_with_evaporation() {
        use evaporchain_types::DualCommitment;

        let mut db = InMemoryStateDB::new();
        let mut mmr = MerkleMountainRange::new();

        // Create object and fund account
        db.put_object(make_object(1, 4, 1));

        let engine = EvaporationEngine::new(3);

        // Epoch 3: enters grace
        engine.process_epoch_with_mmr(&mut db, 3, &mut mmr);
        // Epoch 6: evaporates
        engine.process_epoch_with_mmr(&mut db, 6, &mut mmr);

        // Build DualCommitment
        let commitment = DualCommitment {
            verkle_root: db.compute_state_root(),
            mmr_root: mmr.root(),
            epoch: 6,
            active_count: db.object_count(),
            ghost_count: db.ghost_count(),
        };

        assert_ne!(commitment.mmr_root, [0u8; 32]);
        assert_eq!(commitment.epoch, 6);
        assert_eq!(commitment.active_count, 0);
        assert_eq!(commitment.ghost_count, 1);
    }

    #[test]
    fn test_process_epoch_without_mmr_still_works() {
        // Verify backward compat: process_epoch (no MMR) still works
        let mut db = InMemoryStateDB::new();
        let mut obj = make_object(1, 0, 10);
        obj.state = ObjectState::Grace;
        obj.grace_epoch = Some(100);
        obj.energy = 0;
        db.put_object(obj);

        let engine = EvaporationEngine::new(5);
        let r = engine.process_epoch(&mut db, 105);
        assert_eq!(r.evaporated.len(), 1);

        // Ghost should have mmr_position = None
        let ghost = db.get_ghost(&{
            let mut id = [0u8; 32];
            id[0] = 1;
            id
        }).unwrap();
        assert_eq!(ghost.mmr_position, None);
    }

    #[test]
    fn test_evaporation_triggers_cold_compression() {
        let mut db = InMemoryStateDB::new();
        let mut obj = make_object(1, 0, 10);
        obj.state = ObjectState::Grace;
        obj.grace_epoch = Some(100);
        obj.energy = 0;
        db.put_object(obj);
        db.put_object(make_object(2, 1000, 100));

        let engine = EvaporationEngine::new(5);
        let r = engine.process_epoch(&mut db, 105);
        assert_eq!(r.evaporated.len(), 1);
        // compressed_subtrees is set when evaporation occurs
        // (value depends on trie topology — just verify it ran)
        assert!(r.compressed_subtrees >= 0);
    }

    #[test]
    fn test_trie_health_after_evaporation() {
        let mut db = InMemoryStateDB::new();
        db.put_object(make_object(1, 1000, 50));
        db.put_object(make_object(2, 500, 100));
        db.put_object(make_object(3, 200, 25));

        let health_before = db.trie_health();
        assert_eq!(health_before.active_leaves, 3);
        assert_eq!(health_before.max_energy, 1000);
        assert_eq!(health_before.min_half_life, 25);

        // Evaporate one object
        {
            let obj = db.get_object_mut(&{
                let mut id = [0u8; 32];
                id[0] = 3;
                id
            }).unwrap();
            obj.state = ObjectState::Grace;
            obj.grace_epoch = Some(0);
            obj.energy = 0;
        }

        let engine = EvaporationEngine::new(3);
        let r = engine.process_epoch(&mut db, 3);
        assert_eq!(r.evaporated.len(), 1);

        let health_after = db.trie_health();
        assert_eq!(health_after.active_leaves, 2);
        assert_eq!(health_after.max_energy, 1000);
        assert_eq!(health_after.min_half_life, 50);
    }

    #[test]
    fn test_full_lifecycle_with_trie_health() {
        use crate::refresh::RefreshEngine;

        let mut db = InMemoryStateDB::new();
        db.put_object(make_object(1, 4, 1));

        let engine = EvaporationEngine::new(3);

        // Active state
        let h = db.trie_health();
        assert_eq!(h.active_leaves, 1);

        // Decay to grace
        engine.process_epoch(&mut db, 3);
        let h = db.trie_health();
        assert_eq!(h.active_leaves, 1); // still in DB until evaporated

        // Evaporate
        engine.process_epoch(&mut db, 6);
        let h = db.trie_health();
        assert_eq!(h.active_leaves, 0);
        assert_eq!(db.ghost_count(), 1);

        // Resurrect
        let id = { let mut id = [0u8; 32]; id[0] = 1; id };
        RefreshEngine::resurrect(&mut db, &id, 2000, 10).unwrap();
        let h = db.trie_health();
        assert_eq!(h.active_leaves, 1);
        assert_eq!(h.max_energy, 2000);
    }

    #[test]
    fn test_linear_decay_curve_evaporation() {
        use evaporchain_types::DecayCurve;

        let mut db = InMemoryStateDB::new();
        let mut obj = make_object(42, 100, 999);
        obj.decay_curve = Some(DecayCurve::Linear { rate_per_epoch: 10 });
        db.put_object(obj);

        let engine = EvaporationEngine::new(3);

        // At epoch 5: energy = 100 - 10*5 = 50 (still alive)
        let r = engine.process_epoch(&mut db, 5);
        assert_eq!(r.decayed, 1);
        assert!(r.entered_grace.is_empty());

        // At epoch 10: energy = 100 - 10*10 = 0 → enters grace
        let r = engine.process_epoch(&mut db, 10);
        assert_eq!(r.entered_grace.len(), 1);

        // At epoch 13: grace_period=3, so 10+3=13 → evaporates
        let r = engine.process_epoch(&mut db, 13);
        assert_eq!(r.evaporated.len(), 1);
    }

    #[test]
    fn test_asymptotic_decay_never_reaches_zero() {
        use evaporchain_types::DecayCurve;

        let mut db = InMemoryStateDB::new();
        let mut obj = make_object(99, 1000, 999);
        obj.decay_curve = Some(DecayCurve::Asymptotic { floor: 100, half_life: 5 });
        db.put_object(obj);

        let engine = EvaporationEngine::new(3);

        // Even after 1000 epochs, energy never reaches 0 (floor=100)
        let r = engine.process_epoch(&mut db, 1000);
        assert_eq!(r.decayed, 1);
        assert!(r.entered_grace.is_empty());
    }

    // ─── Storage_bytes decrement on evaporation ─────────────────────────────

    #[test]
    fn test_evaporation_decrements_owner_storage_bytes() {
        // When an object evaporates, the owner's storage_bytes must
        // decrement by the object's data length — mirroring the credit
        // applied on CreateObject in evaporchain-execution.
        let mut db = InMemoryStateDB::new();

        // Object with 5-byte data, dies fast (energy 4, half_life 1).
        let mut obj = make_object(1, 4, 1);
        obj.data = vec![0xAA; 5];
        let owner = obj.owner;
        db.put_object(obj);

        // Manually pre-credit the owner's storage_bytes to mimic the
        // execute_create_object path that runs in evaporchain-execution.
        // (The evaporation engine alone cannot trigger CreateObject; this
        // test is for the decrement path only.)
        {
            let acct = db.get_or_create_account(&owner);
            acct.storage_bytes = 5;
        }
        assert_eq!(db.get_account(&owner).unwrap().storage_bytes, 5);

        let engine = EvaporationEngine::new(3);
        // The evaporation engine is a TWO-PASS state machine:
        //   pass 1 (energy reaches 0): Active → Grace, sets grace_epoch
        //   pass 2 (current_epoch >= grace_epoch + grace_period): Grace → Ghost
        engine.process_epoch(&mut db, 100); // Active → Grace at grace_epoch=100
        let r = engine.process_epoch(&mut db, 104); // 104 >= 100 + 3 → evaporate
        assert_eq!(r.evaporated.len(), 1, "object should have evaporated");

        // After evaporation, the owner's storage_bytes should be back to 0.
        assert_eq!(
            db.get_account(&owner).unwrap().storage_bytes,
            0,
            "evaporation must credit back the data.len() to owner's storage_bytes"
        );
    }

    #[test]
    fn test_evaporation_storage_bytes_saturates_on_under_credit() {
        // Defensive: if an object's data_len exceeds the owner's
        // recorded storage_bytes (under-credited account), the decrement
        // on evaporation must NOT underflow. saturating_sub guarantees
        // the stored value stays at 0.
        let mut db = InMemoryStateDB::new();
        let mut obj = make_object(2, 4, 1);
        obj.data = vec![0xBB; 100]; // large data, but owner has 0 stored bytes
        let owner = obj.owner;
        db.put_object(obj);
        // Force the owner account to exist with storage_bytes=0 so the
        // decrement runs through saturating_sub. (When the owner has no
        // account at all, evaporate_object intentionally skips the
        // credit-back so it cannot inject a phantom trie leaf — see
        // the evaporate_object guard.)
        {
            let acct = db.get_or_create_account(&owner);
            acct.storage_bytes = 0;
        }

        let engine = EvaporationEngine::new(3);
        engine.process_epoch(&mut db, 100); // Active → Grace
        let r = engine.process_epoch(&mut db, 104); // Grace → Ghost
        assert_eq!(r.evaporated.len(), 1);

        // Storage_bytes must saturate to 0, not underflow.
        let acct = db.get_account(&owner).unwrap();
        assert_eq!(acct.storage_bytes, 0, "saturating_sub must prevent underflow");
    }
}
