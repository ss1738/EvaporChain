// wallet/src/key_rotation.rs — Automated key rotation system
//
// Lifecycle management for cryptographic keys:
//   - Track key status (active, rotated, compromised, expired, revoked)
//   - Automatic rotation based on configurable policies
//   - Full rotation history with predecessor/successor chains
//   - Persistent JSON store

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;

static KEY_COUNTER: AtomicU64 = AtomicU64::new(0);

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum KeyRotationError {
    #[error("key already exists: {0}")]
    AlreadyExists(String),
    #[error("key not found: {0}")]
    NotFound(String),
    #[error("invalid state: {0}")]
    InvalidState(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ──────────────────────────── Enums ────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyStatus {
    Active,
    Rotated,
    Compromised,
    Expired,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RotationReason {
    Scheduled,
    Manual,
    SecurityIncident,
    PolicyCompliance,
    KeyAging,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyType {
    Signing,
    Encryption,
    Authentication,
    Derivation,
}

// ──────────────────────────── ManagedKey ────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedKey {
    pub id: String,
    pub key_type: KeyType,
    pub public_key: String,
    pub status: KeyStatus,
    pub created_at: String,
    pub rotated_at: Option<String>,
    pub expires_at: Option<String>,
    pub derivation_path: Option<String>,
    pub successor_id: Option<String>,
    pub predecessor_id: Option<String>,
    pub usage_count: u64,
    pub last_used: Option<String>,
    pub metadata: HashMap<String, String>,
}

impl ManagedKey {
    pub fn new(id: &str, key_type: KeyType, public_key: &str) -> Self {
        Self {
            id: id.to_string(),
            key_type,
            public_key: public_key.to_string(),
            status: KeyStatus::Active,
            created_at: chrono::Utc::now().to_rfc3339(),
            rotated_at: None,
            expires_at: None,
            derivation_path: None,
            successor_id: None,
            predecessor_id: None,
            usage_count: 0,
            last_used: None,
            metadata: HashMap::new(),
        }
    }

    pub fn with_expiry(mut self, expires_at: &str) -> Self {
        self.expires_at = Some(expires_at.to_string());
        self
    }

    pub fn with_derivation_path(mut self, path: &str) -> Self {
        self.derivation_path = Some(path.to_string());
        self
    }

    pub fn is_active(&self) -> bool {
        self.status == KeyStatus::Active
    }

    pub fn is_expired(&self) -> bool {
        if let Some(ref exp) = self.expires_at {
            if let Ok(exp_dt) = chrono::DateTime::parse_from_rfc3339(exp) {
                return exp_dt < chrono::Utc::now();
            }
        }
        false
    }

    /// Returns the number of whole days since the key was created.
    pub fn age_days(&self) -> u64 {
        if let Ok(created) = chrono::DateTime::parse_from_rfc3339(&self.created_at) {
            let dur = chrono::Utc::now().signed_duration_since(created);
            dur.num_days().max(0) as u64
        } else {
            0
        }
    }

    pub fn record_usage(&mut self) {
        self.usage_count += 1;
        self.last_used = Some(chrono::Utc::now().to_rfc3339());
    }

    pub fn mark_rotated(&mut self, successor_id: &str) {
        self.status = KeyStatus::Rotated;
        self.rotated_at = Some(chrono::Utc::now().to_rfc3339());
        self.successor_id = Some(successor_id.to_string());
    }

    pub fn mark_compromised(&mut self) {
        self.status = KeyStatus::Compromised;
    }

    pub fn mark_revoked(&mut self) {
        self.status = KeyStatus::Revoked;
    }
}

// ──────────────────────────── RotationEvent ────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationEvent {
    pub id: String,
    pub key_id: String,
    pub old_key_id: Option<String>,
    pub new_key_id: String,
    pub reason: RotationReason,
    pub performed_at: String,
    pub notes: String,
}

impl RotationEvent {
    pub fn new(id: &str, key_id: &str, new_key_id: &str, reason: RotationReason) -> Self {
        Self {
            id: id.to_string(),
            key_id: key_id.to_string(),
            old_key_id: Some(key_id.to_string()),
            new_key_id: new_key_id.to_string(),
            reason,
            performed_at: chrono::Utc::now().to_rfc3339(),
            notes: String::new(),
        }
    }

    pub fn with_notes(mut self, notes: &str) -> Self {
        self.notes = notes.to_string();
        self
    }
}

// ──────────────────────────── RotationPolicy ───────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationPolicy {
    pub key_type: KeyType,
    pub max_age_days: u64,
    pub max_usage_count: u64,
    pub auto_rotate: bool,
    pub notify_before_days: u64,
}

impl RotationPolicy {
    pub fn new(key_type: KeyType, max_age_days: u64) -> Self {
        Self {
            key_type,
            max_age_days,
            max_usage_count: u64::MAX,
            auto_rotate: false,
            notify_before_days: 7,
        }
    }

    pub fn with_max_usage(mut self, count: u64) -> Self {
        self.max_usage_count = count;
        self
    }

    pub fn with_auto_rotate(mut self) -> Self {
        self.auto_rotate = true;
        self
    }

    pub fn with_notify_before(mut self, days: u64) -> Self {
        self.notify_before_days = days;
        self
    }

    /// Returns true if the key exceeds max age or max usage.
    pub fn needs_rotation(&self, key: &ManagedKey) -> bool {
        key.age_days() > self.max_age_days || key.usage_count > self.max_usage_count
    }

    /// Returns true if the key is approaching its rotation deadline.
    pub fn needs_notification(&self, key: &ManagedKey) -> bool {
        if self.max_age_days >= self.notify_before_days {
            key.age_days() > self.max_age_days - self.notify_before_days
        } else {
            true
        }
    }
}

// ──────────────────────────── RotationStats ────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationStats {
    pub total_keys: usize,
    pub active: usize,
    pub rotated: usize,
    pub compromised: usize,
    pub revoked: usize,
    pub total_rotations: usize,
    pub policies: usize,
}

