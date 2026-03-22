use crate::db::StateDB;
use evaporchain_crypto::hash::blake3_hash;
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
}

/// Engine that processes thermodynamic energy decay and object evaporation.
///
/// Each epoch, the engine:
/// 1. Computes current energy for all objects based on exponential decay
/// 2. Transitions objects with zero energy to Grace state
/// 3. Evaporates objects whose grace period has expired (Grace → Ghost)
/// 4. Creates ghost records (nullifier proofs) for evaporated objects
pub struct EvaporationEngine {
    /// Number of epochs an object stays in Grace before evaporation.
    pub grace_period: u64,
}

impl EvaporationEngine {
    pub fn new(grace_period: u64) -> Self {
        Self { grace_period }
    }

    /// Process all objects for the given epoch.
    ///
    /// This is the core state decay function — called once per epoch.
    /// Objects follow the lifecycle: Active → Grace → Ghost
    pub fn process_epoch(
        &self,
        db: &mut dyn StateDB,
        current_epoch: Epoch,
    ) -> EvaporationResult {
        let mut result = EvaporationResult::default();
        let object_ids = db.all_object_ids();

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
                    // Check if grace period has expired
                    let grace_start = obj.grace_epoch.unwrap_or(current_epoch);
                    if current_epoch >= grace_start + self.grace_period {
                        // Grace period expired → evaporate (Ghost)
                        self.evaporate_object(db, &obj, current_epoch);
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
                    let current_energy = obj.energy_at(current_epoch);

                    if current_energy == 0 {
                        // Energy depleted → enter grace period
                        let obj_mut = db.get_object_mut(&id).unwrap();
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
                        result.decayed += 1;
                    }
                }
            }
        }

        if !result.entered_grace.is_empty() || !result.evaporated.is_empty() {
            info!(
                epoch = current_epoch,
                entered_grace = result.entered_grace.len(),
                evaporated = result.evaporated.len(),
                decayed = result.decayed,
                "Epoch evaporation complete"
            );
        }

        result
    }

    /// Remove an object from active state and create a ghost record.
    fn evaporate_object(
        &self,
        db: &mut dyn StateDB,
        obj: &evaporchain_types::StateObject,
        current_epoch: Epoch,
    ) {
        let data_hash = blake3_hash(&obj.data);

        let ghost = GhostRecord {
            object_id: obj.id,
            owner: obj.owner,
            evaporated_at: current_epoch,
            data_hash,
            original_data: obj.data.clone(),
        };

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
        assert_eq!(ghost.original_data, vec![1, 1]);
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
}
