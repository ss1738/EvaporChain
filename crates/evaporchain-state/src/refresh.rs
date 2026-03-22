use crate::db::StateDB;
use evaporchain_types::{Energy, Epoch, ObjectState, StateObject};
use thiserror::Error;
use tracing::{debug, info};

#[derive(Debug, Error)]
pub enum RefreshError {
    #[error("object {0} not found in active state or ghost records")]
    ObjectNotFound(String),
    #[error("energy deposit must be greater than zero")]
    ZeroEnergy,
    #[error("object {0} is already active with sufficient energy")]
    AlreadyActive(String),
}

/// Engine that handles energy refresh and object resurrection.
///
/// Two operations:
/// 1. **Refresh**: add energy to an Active/Grace object, resetting its decay clock
/// 2. **Resurrect**: bring a Ghost object back to Active state with new energy
pub struct RefreshEngine;

impl RefreshEngine {
    /// Refresh an existing object's energy.
    ///
    /// If the object is Active or Resurrected, adds energy and resets last_refreshed.
    /// If the object is in Grace, restores it to Active with the deposited energy.
    pub fn refresh(
        db: &mut dyn StateDB,
        object_id: &[u8; 32],
        energy_deposit: Energy,
        current_epoch: Epoch,
    ) -> Result<(), RefreshError> {
        if energy_deposit == 0 {
            return Err(RefreshError::ZeroEnergy);
        }

        let obj = db
            .get_object_mut(object_id)
            .ok_or_else(|| RefreshError::ObjectNotFound(hex::encode(object_id)))?;

        match obj.state {
            ObjectState::Active | ObjectState::Resurrected => {
                obj.energy = obj.energy.saturating_add(energy_deposit);
                obj.last_refreshed = current_epoch;
                debug!(
                    object_id = hex::encode(object_id),
                    new_energy = obj.energy,
                    epoch = current_epoch,
                    "Object energy refreshed"
                );
                Ok(())
            }
            ObjectState::Grace => {
                obj.energy = energy_deposit;
                obj.state = ObjectState::Active;
                obj.last_refreshed = current_epoch;
                obj.grace_epoch = None;
                info!(
                    object_id = hex::encode(object_id),
                    energy = energy_deposit,
                    epoch = current_epoch,
                    "Object rescued from grace period (Grace → Active)"
                );
                Ok(())
            }
            ObjectState::Ghost => {
                // Should not have a Ghost object in the active DB — it should be in ghosts
                Err(RefreshError::ObjectNotFound(hex::encode(object_id)))
            }
        }
    }

