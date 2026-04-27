use std::collections::HashMap;
use std::path::Path;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum SecureEnclaveError {
    #[error("key not found: {0}")]
    KeyNotFound(String),
    #[error("duplicate key: {0}")]
    DuplicateKey(String),
    #[error("enclave is sealed: {0}")]
    EnclaveSealed(String),
    #[error("tamper detected: {0}")]
    TamperDetected(String),
    #[error("key expired: {0}")]
    KeyExpired(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum KeyPurpose {
    Signing,
    Encryption,
    Authentication,
    Derivation,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum KeyStatus2 {
    Active,
    Locked,
    Expired,
    Wiped,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum EnclaveStatus {
    #[default]
    Open,
    Sealed,
    Compromised,
}

// ---------------------------------------------------------------------------
// Supporting structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnclaveKey {
    pub id: String,
    pub purpose: KeyPurpose,
    pub status: KeyStatus2,
    pub key_hash: String,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub last_accessed: Option<String>,
    pub access_count: u64,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessLog2 {
    pub key_id: String,
    pub action: String,
    pub timestamp: String,
    pub success: bool,
    pub ip_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TamperEvent {
    pub id: String,
    pub description: String,
    pub detected_at: String,
    pub key_id: Option<String>,
    pub auto_wiped: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnclaveStats2 {
    pub total_keys: usize,
    pub active_keys: usize,
    pub locked_keys: usize,
    pub expired_keys: usize,
    pub wiped_keys: usize,
    pub total_accesses: u64,
    pub tamper_events: usize,
    pub enclave_status: EnclaveStatus,
}

// ---------------------------------------------------------------------------
// SecureEnclave
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SecureEnclave {
    pub keys: HashMap<String, EnclaveKey>,
    pub access_logs: Vec<AccessLog2>,
    pub tamper_events: Vec<TamperEvent>,
    pub status: EnclaveStatus,
}

impl SecureEnclave {
    pub fn new() -> Self {
        Self::default()
    }

    // -- key storage --------------------------------------------------------

    pub fn store_key(
        &mut self,
        id: &str,
        key_material: &str,
        purpose: KeyPurpose,
        expires_at: Option<String>,
    ) -> Result<(), SecureEnclaveError> {
        if self.status != EnclaveStatus::Open {
            return Err(SecureEnclaveError::EnclaveSealed(
                "cannot store key while enclave is not open".into(),
            ));
        }
        if self.keys.contains_key(id) {
            return Err(SecureEnclaveError::DuplicateKey(id.to_string()));
        }

        let key_hash = blake3::hash(key_material.as_bytes()).to_hex().to_string();
        let now = Utc::now().to_rfc3339();

        let key = EnclaveKey {
            id: id.to_string(),
            purpose,
            status: KeyStatus2::Active,
            key_hash,
            created_at: now,
            expires_at,
            last_accessed: None,
            access_count: 0,
            metadata: HashMap::new(),
        };
        self.keys.insert(id.to_string(), key);
        Ok(())
    }

    pub fn remove_key(&mut self, id: &str) -> Result<EnclaveKey, SecureEnclaveError> {
        self.keys
            .remove(id)
            .ok_or_else(|| SecureEnclaveError::KeyNotFound(id.to_string()))
    }

    // -- access -------------------------------------------------------------

    pub fn access_key(&mut self, id: &str) -> Result<&EnclaveKey, SecureEnclaveError> {
        if self.status != EnclaveStatus::Open {
            return Err(SecureEnclaveError::EnclaveSealed(
                "enclave is not open".into(),
            ));
        }

        // Check existence first.
        if !self.keys.contains_key(id) {
            return Err(SecureEnclaveError::KeyNotFound(id.to_string()));
        }

        // Check status (mutable borrow).
        {
            let key = self.keys.get_mut(id).unwrap();
            match key.status {
                KeyStatus2::Active => {}
                KeyStatus2::Expired => {
                    return Err(SecureEnclaveError::KeyExpired(id.to_string()));
                }
                KeyStatus2::Wiped | KeyStatus2::Locked => {
                    return Err(SecureEnclaveError::KeyNotFound(id.to_string()));
                }
            }
            key.access_count += 1;
            key.last_accessed = Some(Utc::now().to_rfc3339());
        }

        // Log the access.
        self.access_logs.push(AccessLog2 {
            key_id: id.to_string(),
            action: "access".to_string(),
            timestamp: Utc::now().to_rfc3339(),
            success: true,
            ip_hint: None,
        });

        Ok(self.keys.get(id).unwrap())
    }

    // -- status mutations ---------------------------------------------------

    pub fn lock_key(&mut self, id: &str) -> Result<(), SecureEnclaveError> {
        let key = self
            .keys
            .get_mut(id)
            .ok_or_else(|| SecureEnclaveError::KeyNotFound(id.to_string()))?;
        key.status = KeyStatus2::Locked;
        Ok(())
    }

    pub fn unlock_key(&mut self, id: &str) -> Result<(), SecureEnclaveError> {
        let key = self
            .keys
            .get_mut(id)
            .ok_or_else(|| SecureEnclaveError::KeyNotFound(id.to_string()))?;
        key.status = KeyStatus2::Active;
        Ok(())
    }

    pub fn expire_key(&mut self, id: &str) -> Result<(), SecureEnclaveError> {
        let key = self
            .keys
            .get_mut(id)
            .ok_or_else(|| SecureEnclaveError::KeyNotFound(id.to_string()))?;
        key.status = KeyStatus2::Expired;
        Ok(())
    }

    pub fn wipe_key(&mut self, id: &str) -> Result<(), SecureEnclaveError> {
        let key = self
            .keys
            .get_mut(id)
            .ok_or_else(|| SecureEnclaveError::KeyNotFound(id.to_string()))?;
        key.status = KeyStatus2::Wiped;
        key.key_hash = String::new();
        Ok(())
    }

    // -- enclave seal / unseal ----------------------------------------------

    pub fn seal_enclave(&mut self) {
        self.status = EnclaveStatus::Sealed;
    }

    pub fn unseal_enclave(&mut self) {
        self.status = EnclaveStatus::Open;
    }

    // -- tamper detection ---------------------------------------------------

    pub fn report_tamper(
        &mut self,
        description: &str,
        key_id: Option<&str>,
    ) -> TamperEvent {
        let mut auto_wiped = false;

        if let Some(kid) = key_id {
            if let Some(key) = self.keys.get_mut(kid) {
                key.status = KeyStatus2::Wiped;
                key.key_hash = String::new();
                auto_wiped = true;
            }
        }

        self.status = EnclaveStatus::Compromised;

        let event = TamperEvent {
            id: format!("tamper-{}", self.tamper_events.len() + 1),
            description: description.to_string(),
            detected_at: Utc::now().to_rfc3339(),
            key_id: key_id.map(|s| s.to_string()),
            auto_wiped,
        };
        self.tamper_events.push(event.clone());
        event
    }

    // -- verification -------------------------------------------------------

    pub fn verify_key_integrity(
        &self,
        id: &str,
        original_material: &str,
    ) -> Result<bool, SecureEnclaveError> {
        let key = self
            .keys
            .get(id)
            .ok_or_else(|| SecureEnclaveError::KeyNotFound(id.to_string()))?;
        let computed = blake3::hash(original_material.as_bytes())
            .to_hex()
            .to_string();
        Ok(computed == key.key_hash)
    }

    // -- queries ------------------------------------------------------------

    pub fn keys_by_purpose(&self, purpose: &KeyPurpose) -> Vec<&EnclaveKey> {
        self.keys
            .values()
            .filter(|k| &k.purpose == purpose)
            .collect()
    }

    pub fn active_keys(&self) -> Vec<&EnclaveKey> {
        self.keys
            .values()
            .filter(|k| k.status == KeyStatus2::Active)
            .collect()
    }

    pub fn expired_keys(&self) -> Vec<&EnclaveKey> {
        self.keys
            .values()
            .filter(|k| k.status == KeyStatus2::Expired)
            .collect()
    }

    pub fn recent_access_logs(&self, n: usize) -> Vec<&AccessLog2> {
        let len = self.access_logs.len();
        let start = len.saturating_sub(n);
        self.access_logs[start..].iter().collect()
    }

    pub fn stats(&self) -> EnclaveStats2 {
        let total_accesses = self.keys.values().map(|k| k.access_count).sum();
        EnclaveStats2 {
            total_keys: self.keys.len(),
            active_keys: self
                .keys
                .values()
                .filter(|k| k.status == KeyStatus2::Active)
                .count(),
            locked_keys: self
                .keys
                .values()
                .filter(|k| k.status == KeyStatus2::Locked)
                .count(),
            expired_keys: self
                .keys
                .values()
                .filter(|k| k.status == KeyStatus2::Expired)
                .count(),
            wiped_keys: self
                .keys
                .values()
                .filter(|k| k.status == KeyStatus2::Wiped)
                .count(),
            total_accesses,
            tamper_events: self.tamper_events.len(),
            enclave_status: self.status.clone(),
        }
    }

    // -- persistence --------------------------------------------------------

    pub fn save(&self, path: &Path) -> Result<(), SecureEnclaveError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, SecureEnclaveError> {
        let data = std::fs::read_to_string(path)?;
        let enclave: Self = serde_json::from_str(&data)?;
        Ok(enclave)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;
    use std::process;

    fn test_path(name: &str) -> std::path::PathBuf {
        temp_dir().join(format!("secure_enclave_test_{}_{}.json", process::id(), name))
    }

    #[test]
    fn test_store_key() {
        let mut enc = SecureEnclave::new();
        enc.store_key("k1", "secret", KeyPurpose::Signing, None)
            .unwrap();
        assert!(enc.keys.contains_key("k1"));
        assert_eq!(enc.keys["k1"].status, KeyStatus2::Active);
        assert!(!enc.keys["k1"].key_hash.is_empty());
    }

    #[test]
    fn test_duplicate_key() {
        let mut enc = SecureEnclave::new();
        enc.store_key("k1", "secret", KeyPurpose::Signing, None)
            .unwrap();
        let err = enc
            .store_key("k1", "other", KeyPurpose::Encryption, None)
            .unwrap_err();
        assert!(matches!(err, SecureEnclaveError::DuplicateKey(_)));
    }

    #[test]
    fn test_remove_key() {
        let mut enc = SecureEnclave::new();
        enc.store_key("k1", "secret", KeyPurpose::Signing, None)
            .unwrap();
        let removed = enc.remove_key("k1").unwrap();
        assert_eq!(removed.id, "k1");
        assert!(!enc.keys.contains_key("k1"));
    }

    #[test]
    fn test_remove_key_not_found() {
        let mut enc = SecureEnclave::new();
        let err = enc.remove_key("nope").unwrap_err();
        assert!(matches!(err, SecureEnclaveError::KeyNotFound(_)));
    }

    #[test]
    fn test_access_key_success() {
        let mut enc = SecureEnclave::new();
        enc.store_key("k1", "secret", KeyPurpose::Signing, None)
            .unwrap();
        let key = enc.access_key("k1").unwrap();
        assert_eq!(key.access_count, 1);
        assert!(key.last_accessed.is_some());
    }

    #[test]
    fn test_access_key_increments_count() {
        let mut enc = SecureEnclave::new();
        enc.store_key("k1", "secret", KeyPurpose::Signing, None)
            .unwrap();
        enc.access_key("k1").unwrap();
        enc.access_key("k1").unwrap();
        enc.access_key("k1").unwrap();
        assert_eq!(enc.keys["k1"].access_count, 3);
    }

    #[test]
    fn test_lock_key() {
        let mut enc = SecureEnclave::new();
        enc.store_key("k1", "secret", KeyPurpose::Signing, None)
            .unwrap();
        enc.lock_key("k1").unwrap();
        assert_eq!(enc.keys["k1"].status, KeyStatus2::Locked);
    }

    #[test]
    fn test_unlock_key() {
        let mut enc = SecureEnclave::new();
        enc.store_key("k1", "secret", KeyPurpose::Signing, None)
            .unwrap();
        enc.lock_key("k1").unwrap();
        enc.unlock_key("k1").unwrap();
        assert_eq!(enc.keys["k1"].status, KeyStatus2::Active);
    }

    #[test]
    fn test_expire_key() {
        let mut enc = SecureEnclave::new();
        enc.store_key("k1", "secret", KeyPurpose::Signing, None)
            .unwrap();
        enc.expire_key("k1").unwrap();
        assert_eq!(enc.keys["k1"].status, KeyStatus2::Expired);
    }

    #[test]
    fn test_wipe_key_clears_hash() {
        let mut enc = SecureEnclave::new();
        enc.store_key("k1", "secret", KeyPurpose::Signing, None)
            .unwrap();
        enc.wipe_key("k1").unwrap();
        assert_eq!(enc.keys["k1"].status, KeyStatus2::Wiped);
        assert!(enc.keys["k1"].key_hash.is_empty());
    }

    #[test]
    fn test_seal_and_unseal() {
        let mut enc = SecureEnclave::new();
        enc.seal_enclave();
        assert_eq!(enc.status, EnclaveStatus::Sealed);
        enc.unseal_enclave();
        assert_eq!(enc.status, EnclaveStatus::Open);
    }

    #[test]
    fn test_access_while_sealed_fails() {
        let mut enc = SecureEnclave::new();
        enc.store_key("k1", "secret", KeyPurpose::Signing, None)
            .unwrap();
        enc.seal_enclave();
        let err = enc.access_key("k1").unwrap_err();
        assert!(matches!(err, SecureEnclaveError::EnclaveSealed(_)));
    }

    #[test]
    fn test_store_while_sealed_fails() {
        let mut enc = SecureEnclave::new();
        enc.seal_enclave();
        let err = enc
            .store_key("k1", "secret", KeyPurpose::Signing, None)
            .unwrap_err();
        assert!(matches!(err, SecureEnclaveError::EnclaveSealed(_)));
    }

    #[test]
    fn test_tamper_detection_auto_wipe() {
        let mut enc = SecureEnclave::new();
        enc.store_key("k1", "secret", KeyPurpose::Signing, None)
            .unwrap();
        let event = enc.report_tamper("voltage glitch detected", Some("k1"));
        assert!(event.auto_wiped);
        assert_eq!(enc.keys["k1"].status, KeyStatus2::Wiped);
        assert!(enc.keys["k1"].key_hash.is_empty());
        assert_eq!(enc.status, EnclaveStatus::Compromised);
    }

    #[test]
    fn test_tamper_without_key() {
        let mut enc = SecureEnclave::new();
        let event = enc.report_tamper("physical probe", None);
        assert!(!event.auto_wiped);
        assert_eq!(enc.status, EnclaveStatus::Compromised);
        assert_eq!(enc.tamper_events.len(), 1);
    }

    #[test]
    fn test_verify_integrity_match() {
        let mut enc = SecureEnclave::new();
        enc.store_key("k1", "secret", KeyPurpose::Signing, None)
            .unwrap();
        assert!(enc.verify_key_integrity("k1", "secret").unwrap());
    }

    #[test]
    fn test_verify_integrity_mismatch() {
        let mut enc = SecureEnclave::new();
        enc.store_key("k1", "secret", KeyPurpose::Signing, None)
            .unwrap();
        assert!(!enc.verify_key_integrity("k1", "wrong").unwrap());
    }

    #[test]
    fn test_keys_by_purpose() {
        let mut enc = SecureEnclave::new();
        enc.store_key("k1", "a", KeyPurpose::Signing, None).unwrap();
        enc.store_key("k2", "b", KeyPurpose::Encryption, None).unwrap();
        enc.store_key("k3", "c", KeyPurpose::Signing, None).unwrap();
        let signing = enc.keys_by_purpose(&KeyPurpose::Signing);
        assert_eq!(signing.len(), 2);
    }

    #[test]
    fn test_active_keys() {
        let mut enc = SecureEnclave::new();
        enc.store_key("k1", "a", KeyPurpose::Signing, None).unwrap();
        enc.store_key("k2", "b", KeyPurpose::Encryption, None).unwrap();
        enc.lock_key("k2").unwrap();
        assert_eq!(enc.active_keys().len(), 1);
    }

    #[test]
    fn test_expired_keys_query() {
        let mut enc = SecureEnclave::new();
        enc.store_key("k1", "a", KeyPurpose::Signing, None).unwrap();
        enc.store_key("k2", "b", KeyPurpose::Encryption, None).unwrap();
        enc.expire_key("k1").unwrap();
        let expired = enc.expired_keys();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].id, "k1");
    }

    #[test]
    fn test_stats() {
        let mut enc = SecureEnclave::new();
        enc.store_key("k1", "a", KeyPurpose::Signing, None).unwrap();
        enc.store_key("k2", "b", KeyPurpose::Encryption, None).unwrap();
        enc.store_key("k3", "c", KeyPurpose::Authentication, None).unwrap();
        enc.lock_key("k2").unwrap();
        enc.expire_key("k3").unwrap();
        enc.access_key("k1").unwrap();
        enc.access_key("k1").unwrap();

        let s = enc.stats();
        assert_eq!(s.total_keys, 3);
        assert_eq!(s.active_keys, 1);
        assert_eq!(s.locked_keys, 1);
        assert_eq!(s.expired_keys, 1);
        assert_eq!(s.total_accesses, 2);
        assert_eq!(s.enclave_status, EnclaveStatus::Open);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path("roundtrip");
        let mut enc = SecureEnclave::new();
        enc.store_key("k1", "secret", KeyPurpose::Signing, None)
            .unwrap();
        enc.access_key("k1").unwrap();
        enc.save(&path).unwrap();

        let loaded = SecureEnclave::load(&path).unwrap();
        assert!(loaded.keys.contains_key("k1"));
        assert_eq!(loaded.keys["k1"].access_count, 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = test_path("nonexistent");
        let _ = std::fs::remove_file(&path);
        let enc = SecureEnclave::load_or_default(&path);
        assert_eq!(enc.status, EnclaveStatus::Open);
        assert!(enc.keys.is_empty());
    }

    #[test]
    fn test_access_log_recording() {
        let mut enc = SecureEnclave::new();
        enc.store_key("k1", "a", KeyPurpose::Signing, None).unwrap();
        enc.access_key("k1").unwrap();
        enc.access_key("k1").unwrap();
        let logs = enc.recent_access_logs(10);
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].key_id, "k1");
        assert!(logs[0].success);
    }
}