// ──────────────────────────── KeyRotationManager ───────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KeyRotationManager {
    pub keys: HashMap<String, ManagedKey>,
    pub rotation_history: Vec<RotationEvent>,
    pub policies: Vec<RotationPolicy>,
}

impl KeyRotationManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_key(&mut self, key: ManagedKey) -> Result<(), KeyRotationError> {
        if self.keys.contains_key(&key.id) {
            return Err(KeyRotationError::AlreadyExists(key.id.clone()));
        }
        self.keys.insert(key.id.clone(), key);
        Ok(())
    }

    pub fn remove_key(&mut self, id: &str) -> Result<ManagedKey, KeyRotationError> {
        self.keys
            .remove(id)
            .ok_or_else(|| KeyRotationError::NotFound(id.to_string()))
    }

    pub fn get_key(&self, id: &str) -> Option<&ManagedKey> {
        self.keys.get(id)
    }

    pub fn get_key_mut(&mut self, id: &str) -> Option<&mut ManagedKey> {
        self.keys.get_mut(id)
    }

    pub fn active_keys(&self) -> Vec<&ManagedKey> {
        self.keys.values().filter(|k| k.is_active()).collect()
    }

    pub fn by_type(&self, key_type: &KeyType) -> Vec<&ManagedKey> {
        self.keys
            .values()
            .filter(|k| &k.key_type == key_type)
            .collect()
    }

    pub fn by_status(&self, status: &KeyStatus) -> Vec<&ManagedKey> {
        self.keys
            .values()
            .filter(|k| &k.status == status)
            .collect()
    }

    /// Rotate a key: create a new key, link old->new, mark old as rotated,
    /// record the event. Returns the new key's ID.
    pub fn rotate_key(
        &mut self,
        key_id: &str,
        new_public_key: &str,
        reason: RotationReason,
    ) -> Result<String, KeyRotationError> {
        // Verify old key exists
        let old_key = self
            .keys
            .get(key_id)
            .ok_or_else(|| KeyRotationError::NotFound(key_id.to_string()))?;

        if !old_key.is_active() {
            return Err(KeyRotationError::InvalidState(format!(
                "key {} is not active",
                key_id
            )));
        }

        let key_type = old_key.key_type;
        let counter = KEY_COUNTER.fetch_add(1, Ordering::SeqCst);
        let ts = chrono::Utc::now().timestamp_millis();
        let new_id = format!("key_{}_{}", ts, counter);

        // Create new key with predecessor link
        let mut new_key = ManagedKey::new(&new_id, key_type, new_public_key);
        new_key.predecessor_id = Some(key_id.to_string());

        // Mark old key as rotated
        let old_key_mut = self.keys.get_mut(key_id).unwrap();
        old_key_mut.mark_rotated(&new_id);

        // Insert new key
        self.keys.insert(new_id.clone(), new_key);

        // Record rotation event
        let event_id = format!("rot_{}_{}", ts, counter);
        let event = RotationEvent::new(&event_id, key_id, &new_id, reason);
        self.rotation_history.push(event);

        // Cap history at 500
        if self.rotation_history.len() > 500 {
            let excess = self.rotation_history.len() - 500;
            self.rotation_history.drain(..excess);
        }

        Ok(new_id)
    }

    pub fn add_policy(&mut self, policy: RotationPolicy) {
        self.policies.push(policy);
    }

    /// Returns (key_id, reason) for every active key that needs rotation per policy.
    pub fn check_policies(&self) -> Vec<(String, String)> {
        let mut results = Vec::new();
        for key in self.keys.values() {
            if !key.is_active() {
                continue;
            }
            for policy in &self.policies {
                if policy.key_type == key.key_type && policy.needs_rotation(key) {
                    let reason = if key.age_days() > policy.max_age_days {
                        format!("age {} days exceeds max {}", key.age_days(), policy.max_age_days)
                    } else {
                        format!(
                            "usage {} exceeds max {}",
                            key.usage_count, policy.max_usage_count
                        )
                    };
                    results.push((key.id.clone(), reason));
                    break; // one reason per key is enough
                }
            }
        }
        results
    }

    /// Keys approaching rotation deadline per their matching policy.
    pub fn keys_needing_notification(&self) -> Vec<&ManagedKey> {
        self.keys
            .values()
            .filter(|key| {
                if !key.is_active() {
                    return false;
                }
                self.policies
                    .iter()
                    .any(|p| p.key_type == key.key_type && p.needs_notification(key))
            })
            .collect()
    }

    pub fn compromised_keys(&self) -> Vec<&ManagedKey> {
        self.by_status(&KeyStatus::Compromised)
    }

    /// Follow the predecessor chain backwards to build the full key history.
    pub fn key_chain(&self, key_id: &str) -> Vec<&ManagedKey> {
        let mut chain = Vec::new();
        let mut current_id = Some(key_id.to_string());

        // First, walk backwards to find the root
        let mut ids_back = Vec::new();
        let mut visit = key_id.to_string();
        loop {
            if let Some(key) = self.keys.get(&visit) {
                ids_back.push(visit.clone());
                if let Some(ref pred) = key.predecessor_id {
                    visit = pred.clone();
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        // Reverse so chain goes oldest -> newest
        ids_back.reverse();
        for id in &ids_back {
            if let Some(key) = self.keys.get(id) {
                chain.push(key);
            }
        }

        // Also walk forward from the original key via successors
        current_id = self
            .keys
            .get(key_id)
            .and_then(|k| k.successor_id.clone());
        while let Some(ref id) = current_id {
            if ids_back.contains(id) {
                break; // avoid loops
            }
            if let Some(key) = self.keys.get(id) {
                chain.push(key);
                current_id = key.successor_id.clone();
            } else {
                break;
            }
        }

        chain
    }

    pub fn rotation_count(&self) -> usize {
        self.rotation_history.len()
    }

    pub fn recent_rotations(&self, n: usize) -> Vec<&RotationEvent> {
        self.rotation_history.iter().rev().take(n).collect()
    }

    pub fn stats(&self) -> RotationStats {
        RotationStats {
            total_keys: self.keys.len(),
            active: self.by_status(&KeyStatus::Active).len(),
            rotated: self.by_status(&KeyStatus::Rotated).len(),
            compromised: self.by_status(&KeyStatus::Compromised).len(),
            revoked: self.by_status(&KeyStatus::Revoked).len(),
            total_rotations: self.rotation_history.len(),
            policies: self.policies.len(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), KeyRotationError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, KeyRotationError> {
        let data = std::fs::read_to_string(path)?;
        let mgr: Self = serde_json::from_str(&data)?;
        Ok(mgr)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ──────────────────────────── Tests ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("key_rot_test_{}_{}", std::process::id(), name))
    }

    fn make_manager() -> KeyRotationManager {
        KeyRotationManager::new()
    }

    #[test]
    fn test_add_and_get_key() {
        let mut mgr = make_manager();
        let key = ManagedKey::new("k1", KeyType::Signing, "pub_abc");
        mgr.add_key(key).unwrap();
        let fetched = mgr.get_key("k1").unwrap();
        assert_eq!(fetched.public_key, "pub_abc");
        assert_eq!(fetched.key_type, KeyType::Signing);
    }

    #[test]
    fn test_add_duplicate_rejected() {
        let mut mgr = make_manager();
        mgr.add_key(ManagedKey::new("k1", KeyType::Signing, "pub1"))
            .unwrap();
        let err = mgr
            .add_key(ManagedKey::new("k1", KeyType::Signing, "pub2"))
            .unwrap_err();
        assert!(matches!(err, KeyRotationError::AlreadyExists(_)));
    }

    #[test]
    fn test_remove_key() {
        let mut mgr = make_manager();
        mgr.add_key(ManagedKey::new("k1", KeyType::Signing, "pub1"))
            .unwrap();
        let removed = mgr.remove_key("k1").unwrap();
        assert_eq!(removed.id, "k1");
        assert!(mgr.get_key("k1").is_none());
    }

    #[test]
    fn test_key_is_active() {
        let key = ManagedKey::new("k1", KeyType::Signing, "pub1");
        assert!(key.is_active());
    }

    #[test]
    fn test_key_expired() {
        let key = ManagedKey::new("k1", KeyType::Signing, "pub1")
            .with_expiry("2020-01-01T00:00:00+00:00");
        assert!(key.is_expired());
    }

    #[test]
    fn test_key_age_days() {
        let key = ManagedKey::new("k1", KeyType::Signing, "pub1");
        // Just created — age should be 0
        assert_eq!(key.age_days(), 0);
    }

    #[test]
    fn test_record_usage() {
        let mut key = ManagedKey::new("k1", KeyType::Signing, "pub1");
        assert_eq!(key.usage_count, 0);
        assert!(key.last_used.is_none());
        key.record_usage();
        assert_eq!(key.usage_count, 1);
        assert!(key.last_used.is_some());
        key.record_usage();
        assert_eq!(key.usage_count, 2);
    }

    #[test]
    fn test_rotate_key() {
        let mut mgr = make_manager();
        mgr.add_key(ManagedKey::new("k1", KeyType::Signing, "pub1"))
            .unwrap();

        let new_id = mgr
            .rotate_key("k1", "pub2", RotationReason::Manual)
            .unwrap();

        assert!(new_id.starts_with("key_"));
        // Old key should be rotated
        let old = mgr.get_key("k1").unwrap();
        assert_eq!(old.status, KeyStatus::Rotated);
        // New key should be active
        let new_key = mgr.get_key(&new_id).unwrap();
        assert!(new_key.is_active());
        assert_eq!(new_key.public_key, "pub2");
    }

    #[test]
    fn test_rotate_links_successor() {
        let mut mgr = make_manager();
        mgr.add_key(ManagedKey::new("k1", KeyType::Signing, "pub1"))
            .unwrap();

        let new_id = mgr
            .rotate_key("k1", "pub2", RotationReason::Scheduled)
            .unwrap();

        let old = mgr.get_key("k1").unwrap();
        assert_eq!(old.successor_id.as_deref(), Some(new_id.as_str()));
    }

    #[test]
    fn test_rotate_links_predecessor() {
        let mut mgr = make_manager();
        mgr.add_key(ManagedKey::new("k1", KeyType::Signing, "pub1"))
            .unwrap();

        let new_id = mgr
            .rotate_key("k1", "pub2", RotationReason::Scheduled)
            .unwrap();

        let new_key = mgr.get_key(&new_id).unwrap();
        assert_eq!(new_key.predecessor_id.as_deref(), Some("k1"));
    }

    #[test]
    fn test_mark_compromised() {
        let mut key = ManagedKey::new("k1", KeyType::Signing, "pub1");
        key.mark_compromised();
        assert_eq!(key.status, KeyStatus::Compromised);
    }

    #[test]
    fn test_mark_revoked() {
        let mut key = ManagedKey::new("k1", KeyType::Signing, "pub1");
        key.mark_revoked();
        assert_eq!(key.status, KeyStatus::Revoked);
    }

    #[test]
    fn test_active_keys() {
        let mut mgr = make_manager();
        mgr.add_key(ManagedKey::new("k1", KeyType::Signing, "pub1"))
            .unwrap();
        mgr.add_key(ManagedKey::new("k2", KeyType::Encryption, "pub2"))
            .unwrap();
        mgr.get_key_mut("k2").unwrap().mark_revoked();

        let active = mgr.active_keys();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "k1");
    }

    #[test]
    fn test_by_type() {
        let mut mgr = make_manager();
        mgr.add_key(ManagedKey::new("k1", KeyType::Signing, "pub1"))
            .unwrap();
        mgr.add_key(ManagedKey::new("k2", KeyType::Encryption, "pub2"))
            .unwrap();
        mgr.add_key(ManagedKey::new("k3", KeyType::Signing, "pub3"))
            .unwrap();

        let signing = mgr.by_type(&KeyType::Signing);
        assert_eq!(signing.len(), 2);
    }

    #[test]
    fn test_by_status() {
        let mut mgr = make_manager();
        mgr.add_key(ManagedKey::new("k1", KeyType::Signing, "pub1"))
            .unwrap();
        mgr.add_key(ManagedKey::new("k2", KeyType::Signing, "pub2"))
            .unwrap();
        mgr.get_key_mut("k2").unwrap().mark_compromised();

        let compromised = mgr.by_status(&KeyStatus::Compromised);
        assert_eq!(compromised.len(), 1);
        assert_eq!(compromised[0].id, "k2");
    }

    #[test]
    fn test_policy_needs_rotation() {
        let policy = RotationPolicy::new(KeyType::Signing, 30).with_max_usage(100);

        let mut key = ManagedKey::new("k1", KeyType::Signing, "pub1");
        // Fresh key — should not need rotation
        assert!(!policy.needs_rotation(&key));

        // Exceed usage
        key.usage_count = 101;
        assert!(policy.needs_rotation(&key));
    }

    #[test]
    fn test_check_policies() {
        let mut mgr = make_manager();
        let mut key = ManagedKey::new("k1", KeyType::Signing, "pub1");
        key.usage_count = 200;
        mgr.add_key(key).unwrap();

        mgr.add_policy(RotationPolicy::new(KeyType::Signing, 90).with_max_usage(100));

        let results = mgr.check_policies();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "k1");
        assert!(results[0].1.contains("usage"));
    }

    #[test]
    fn test_key_chain() {
        let mut mgr = make_manager();
        mgr.add_key(ManagedKey::new("k1", KeyType::Signing, "pub1"))
            .unwrap();
        let k2 = mgr
            .rotate_key("k1", "pub2", RotationReason::Manual)
            .unwrap();
        let k3 = mgr
            .rotate_key(&k2, "pub3", RotationReason::Manual)
            .unwrap();

        let chain = mgr.key_chain(&k3);
        assert_eq!(chain.len(), 3);
        assert_eq!(chain[0].id, "k1");
        assert_eq!(chain[1].id, k2);
        assert_eq!(chain[2].id, k3);
    }

    #[test]
    fn test_recent_rotations() {
        let mut mgr = make_manager();
        mgr.add_key(ManagedKey::new("k1", KeyType::Signing, "pub1"))
            .unwrap();
        let k2 = mgr
            .rotate_key("k1", "pub2", RotationReason::Manual)
            .unwrap();
        mgr.rotate_key(&k2, "pub3", RotationReason::Scheduled)
            .unwrap();

        let recent = mgr.recent_rotations(1);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].reason, RotationReason::Scheduled);

        let all = mgr.recent_rotations(10);
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_stats() {
        let mut mgr = make_manager();
        mgr.add_key(ManagedKey::new("k1", KeyType::Signing, "pub1"))
            .unwrap();
        mgr.add_key(ManagedKey::new("k2", KeyType::Encryption, "pub2"))
            .unwrap();
        mgr.get_key_mut("k2").unwrap().mark_compromised();
        mgr.add_policy(RotationPolicy::new(KeyType::Signing, 90));

        let k3 = mgr
            .rotate_key("k1", "pub3", RotationReason::Manual)
            .unwrap();

        let s = mgr.stats();
        assert_eq!(s.total_keys, 3); // k1, k2, and the new rotated key
        assert_eq!(s.active, 1); // only k3
        assert_eq!(s.rotated, 1); // k1
        assert_eq!(s.compromised, 1); // k2
        assert_eq!(s.total_rotations, 1);
        assert_eq!(s.policies, 1);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path("roundtrip.json");
        let mut mgr = make_manager();
        mgr.add_key(ManagedKey::new("k1", KeyType::Signing, "pub1"))
            .unwrap();
        mgr.add_key(ManagedKey::new("k2", KeyType::Encryption, "pub2"))
            .unwrap();
        let _k3 = mgr
            .rotate_key("k1", "pub3", RotationReason::Manual)
            .unwrap();
        mgr.add_policy(RotationPolicy::new(KeyType::Signing, 90));

        mgr.save(&path).unwrap();

        let loaded = KeyRotationManager::load(&path).unwrap();
        assert_eq!(loaded.keys.len(), 3);
        assert_eq!(loaded.rotation_history.len(), 1);
        assert_eq!(loaded.policies.len(), 1);

        // Clean up
        let _ = std::fs::remove_file(&path);
    }
}
