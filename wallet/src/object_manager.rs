use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ObjectManagerError {
    #[error("Object already exists: {0}")]
    ObjectExists(String),
    #[error("Object not found: {0}")]
    ObjectNotFound(String),
    #[error("Object is frozen: {0}")]
    ObjectFrozen(String),
    #[error("Object is not a ghost: {0}")]
    NotGhost(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjLifecycle {
    Active,
    Grace,
    Ghost,
    Evaporated,
    Resurrected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjType {
    Data,
    Contract,
    NFT,
    Token,
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjAction {
    Create,
    Refresh,
    Transfer,
    Freeze,
    Unfreeze,
    Resurrect,
}

// ---------------------------------------------------------------------------
// Domain structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedObject {
    pub id: String,
    pub obj_type: ObjType,
    pub owner: String,
    pub name: String,
    pub energy: u64,
    pub max_energy: u64,
    pub lifecycle: ObjLifecycle,
    pub created_at: String,
    pub last_refreshed: Option<String>,
    pub transfer_count: u32,
    pub metadata: HashMap<String, String>,
    pub frozen: bool,
    pub size_bytes: u64,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectEvent {
    pub object_id: String,
    pub action: ObjAction,
    pub timestamp: String,
    pub details: String,
    pub energy_before: u64,
    pub energy_after: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResurrectionPlan {
    pub object_id: String,
    pub cost: u64,
    pub energy_restored: u64,
    pub viable: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectStats {
    pub total_objects: usize,
    pub active: usize,
    pub grace: usize,
    pub ghost: usize,
    pub evaporated: usize,
    pub frozen: usize,
    pub total_energy: u64,
    pub avg_energy_pct: f64,
    pub total_events: usize,
}

// ---------------------------------------------------------------------------
// ObjectManager
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ObjectManager {
    pub objects: HashMap<String, ManagedObject>,
    pub events: Vec<ObjectEvent>,
}

impl ObjectManager {
    pub fn new() -> Self {
        Self::default()
    }

    // -- CRUD ---------------------------------------------------------------

    pub fn add_object(&mut self, obj: ManagedObject) -> Result<(), ObjectManagerError> {
        if self.objects.contains_key(&obj.id) {
            return Err(ObjectManagerError::ObjectExists(obj.id.clone()));
        }
        let event = ObjectEvent {
            object_id: obj.id.clone(),
            action: ObjAction::Create,
            timestamp: chrono::Utc::now().to_rfc3339(),
            details: format!("Created object '{}'", obj.name),
            energy_before: 0,
            energy_after: obj.energy,
        };
        self.events.push(event);
        self.objects.insert(obj.id.clone(), obj);
        Ok(())
    }

    pub fn remove_object(&mut self, id: &str) -> Result<ManagedObject, ObjectManagerError> {
        self.objects
            .remove(id)
            .ok_or_else(|| ObjectManagerError::ObjectNotFound(id.to_string()))
    }

    pub fn get_object(&self, id: &str) -> Option<&ManagedObject> {
        self.objects.get(id)
    }

    pub fn get_object_mut(&mut self, id: &str) -> Option<&mut ManagedObject> {
        self.objects.get_mut(id)
    }

    // -- Energy / lifecycle -------------------------------------------------

    pub fn refresh_object(
        &mut self,
        id: &str,
        energy_added: u64,
    ) -> Result<(), ObjectManagerError> {
        let obj = self
            .objects
            .get_mut(id)
            .ok_or_else(|| ObjectManagerError::ObjectNotFound(id.to_string()))?;
        if obj.frozen {
            return Err(ObjectManagerError::ObjectFrozen(id.to_string()));
        }
        let before = obj.energy;
        obj.energy = (obj.energy + energy_added).min(obj.max_energy);
        let after = obj.energy;
        obj.last_refreshed = Some(chrono::Utc::now().to_rfc3339());
        self.events.push(ObjectEvent {
            object_id: id.to_string(),
            action: ObjAction::Refresh,
            timestamp: chrono::Utc::now().to_rfc3339(),
            details: format!("Refreshed +{energy_added} energy"),
            energy_before: before,
            energy_after: after,
        });
        Ok(())
    }

    pub fn transfer_object(&mut self, id: &str, new_owner: &str) -> Result<(), ObjectManagerError> {
        let obj = self
            .objects
            .get_mut(id)
            .ok_or_else(|| ObjectManagerError::ObjectNotFound(id.to_string()))?;
        if obj.frozen {
            return Err(ObjectManagerError::ObjectFrozen(id.to_string()));
        }
        let old_owner = obj.owner.clone();
        obj.owner = new_owner.to_string();
        obj.transfer_count += 1;
        self.events.push(ObjectEvent {
            object_id: id.to_string(),
            action: ObjAction::Transfer,
            timestamp: chrono::Utc::now().to_rfc3339(),
            details: format!("Transferred from {old_owner} to {new_owner}"),
            energy_before: obj.energy,
            energy_after: obj.energy,
        });
        Ok(())
    }

    pub fn freeze_object(&mut self, id: &str) -> Result<(), ObjectManagerError> {
        let obj = self
            .objects
            .get_mut(id)
            .ok_or_else(|| ObjectManagerError::ObjectNotFound(id.to_string()))?;
        obj.frozen = true;
        self.events.push(ObjectEvent {
            object_id: id.to_string(),
            action: ObjAction::Freeze,
            timestamp: chrono::Utc::now().to_rfc3339(),
            details: "Object frozen".to_string(),
            energy_before: obj.energy,
            energy_after: obj.energy,
        });
        Ok(())
    }

    pub fn unfreeze_object(&mut self, id: &str) -> Result<(), ObjectManagerError> {
        let obj = self
            .objects
            .get_mut(id)
            .ok_or_else(|| ObjectManagerError::ObjectNotFound(id.to_string()))?;
        obj.frozen = false;
        self.events.push(ObjectEvent {
            object_id: id.to_string(),
            action: ObjAction::Unfreeze,
            timestamp: chrono::Utc::now().to_rfc3339(),
            details: "Object unfrozen".to_string(),
            energy_before: obj.energy,
            energy_after: obj.energy,
        });
        Ok(())
    }

    pub fn mark_ghost(&mut self, id: &str) -> Result<(), ObjectManagerError> {
        let obj = self
            .objects
            .get_mut(id)
            .ok_or_else(|| ObjectManagerError::ObjectNotFound(id.to_string()))?;
        obj.lifecycle = ObjLifecycle::Ghost;
        Ok(())
    }

    pub fn mark_evaporated(&mut self, id: &str) -> Result<(), ObjectManagerError> {
        let obj = self
            .objects
            .get_mut(id)
            .ok_or_else(|| ObjectManagerError::ObjectNotFound(id.to_string()))?;
        obj.lifecycle = ObjLifecycle::Evaporated;
        obj.energy = 0;
        Ok(())
    }

    pub fn resurrect(
        &mut self,
        id: &str,
        energy: u64,
        cost: u64,
    ) -> Result<(), ObjectManagerError> {
        let obj = self
            .objects
            .get_mut(id)
            .ok_or_else(|| ObjectManagerError::ObjectNotFound(id.to_string()))?;
        if obj.lifecycle != ObjLifecycle::Ghost {
            return Err(ObjectManagerError::NotGhost(id.to_string()));
        }
        let before = obj.energy;
        obj.energy = energy.min(obj.max_energy);
        obj.lifecycle = ObjLifecycle::Resurrected;
        self.events.push(ObjectEvent {
            object_id: id.to_string(),
            action: ObjAction::Resurrect,
            timestamp: chrono::Utc::now().to_rfc3339(),
            details: format!("Resurrected with cost={cost}, energy={energy}"),
            energy_before: before,
            energy_after: obj.energy,
        });
        Ok(())
    }

    pub fn plan_resurrection(&self, id: &str) -> Result<ResurrectionPlan, ObjectManagerError> {
        let obj = self
            .objects
            .get(id)
            .ok_or_else(|| ObjectManagerError::ObjectNotFound(id.to_string()))?;
        let cost = 2 * obj.max_energy;
        let viable = obj.lifecycle == ObjLifecycle::Ghost;
        let reason = if !viable {
            Some(format!(
                "Object lifecycle is {:?}, must be Ghost",
                obj.lifecycle
            ))
        } else {
            None
        };
        Ok(ResurrectionPlan {
            object_id: id.to_string(),
            cost,
            energy_restored: obj.max_energy,
            viable,
            reason,
        })
    }

    // -- Query helpers ------------------------------------------------------

    pub fn objects_by_owner(&self, owner: &str) -> Vec<&ManagedObject> {
        self.objects.values().filter(|o| o.owner == owner).collect()
    }

    pub fn objects_by_type(&self, obj_type: &ObjType) -> Vec<&ManagedObject> {
        self.objects
            .values()
            .filter(|o| &o.obj_type == obj_type)
            .collect()
    }

    pub fn objects_by_lifecycle(&self, lifecycle: &ObjLifecycle) -> Vec<&ManagedObject> {
        self.objects
            .values()
            .filter(|o| &o.lifecycle == lifecycle)
            .collect()
    }

    pub fn search_by_tag(&self, tag: &str) -> Vec<&ManagedObject> {
        self.objects
            .values()
            .filter(|o| o.tags.iter().any(|t| t == tag))
            .collect()
    }

    pub fn low_energy_objects(&self, threshold_pct: f64) -> Vec<&ManagedObject> {
        self.objects
            .values()
            .filter(|o| o.max_energy > 0 && (o.energy as f64 / o.max_energy as f64) < threshold_pct)
            .collect()
    }

    pub fn event_history(&self, object_id: &str) -> Vec<&ObjectEvent> {
        self.events
            .iter()
            .filter(|e| e.object_id == object_id)
            .collect()
    }

    pub fn stats(&self) -> ObjectStats {
        let total_objects = self.objects.len();
        let mut active = 0usize;
        let mut grace = 0usize;
        let mut ghost = 0usize;
        let mut evaporated = 0usize;
        let mut frozen = 0usize;
        let mut total_energy = 0u64;
        let mut pct_sum = 0.0f64;

        for obj in self.objects.values() {
            match obj.lifecycle {
                ObjLifecycle::Active => active += 1,
                ObjLifecycle::Grace => grace += 1,
                ObjLifecycle::Ghost => ghost += 1,
                ObjLifecycle::Evaporated => evaporated += 1,
                ObjLifecycle::Resurrected => {}
            }
            if obj.frozen {
                frozen += 1;
            }
            total_energy += obj.energy;
            if obj.max_energy > 0 {
                pct_sum += obj.energy as f64 / obj.max_energy as f64;
            }
        }

        let avg_energy_pct = if total_objects > 0 {
            pct_sum / total_objects as f64
        } else {
            0.0
        };

        ObjectStats {
            total_objects,
            active,
            grace,
            ghost,
            evaporated,
            frozen,
            total_energy,
            avg_energy_pct,
            total_events: self.events.len(),
        }
    }

    // -- Persistence --------------------------------------------------------

    pub fn load(path: &Path) -> Result<Self, ObjectManagerError> {
        let data = std::fs::read_to_string(path)?;
        let mgr: Self = serde_json::from_str(&data)?;
        Ok(mgr)
    }

    pub fn save(&self, path: &Path) -> Result<(), ObjectManagerError> {
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Helper to build a test object
// ---------------------------------------------------------------------------

#[cfg(test)]
fn make_test_object(id: &str, owner: &str, energy: u64, max_energy: u64) -> ManagedObject {
    ManagedObject {
        id: id.to_string(),
        obj_type: ObjType::Data,
        owner: owner.to_string(),
        name: format!("obj-{id}"),
        energy,
        max_energy,
        lifecycle: ObjLifecycle::Active,
        created_at: chrono::Utc::now().to_rfc3339(),
        last_refreshed: None,
        transfer_count: 0,
        metadata: HashMap::new(),
        frozen: false,
        size_bytes: 128,
        tags: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "evaporchain_objmgr_test_{}_{}",
            std::process::id(),
            name
        ))
    }

    // 1
    #[test]
    fn test_new_manager_empty() {
        let mgr = ObjectManager::new();
        assert!(mgr.objects.is_empty());
        assert!(mgr.events.is_empty());
    }

    // 2
    #[test]
    fn test_add_object() {
        let mut mgr = ObjectManager::new();
        let obj = make_test_object("a1", "alice", 100, 200);
        mgr.add_object(obj).unwrap();
        assert_eq!(mgr.objects.len(), 1);
        assert!(mgr.get_object("a1").is_some());
    }

    // 3
    #[test]
    fn test_add_duplicate_errors() {
        let mut mgr = ObjectManager::new();
        mgr.add_object(make_test_object("a1", "alice", 100, 200))
            .unwrap();
        let res = mgr.add_object(make_test_object("a1", "bob", 50, 100));
        assert!(res.is_err());
    }

    // 4
    #[test]
    fn test_remove_object() {
        let mut mgr = ObjectManager::new();
        mgr.add_object(make_test_object("a1", "alice", 100, 200))
            .unwrap();
        let removed = mgr.remove_object("a1").unwrap();
        assert_eq!(removed.id, "a1");
        assert!(mgr.objects.is_empty());
    }

    // 5
    #[test]
    fn test_remove_missing_errors() {
        let mut mgr = ObjectManager::new();
        assert!(mgr.remove_object("nope").is_err());
    }

    // 6
    #[test]
    fn test_get_object_mut() {
        let mut mgr = ObjectManager::new();
        mgr.add_object(make_test_object("a1", "alice", 100, 200))
            .unwrap();
        let obj = mgr.get_object_mut("a1").unwrap();
        obj.name = "renamed".to_string();
        assert_eq!(mgr.get_object("a1").unwrap().name, "renamed");
    }

    // 7
    #[test]
    fn test_refresh_object() {
        let mut mgr = ObjectManager::new();
        mgr.add_object(make_test_object("a1", "alice", 50, 200))
            .unwrap();
        mgr.refresh_object("a1", 30).unwrap();
        let obj = mgr.get_object("a1").unwrap();
        assert_eq!(obj.energy, 80);
        assert!(obj.last_refreshed.is_some());
    }

    // 8
    #[test]
    fn test_refresh_caps_at_max() {
        let mut mgr = ObjectManager::new();
        mgr.add_object(make_test_object("a1", "alice", 190, 200))
            .unwrap();
        mgr.refresh_object("a1", 50).unwrap();
        assert_eq!(mgr.get_object("a1").unwrap().energy, 200);
    }

    // 9
    #[test]
    fn test_refresh_frozen_errors() {
        let mut mgr = ObjectManager::new();
        mgr.add_object(make_test_object("a1", "alice", 50, 200))
            .unwrap();
        mgr.freeze_object("a1").unwrap();
        assert!(mgr.refresh_object("a1", 10).is_err());
    }

    // 10
    #[test]
    fn test_transfer_object() {
        let mut mgr = ObjectManager::new();
        mgr.add_object(make_test_object("a1", "alice", 100, 200))
            .unwrap();
        mgr.transfer_object("a1", "bob").unwrap();
        let obj = mgr.get_object("a1").unwrap();
        assert_eq!(obj.owner, "bob");
        assert_eq!(obj.transfer_count, 1);
    }

    // 11
    #[test]
    fn test_transfer_frozen_errors() {
        let mut mgr = ObjectManager::new();
        mgr.add_object(make_test_object("a1", "alice", 100, 200))
            .unwrap();
        mgr.freeze_object("a1").unwrap();
        assert!(mgr.transfer_object("a1", "bob").is_err());
    }

    // 12
    #[test]
    fn test_freeze_unfreeze() {
        let mut mgr = ObjectManager::new();
        mgr.add_object(make_test_object("a1", "alice", 100, 200))
            .unwrap();
        mgr.freeze_object("a1").unwrap();
        assert!(mgr.get_object("a1").unwrap().frozen);
        mgr.unfreeze_object("a1").unwrap();
        assert!(!mgr.get_object("a1").unwrap().frozen);
    }

    // 13
    #[test]
    fn test_mark_ghost() {
        let mut mgr = ObjectManager::new();
        mgr.add_object(make_test_object("a1", "alice", 100, 200))
            .unwrap();
        mgr.mark_ghost("a1").unwrap();
        assert_eq!(mgr.get_object("a1").unwrap().lifecycle, ObjLifecycle::Ghost);
    }

    // 14
    #[test]
    fn test_mark_evaporated() {
        let mut mgr = ObjectManager::new();
        mgr.add_object(make_test_object("a1", "alice", 100, 200))
            .unwrap();
        mgr.mark_evaporated("a1").unwrap();
        let obj = mgr.get_object("a1").unwrap();
        assert_eq!(obj.lifecycle, ObjLifecycle::Evaporated);
        assert_eq!(obj.energy, 0);
    }

    // 15
    #[test]
    fn test_resurrect() {
        let mut mgr = ObjectManager::new();
        mgr.add_object(make_test_object("a1", "alice", 100, 200))
            .unwrap();
        mgr.mark_ghost("a1").unwrap();
        mgr.resurrect("a1", 150, 400).unwrap();
        let obj = mgr.get_object("a1").unwrap();
        assert_eq!(obj.lifecycle, ObjLifecycle::Resurrected);
        assert_eq!(obj.energy, 150);
    }

    // 16
    #[test]
    fn test_resurrect_non_ghost_errors() {
        let mut mgr = ObjectManager::new();
        mgr.add_object(make_test_object("a1", "alice", 100, 200))
            .unwrap();
        assert!(mgr.resurrect("a1", 100, 400).is_err());
    }

    // 17
    #[test]
    fn test_plan_resurrection_viable() {
        let mut mgr = ObjectManager::new();
        mgr.add_object(make_test_object("a1", "alice", 10, 200))
            .unwrap();
        mgr.mark_ghost("a1").unwrap();
        let plan = mgr.plan_resurrection("a1").unwrap();
        assert!(plan.viable);
        assert_eq!(plan.cost, 400);
        assert_eq!(plan.energy_restored, 200);
        assert!(plan.reason.is_none());
    }

    // 18
    #[test]
    fn test_plan_resurrection_not_viable() {
        let mut mgr = ObjectManager::new();
        mgr.add_object(make_test_object("a1", "alice", 100, 200))
            .unwrap();
        let plan = mgr.plan_resurrection("a1").unwrap();
        assert!(!plan.viable);
        assert!(plan.reason.is_some());
    }

    // 19
    #[test]
    fn test_query_by_owner_type_lifecycle_tag() {
        let mut mgr = ObjectManager::new();
        let mut o1 = make_test_object("a1", "alice", 100, 200);
        o1.tags.push("important".to_string());
        let mut o2 = make_test_object("a2", "bob", 50, 100);
        o2.obj_type = ObjType::NFT;
        mgr.add_object(o1).unwrap();
        mgr.add_object(o2).unwrap();

        assert_eq!(mgr.objects_by_owner("alice").len(), 1);
        assert_eq!(mgr.objects_by_type(&ObjType::NFT).len(), 1);
        assert_eq!(mgr.objects_by_lifecycle(&ObjLifecycle::Active).len(), 2);
        assert_eq!(mgr.search_by_tag("important").len(), 1);
    }

    // 20
    #[test]
    fn test_low_energy_objects() {
        let mut mgr = ObjectManager::new();
        mgr.add_object(make_test_object("a1", "alice", 10, 200))
            .unwrap();
        mgr.add_object(make_test_object("a2", "bob", 150, 200))
            .unwrap();
        let low = mgr.low_energy_objects(0.5);
        assert_eq!(low.len(), 1);
        assert_eq!(low[0].id, "a1");
    }

    // 21
    #[test]
    fn test_event_history_and_stats() {
        let mut mgr = ObjectManager::new();
        mgr.add_object(make_test_object("a1", "alice", 100, 200))
            .unwrap();
        mgr.refresh_object("a1", 10).unwrap();
        mgr.transfer_object("a1", "bob").unwrap();

        let history = mgr.event_history("a1");
        assert_eq!(history.len(), 3); // create + refresh + transfer

        let stats = mgr.stats();
        assert_eq!(stats.total_objects, 1);
        assert_eq!(stats.active, 1);
        assert_eq!(stats.total_events, 3);
    }

    // 22
    #[test]
    fn test_save_load_roundtrip() {
        let path = tmp_path("roundtrip.json");
        let mut mgr = ObjectManager::new();
        mgr.add_object(make_test_object("a1", "alice", 100, 200))
            .unwrap();
        mgr.refresh_object("a1", 20).unwrap();
        mgr.save(&path).unwrap();

        let loaded = ObjectManager::load(&path).unwrap();
        assert_eq!(loaded.objects.len(), 1);
        assert_eq!(loaded.get_object("a1").unwrap().energy, 120);
        assert_eq!(loaded.events.len(), 2);

        let _ = std::fs::remove_file(&path);
    }

    // 23
    #[test]
    fn test_load_or_default_missing_file() {
        let path = tmp_path("nonexistent.json");
        let _ = std::fs::remove_file(&path); // ensure missing
        let mgr = ObjectManager::load_or_default(&path);
        assert!(mgr.objects.is_empty());
    }
}
