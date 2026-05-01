//! Backup and recovery — encrypted exports, key rotation, and verification.
//!
//! Exports the entire keystore as an AES-256-GCM encrypted blob with a
//! BLAKE3 checksum for integrity verification. Uses Argon2id KDF. Supports password rotation
//! for individual or all keys.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

use evaporchain_crypto::signatures::Signer;

use crate::keystore::{KeyStore, KeyStoreError};

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum BackupError {
    #[error("keystore error: {0}")]
    KeyStore(#[from] KeyStoreError),
    #[error("encryption error: {0}")]
    Encryption(String),
    #[error("decryption error: wrong password or corrupted backup")]
    Decryption,
    #[error("checksum mismatch: backup is corrupted")]
    ChecksumMismatch,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ──────────────────────────── Encrypted Backup ─────────────────────────

/// An encrypted backup of the entire keystore.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedBackup {
    /// Backup format version.
    pub version: u32,
    /// ISO 8601 timestamp.
    pub created_at: String,
    /// AES-256-GCM ciphertext (hex-encoded).
    pub encrypted_data: String,
    /// AES-GCM nonce (hex-encoded, 12 bytes).
    pub nonce: String,
    /// Password salt (hex-encoded, 32 bytes).
    pub salt: String,
    /// BLAKE3 checksum of the plaintext JSON (hex-encoded).
    pub checksum: String,
    /// Number of keys in the backup.
    pub key_count: usize,
}

// ──────────────────────────── Backup Manager ───────────────────────────

/// Manages keystore backup, recovery, and key rotation.
pub struct BackupManager;

impl BackupManager {
    /// Export the keystore as an encrypted backup.
    pub fn export_encrypted(
        keystore: &KeyStore,
        password: &str,
    ) -> Result<EncryptedBackup, BackupError> {
        let plaintext = serde_json::to_string_pretty(keystore)?;
        let checksum = hex::encode(blake3::hash(plaintext.as_bytes()).as_bytes());

        let mut salt = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut salt);
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);

        let key = derive_key(password, &salt);
        let cipher =
            Aes256Gcm::new_from_slice(&key).map_err(|e| BackupError::Encryption(e.to_string()))?;

        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_bytes())
            .map_err(|e| BackupError::Encryption(e.to_string()))?;

        Ok(EncryptedBackup {
            version: 1,
            created_at: chrono::Utc::now().to_rfc3339(),
            encrypted_data: hex::encode(&ciphertext),
            nonce: hex::encode(nonce_bytes),
            salt: hex::encode(salt),
            checksum,
            key_count: keystore.len(),
        })
    }

    /// Import a keystore from an encrypted backup.
    pub fn import_encrypted(
        backup: &EncryptedBackup,
        password: &str,
    ) -> Result<KeyStore, BackupError> {
        let salt = hex::decode(&backup.salt).map_err(|e| BackupError::Encryption(e.to_string()))?;
        let nonce_bytes =
            hex::decode(&backup.nonce).map_err(|e| BackupError::Encryption(e.to_string()))?;
        let ciphertext = hex::decode(&backup.encrypted_data)
            .map_err(|e| BackupError::Encryption(e.to_string()))?;

        let key = derive_key(password, &salt);
        let cipher =
            Aes256Gcm::new_from_slice(&key).map_err(|e| BackupError::Encryption(e.to_string()))?;

        let plaintext = cipher
            .decrypt(Nonce::from_slice(&nonce_bytes), ciphertext.as_ref())
            .map_err(|_| BackupError::Decryption)?;

        // Verify checksum
        let actual_checksum = hex::encode(blake3::hash(&plaintext).as_bytes());
        if actual_checksum != backup.checksum {
            return Err(BackupError::ChecksumMismatch);
        }

        let keystore: KeyStore = serde_json::from_slice(&plaintext)?;
        Ok(keystore)
    }

    /// Export backup to a JSON file.
    pub fn export_to_file(
        keystore: &KeyStore,
        path: &Path,
        password: &str,
    ) -> Result<(), BackupError> {
        let backup = Self::export_encrypted(keystore, password)?;
        let json = serde_json::to_string_pretty(&backup)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Import backup from a JSON file.
    pub fn import_from_file(path: &Path, password: &str) -> Result<KeyStore, BackupError> {
        let json = std::fs::read_to_string(path)?;
        let backup: EncryptedBackup = serde_json::from_str(&json)?;
        Self::import_encrypted(&backup, password)
    }

    /// Verify a backup without importing it.
    /// Returns `true` if the password is correct and checksum matches.
    pub fn verify_backup(backup: &EncryptedBackup, password: &str) -> Result<bool, BackupError> {
        match Self::import_encrypted(backup, password) {
            Ok(_) => Ok(true),
            Err(BackupError::Decryption) => Ok(false),
            Err(BackupError::ChecksumMismatch) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Rotate the password for a single key in the keystore.
    /// Decrypts with old password, re-encrypts with new password.
    pub fn rotate_key(
        keystore: &mut KeyStore,
        name: &str,
        old_password: &str,
        new_password: &str,
    ) -> Result<(), BackupError> {
        // Unlock the key to get the raw keypair
        let keypair = keystore.unlock_key(name, old_password)?;
        let pk = keypair.public_key_bytes();
        let sk_bytes = keypair.secret_key().to_vec();

        // Remove the old entry and re-import with new password
        keystore.remove(name);
        keystore.import_key(name, new_password, &pk, &sk_bytes)?;

        Ok(())
    }

    /// Rotate passwords for all keys in the keystore.
    pub fn rotate_all(
        keystore: &mut KeyStore,
        old_password: &str,
        new_password: &str,
    ) -> Result<usize, BackupError> {
        let names: Vec<String> = keystore.list().iter().map(|(n, _)| n.to_string()).collect();
        let mut rotated = 0;

        for name in &names {
            Self::rotate_key(keystore, name, old_password, new_password)?;
            rotated += 1;
        }

        Ok(rotated)
    }

    /// Export all public keys (for view-only sharing or auditing).
    pub fn export_public_keys(keystore: &KeyStore) -> Vec<(String, String)> {
        keystore
            .list()
            .iter()
            .filter_map(|(name, _)| {
                keystore
                    .get_public_key(name)
                    .map(|pk| (name.to_string(), hex::encode(pk)))
            })
            .collect()
    }
}

// ──────────────────────────── Internal ─────────────────────────────────

fn derive_key(password: &str, salt: &[u8]) -> [u8; 32] {
    use argon2::{Algorithm, Argon2, Params, Version};

    let params = Params::new(64 * 1024, 3, 1, Some(32)).expect("valid argon2 params");
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .expect("argon2 hash should not fail");
    key
}

// ──────────────────────────── Tests ────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::derive_address;
    use evaporchain_crypto::signatures::Signer;

    fn make_keystore() -> KeyStore {
        let mut ks = KeyStore::new();
        ks.generate_key("alice", "pass123").unwrap();
        ks.generate_key("bob", "pass456").unwrap();
        ks
    }

    #[test]
    fn test_export_import_roundtrip() {
        let ks = make_keystore();
        let backup = BackupManager::export_encrypted(&ks, "backup_pass").unwrap();

        assert_eq!(backup.version, 1);
        assert_eq!(backup.key_count, 2);
        assert!(!backup.encrypted_data.is_empty());

        let restored = BackupManager::import_encrypted(&backup, "backup_pass").unwrap();
        assert_eq!(restored.len(), 2);

        // Verify keys still work
        let kp = restored.unlock_key("alice", "pass123").unwrap();
        assert!(!kp.public_key_bytes().is_empty());
    }

    #[test]
    fn test_wrong_backup_password() {
        let ks = make_keystore();
        let backup = BackupManager::export_encrypted(&ks, "correct").unwrap();
        let result = BackupManager::import_encrypted(&backup, "wrong");
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_backup_correct() {
        let ks = make_keystore();
        let backup = BackupManager::export_encrypted(&ks, "pass").unwrap();
        assert!(BackupManager::verify_backup(&backup, "pass").unwrap());
    }

    #[test]
    fn test_verify_backup_wrong_password() {
        let ks = make_keystore();
        let backup = BackupManager::export_encrypted(&ks, "pass").unwrap();
        assert!(!BackupManager::verify_backup(&backup, "wrong").unwrap());
    }

    #[test]
    fn test_checksum_detects_corruption() {
        let ks = make_keystore();
        let mut backup = BackupManager::export_encrypted(&ks, "pass").unwrap();
        // Corrupt the checksum
        backup.checksum =
            "0000000000000000000000000000000000000000000000000000000000000000".to_string();
        let result = BackupManager::import_encrypted(&backup, "pass");
        assert!(matches!(result, Err(BackupError::ChecksumMismatch)));
    }

    #[test]
    fn test_file_export_import() {
        let ks = make_keystore();
        let dir = std::env::temp_dir().join("evaporchain_backup_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test_backup.json");

        BackupManager::export_to_file(&ks, &path, "filepass").unwrap();
        let restored = BackupManager::import_from_file(&path, "filepass").unwrap();
        assert_eq!(restored.len(), 2);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_rotate_single_key() {
        let mut ks = make_keystore();
        let addr_before = ks.get_address("alice").unwrap();

        BackupManager::rotate_key(&mut ks, "alice", "pass123", "newpass").unwrap();

        // Old password should fail
        assert!(ks.unlock_key("alice", "pass123").is_err());
        // New password should work
        let kp = ks.unlock_key("alice", "newpass").unwrap();
        assert_eq!(derive_address(&kp.public_key_bytes()), addr_before);
    }

    #[test]
    fn test_rotate_all() {
        let mut ks = KeyStore::new();
        ks.generate_key("key1", "same_pass").unwrap();
        ks.generate_key("key2", "same_pass").unwrap();

        let count = BackupManager::rotate_all(&mut ks, "same_pass", "new_pass").unwrap();
        assert_eq!(count, 2);

        // Both should work with new password
        assert!(ks.unlock_key("key1", "new_pass").is_ok());
        assert!(ks.unlock_key("key2", "new_pass").is_ok());
    }

    #[test]
    fn test_export_public_keys() {
        let ks = make_keystore();
        let pks = BackupManager::export_public_keys(&ks);
        assert_eq!(pks.len(), 2);
        assert_eq!(pks[0].0, "alice");
        assert!(!pks[0].1.is_empty());
        // ML-DSA public key is 1952 bytes = 3904 hex chars
        assert_eq!(pks[0].1.len(), 3904);
    }
}