    /// Resurrect a Ghost object back to Active state.
    ///
    /// Looks up the ghost record, reconstructs the object with new energy,
    /// and removes the ghost record from the accumulator.
    pub fn resurrect(
        db: &mut dyn StateDB,
        object_id: &[u8; 32],
        energy_deposit: Energy,
        current_epoch: Epoch,
    ) -> Result<(), RefreshError> {
        if energy_deposit == 0 {
            return Err(RefreshError::ZeroEnergy);
        }

        // Check if it exists as a ghost
        let ghost = db
            .remove_ghost(object_id)
            .ok_or_else(|| RefreshError::ObjectNotFound(hex::encode(object_id)))?;

        // Reconstruct the object with fresh energy
        let resurrected = StateObject {
            id: ghost.object_id,
            owner: ghost.owner,
            energy: energy_deposit,
            half_life: 100, // Default half-life for resurrected objects
            created_at: ghost.evaporated_at, // Track original creation separately if needed
            last_refreshed: current_epoch,
            state: ObjectState::Resurrected,
            grace_epoch: None,
            data: ghost.original_data,
        };

        db.put_object(resurrected);

        info!(
            object_id = hex::encode(object_id),
            energy = energy_deposit,
            epoch = current_epoch,
            "Object resurrected (Ghost → Resurrected)"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::InMemoryStateDB;
    use evaporchain_types::GhostRecord;

    fn make_active_object(id_byte: u8, energy: u64) -> StateObject {
        StateObject {
            id: {
                let mut id = [0u8; 32];
                id[0] = id_byte;
                id
            },
            owner: [0u8; 32],
            energy,
            half_life: 100,
            created_at: 0,
            last_refreshed: 0,
            state: ObjectState::Active,
            grace_epoch: None,
            data: vec![id_byte],
        }
    }

    fn obj_id(byte: u8) -> [u8; 32] {
        let mut id = [0u8; 32];
        id[0] = byte;
        id
    }

    #[test]
    fn test_refresh_active_object() {
        let mut db = InMemoryStateDB::new();
        db.put_object(make_active_object(1, 500));

        RefreshEngine::refresh(&mut db, &obj_id(1), 300, 50).unwrap();

        let obj = db.get_object(&obj_id(1)).unwrap();
        assert_eq!(obj.energy, 800);
        assert_eq!(obj.last_refreshed, 50);
        assert_eq!(obj.state, ObjectState::Active);
    }

    #[test]
    fn test_refresh_grace_object_rescues_it() {
        let mut db = InMemoryStateDB::new();
        let mut obj = make_active_object(1, 0);
        obj.state = ObjectState::Grace;
        obj.grace_epoch = Some(90);
        db.put_object(obj);

        RefreshEngine::refresh(&mut db, &obj_id(1), 1000, 95).unwrap();

        let obj = db.get_object(&obj_id(1)).unwrap();
        assert_eq!(obj.state, ObjectState::Active);
        assert_eq!(obj.energy, 1000);
        assert_eq!(obj.last_refreshed, 95);
        assert_eq!(obj.grace_epoch, None);
    }

    #[test]
    fn test_refresh_nonexistent_object_fails() {
        let mut db = InMemoryStateDB::new();
        let result = RefreshEngine::refresh(&mut db, &obj_id(99), 100, 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_refresh_zero_energy_fails() {
        let mut db = InMemoryStateDB::new();
        db.put_object(make_active_object(1, 500));
        let result = RefreshEngine::refresh(&mut db, &obj_id(1), 0, 0);
        assert!(matches!(result, Err(RefreshError::ZeroEnergy)));
    }

    #[test]
    fn test_resurrect_ghost_object() {
        let mut db = InMemoryStateDB::new();
        db.put_ghost(GhostRecord {
            object_id: obj_id(1),
            owner: [0u8; 32],
            evaporated_at: 100,
            data_hash: [0u8; 32],
            original_data: vec![1, 2, 3],
        });

        assert_eq!(db.ghost_count(), 1);
        assert_eq!(db.object_count(), 0);

        RefreshEngine::resurrect(&mut db, &obj_id(1), 500, 150).unwrap();

        assert_eq!(db.ghost_count(), 0);
        assert_eq!(db.object_count(), 1);

        let obj = db.get_object(&obj_id(1)).unwrap();
        assert_eq!(obj.state, ObjectState::Resurrected);
        assert_eq!(obj.energy, 500);
        assert_eq!(obj.last_refreshed, 150);
        assert_eq!(obj.data, vec![1, 2, 3]);
    }

    #[test]
    fn test_resurrect_nonexistent_ghost_fails() {
        let mut db = InMemoryStateDB::new();
        let result = RefreshEngine::resurrect(&mut db, &obj_id(99), 100, 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_full_lifecycle_with_resurrection() {
        use crate::evaporation::EvaporationEngine;

        let mut db = InMemoryStateDB::new();
        // Low energy, fast decay
        db.put_object(make_active_object(1, 4));
        let id = obj_id(1);

        let evap = EvaporationEngine::new(3);

        // Epoch 0: Active with energy 4
        assert_eq!(db.get_object(&id).unwrap().state, ObjectState::Active);

        // Fast forward: energy depletes
        // half_life=100, energy=4 → energy_at(200) = 4 >> 2 = 1, energy_at(700) = 0
        let _ = evap.process_epoch(&mut db, 700);
        assert_eq!(db.get_object(&id).unwrap().state, ObjectState::Grace);

        // Grace period expires
        let _ = evap.process_epoch(&mut db, 703);
        assert_eq!(db.object_count(), 0);
        assert_eq!(db.ghost_count(), 1);

        // Resurrect
        RefreshEngine::resurrect(&mut db, &id, 2000, 710).unwrap();
        assert_eq!(db.object_count(), 1);
        assert_eq!(db.ghost_count(), 0);

        let obj = db.get_object(&id).unwrap();
        assert_eq!(obj.state, ObjectState::Resurrected);
        assert_eq!(obj.energy, 2000);
    }
}
