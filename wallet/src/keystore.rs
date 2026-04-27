//! Encrypted keystore for ML-DSA key pairs.
//!
//! Keys are encrypted at rest using AES-256-GCM with a key derived from a
//! user password via Argon2id (memory-hard KDF). The keystore is serialized
//! to JSON for portability.
//!
//! # Format
//!
//! ```json
//! {
//!   "version": 1,
//!   "entries": [
//!     {
//!       "name": "default",
//!       "address": "0xabc...",
//!       "public_key": "hex...",
//!       "encrypted_secret_key": "hex...",
//!       "nonce": "hex...",
//!       "salt": "hex...",
//!       "created_at": "2026-04-06T..."
//!     }
//!   ]
//! }
//! ```

use crate::address::derive_address;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use evaporchain_crypto::signatures::{EcdsaKeypair, HybridKeypair, MlDsaKeypair, Signer};
use evaporchain_types::AccountAddress;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

/// Current keystore format version.
const KEYSTORE_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum KeyStoreError {
    #[error("entry not found: {0}")]
    NotFound(String),
    #[error("duplicate entry name: {0}")]
    DuplicateName(String),
    #[error("duplicate address: {0}")]
    DuplicateAddress(String),
    #[error("encryption error: {0}")]
    Encryption(String),
    #[error("decryption error: wrong password or corrupted data")]
    Decryption,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("key reconstruction failed: {0}")]
    KeyReconstruction(String),
}

/// A single encrypted key pair entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyEntry {
    /// Human-readable name for this key.
    pub name: String,
    /// Derived address (hex with 0x prefix).
    pub address: String,
    /// Public key (hex-encoded).
    pub public_key: String,
    /// AES-256-GCM encrypted secret key (hex-encoded ciphertext).
    encrypted_secret_key: String,
    /// AES-256-GCM nonce (hex-encoded, 12 bytes).
    nonce: String,
    /// Password salt (hex-encoded, 32 bytes).
    salt: String,
    /// Creation timestamp (ISO 8601).
    pub created_at: String,
}

/// The full keystore containing multiple key entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyStore {
    /// Format version.
    pub version: u32,
    /// Key entries.
    pub entries: Vec<KeyEntry>,
}

impl KeyStore {
    /// Create a new empty keystore.
    pub fn new() -> Self {
        Self {
            version: KEYSTORE_VERSION,
            entries: Vec::new(),
        }
    }

    /// Load a keystore from a JSON file.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, KeyStoreError> {
        let data = std::fs::read_to_string(path)?;
        let store: KeyStore = serde_json::from_str(&data)?;
        Ok(store)
    }

    /// Save the keystore to a JSON file.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), KeyStoreError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Generate a new ML-DSA key pair, encrypt it, and add to the keystore.
    /// Returns the derived address.
    pub fn generate_key(
        &mut self,
        name: &str,
        password: &str,
    ) -> Result<AccountAddress, KeyStoreError> {
        // Check for duplicate names
        if self.entries.iter().any(|e| e.name == name) {
            return Err(KeyStoreError::DuplicateName(name.to_string()));
        }

        // Generate ML-DSA key pair
        let keypair = MlDsaKeypair::generate();
        let pk_bytes = keypair.public_key_bytes();
        let sk_bytes = keypair.secret_key();

        // Derive address
        let address = derive_address(&pk_bytes);
        let address_hex = crate::address::format_address(&address);

        // Check for duplicate address
        if self.entries.iter().any(|e| e.address == address_hex) {
            return Err(KeyStoreError::DuplicateAddress(address_hex));
        }

        // Encrypt secret key
        let mut salt = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut salt);

        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);

        let encryption_key = derive_encryption_key(password, &salt);
        let cipher = Aes256Gcm::new_from_slice(&encryption_key)
            .map_err(|e| KeyStoreError::Encryption(e.to_string()))?;

        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), sk_bytes.as_ref())
            .map_err(|e| KeyStoreError::Encryption(e.to_string()))?;

        let entry = KeyEntry {
            name: name.to_string(),
            address: address_hex,
            public_key: hex::encode(&pk_bytes),
            encrypted_secret_key: hex::encode(&ciphertext),
            nonce: hex::encode(nonce_bytes),
            salt: hex::encode(salt),
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        self.entries.push(entry);
        Ok(address)
    }

    /// Generate a new Hybrid (ECDSA + ML-DSA) key pair, encrypt it, and add to the keystore.
    /// The encrypted blob contains both secret keys concatenated: ECDSA SK (32 bytes) || ML-DSA SK (4000 bytes).
    pub fn generate_hybrid_key(
        &mut self,
        name: &str,
        password: &str,
    ) -> Result<AccountAddress, KeyStoreError> {
        if self.entries.iter().any(|e| e.name == name) {
            return Err(KeyStoreError::DuplicateName(name.to_string()));
        }

        let keypair = HybridKeypair::generate();
        let pk_bytes = keypair.public_key_bytes();
        let ecdsa_sk = keypair.ecdsa.secret_key_bytes();
        let mldsa_sk = keypair.mldsa.secret_key();

        let mut combined_sk = Vec::with_capacity(ecdsa_sk.len() + mldsa_sk.len());
        combined_sk.extend_from_slice(&ecdsa_sk);
        combined_sk.extend_from_slice(mldsa_sk);

        let address = derive_address(&pk_bytes);
        let address_hex = crate::address::format_address(&address);

        if self.entries.iter().any(|e| e.address == address_hex) {
            return Err(KeyStoreError::DuplicateAddress(address_hex));
        }

        let mut salt = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut salt);
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);

        let encryption_key = derive_encryption_key(password, &salt);
        let cipher = Aes256Gcm::new_from_slice(&encryption_key)
            .map_err(|e| KeyStoreError::Encryption(e.to_string()))?;

        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), combined_sk.as_ref())
            .map_err(|e| KeyStoreError::Encryption(e.to_string()))?;

        let entry = KeyEntry {
            name: name.to_string(),
            address: address_hex,
            public_key: hex::encode(&pk_bytes),
            encrypted_secret_key: hex::encode(&ciphertext),
            nonce: hex::encode(nonce_bytes),
            salt: hex::encode(salt),
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        self.entries.push(entry);
        Ok(address)
    }

    /// Decrypt and return a Hybrid keypair for a hybrid key entry.
    pub fn unlock_hybrid_key(
        &self,
        name: &str,
        password: &str,
    ) -> Result<HybridKeypair, KeyStoreError> {
        let entry = self
            .entries
            .iter()
            .find(|e| e.name == name)
            .ok_or_else(|| KeyStoreError::NotFound(name.to_string()))?;

        decrypt_hybrid_entry(entry, password)
    }

    /// Import an existing ML-DSA key pair into the keystore.
    pub fn import_key(
        &mut self,
        name: &str,
        password: &str,
        public_key: &[u8],
        secret_key: &[u8],
    ) -> Result<AccountAddress, KeyStoreError> {
        if self.entries.iter().any(|e| e.name == name) {
            return Err(KeyStoreError::DuplicateName(name.to_string()));
        }

        let address = derive_address(public_key);
        let address_hex = crate::address::format_address(&address);

        if self.entries.iter().any(|e| e.address == address_hex) {
            return Err(KeyStoreError::DuplicateAddress(address_hex));
        }

        let mut salt = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut salt);
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);

        let encryption_key = derive_encryption_key(password, &salt);
        let cipher = Aes256Gcm::new_from_slice(&encryption_key)
            .map_err(|e| KeyStoreError::Encryption(e.to_string()))?;

        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), secret_key)
            .map_err(|e| KeyStoreError::Encryption(e.to_string()))?;

        let entry = KeyEntry {
            name: name.to_string(),
            address: address_hex,
            public_key: hex::encode(public_key),
            encrypted_secret_key: hex::encode(&ciphertext),
            nonce: hex::encode(nonce_bytes),
            salt: hex::encode(salt),
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        self.entries.push(entry);
        Ok(address)
    }

    /// Decrypt and return the ML-DSA keypair for the given name.
    pub fn unlock_key(
        &self,
        name: &str,
        password: &str,
    ) -> Result<MlDsaKeypair, KeyStoreError> {
        let entry = self
            .entries
            .iter()
            .find(|e| e.name == name)
            .ok_or_else(|| KeyStoreError::NotFound(name.to_string()))?;

        decrypt_entry(entry, password)
    }

    /// Decrypt and return the ML-DSA keypair for the given address.
    pub fn unlock_by_address(
        &self,
        address: &AccountAddress,
        password: &str,
    ) -> Result<MlDsaKeypair, KeyStoreError> {
        let addr_hex = crate::address::format_address(address);
        let entry = self
            .entries
            .iter()
            .find(|e| e.address == addr_hex)
            .ok_or(KeyStoreError::NotFound(addr_hex))?;

        decrypt_entry(entry, password)
    }

    /// Get the address for a named key.
    pub fn get_address(&self, name: &str) -> Option<AccountAddress> {
        self.entries
            .iter()
            .find(|e| e.name == name)
            .and_then(|e| crate::address::parse_address(&e.address).ok())
    }

    /// Get the public key bytes for a named key.
    pub fn get_public_key(&self, name: &str) -> Option<Vec<u8>> {
        self.entries
            .iter()
            .find(|e| e.name == name)
            .and_then(|e| hex::decode(&e.public_key).ok())
    }

    /// List all entry names and addresses.
    pub fn list(&self) -> Vec<(&str, &str)> {
        self.entries
            .iter()
            .map(|e| (e.name.as_str(), e.address.as_str()))
            .collect()
    }

    /// Remove an entry by name.
    pub fn remove(&mut self, name: &str) -> bool {
        let len_before = self.entries.len();
        self.entries.retain(|e| e.name != name);
        self.entries.len() < len_before
    }

    /// Number of stored keys.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the keystore is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for KeyStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Derive a 32-byte AES-256 encryption key from a password and salt.
/// Uses Argon2id (memory-hard KDF) for brute-force resistance.
///
/// Parameters: 64 MiB memory, 3 iterations, 1 thread.
/// These are OWASP-recommended minimums for interactive logins.
fn derive_encryption_key(password: &str, salt: &[u8]) -> [u8; 32] {
    use argon2::{Algorithm, Argon2, Params, Version};

    let params = Params::new(
        64 * 1024, // 64 MiB memory cost (in KiB)
        3,         // 3 iterations
        1,         // 1 lane (parallelism)
        Some(32),  // 32-byte output
    )
    .expect("valid argon2 params");

    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .expect("argon2 hash should not fail with valid params");
    key
}

/// Decrypt a key entry and reconstruct the ML-DSA keypair.
fn decrypt_entry(entry: &KeyEntry, password: &str) -> Result<MlDsaKeypair, KeyStoreError> {
    let salt = hex::decode(&entry.salt).map_err(|e| KeyStoreError::Encryption(e.to_string()))?;
    let nonce_bytes =
        hex::decode(&entry.nonce).map_err(|e| KeyStoreError::Encryption(e.to_string()))?;
    let ciphertext = hex::decode(&entry.encrypted_secret_key)
        .map_err(|e| KeyStoreError::Encryption(e.to_string()))?;
    let pk_bytes =
        hex::decode(&entry.public_key).map_err(|e| KeyStoreError::Encryption(e.to_string()))?;

    let encryption_key = derive_encryption_key(password, &salt);
    let cipher = Aes256Gcm::new_from_slice(&encryption_key)
        .map_err(|e| KeyStoreError::Encryption(e.to_string()))?;

    let sk_bytes = cipher
        .decrypt(Nonce::from_slice(&nonce_bytes), ciphertext.as_ref())
        .map_err(|_| KeyStoreError::Decryption)?;

    MlDsaKeypair::from_bytes(&pk_bytes, &sk_bytes)
        .map_err(|e| KeyStoreError::KeyReconstruction(e.to_string()))
}

const ECDSA_SK_LEN: usize = 32;
const MLDSA_SK_LEN: usize = 4000;

fn decrypt_hybrid_entry(entry: &KeyEntry, password: &str) -> Result<HybridKeypair, KeyStoreError> {
    let salt = hex::decode(&entry.salt).map_err(|e| KeyStoreError::Encryption(e.to_string()))?;
    let nonce_bytes =
        hex::decode(&entry.nonce).map_err(|e| KeyStoreError::Encryption(e.to_string()))?;
    let ciphertext = hex::decode(&entry.encrypted_secret_key)
        .map_err(|e| KeyStoreError::Encryption(e.to_string()))?;
    let pk_bytes =
        hex::decode(&entry.public_key).map_err(|e| KeyStoreError::Encryption(e.to_string()))?;

    let encryption_key = derive_encryption_key(password, &salt);
    let cipher = Aes256Gcm::new_from_slice(&encryption_key)
        .map_err(|e| KeyStoreError::Encryption(e.to_string()))?;

    let combined_sk = cipher
        .decrypt(Nonce::from_slice(&nonce_bytes), ciphertext.as_ref())
        .map_err(|_| KeyStoreError::Decryption)?;

    if combined_sk.len() < ECDSA_SK_LEN + MLDSA_SK_LEN {
        return Err(KeyStoreError::KeyReconstruction(
            "decrypted key too short for hybrid".to_string(),
        ));
    }

    let ecdsa_sk = &combined_sk[..ECDSA_SK_LEN];
    let mldsa_sk = &combined_sk[ECDSA_SK_LEN..];

    let ecdsa_kp = EcdsaKeypair::from_bytes(ecdsa_sk)
        .map_err(|e| KeyStoreError::KeyReconstruction(e.to_string()))?;

    let mldsa_pk = &pk_bytes[1 + 33..]; // skip tag + ECDSA PK
    let mldsa_kp = MlDsaKeypair::from_bytes(mldsa_pk, mldsa_sk)
        .map_err(|e| KeyStoreError::KeyReconstruction(e.to_string()))?;

    Ok(HybridKeypair::from_parts(ecdsa_kp, mldsa_kp))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_and_unlock() {
        let mut store = KeyStore::new();
        let addr = store.generate_key("test", "password123").unwrap();
        assert_ne!(addr, [0u8; 32]);
        assert_eq!(store.len(), 1);

        // Unlock with correct password
        let kp = store.unlock_key("test", "password123").unwrap();
        assert_eq!(derive_address(&kp.public_key_bytes()), addr);
    }

    #[test]
    fn test_wrong_password_fails() {
        let mut store = KeyStore::new();
        store.generate_key("test", "correct").unwrap();
        let result = store.unlock_key("test", "wrong");
        assert!(result.is_err());
    }

    #[test]
    fn test_duplicate_name_rejected() {
        let mut store = KeyStore::new();
        store.generate_key("alice", "pass").unwrap();
        let result = store.generate_key("alice", "pass2");
        assert!(matches!(result, Err(KeyStoreError::DuplicateName(_))));
    }

    #[test]
    fn test_not_found() {
        let store = KeyStore::new();
        let result = store.unlock_key("nonexistent", "pass");
        assert!(matches!(result, Err(KeyStoreError::NotFound(_))));
    }

    #[test]
    fn test_list_keys() {
        let mut store = KeyStore::new();
        store.generate_key("alice", "pass1").unwrap();
        store.generate_key("bob", "pass2").unwrap();
        let list = store.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].0, "alice");
        assert_eq!(list[1].0, "bob");
    }

    #[test]
    fn test_remove_key() {
        let mut store = KeyStore::new();
        store.generate_key("temp", "pass").unwrap();
        assert_eq!(store.len(), 1);
        assert!(store.remove("temp"));
        assert_eq!(store.len(), 0);
        assert!(!store.remove("temp")); // already removed
    }

    #[test]
    fn test_get_address() {
        let mut store = KeyStore::new();
        let addr = store.generate_key("mykey", "pass").unwrap();
        assert_eq!(store.get_address("mykey"), Some(addr));
        assert_eq!(store.get_address("nonexistent"), None);
    }

    #[test]
    fn test_unlock_by_address() {
        let mut store = KeyStore::new();
        let addr = store.generate_key("byaddr", "pass").unwrap();
        let kp = store.unlock_by_address(&addr, "pass").unwrap();
        assert_eq!(derive_address(&kp.public_key_bytes()), addr);
    }

    #[test]
    fn test_json_serialization_roundtrip() {
        let mut store = KeyStore::new();
        store.generate_key("persist", "pass").unwrap();

        let json = serde_json::to_string_pretty(&store).unwrap();
        let loaded: KeyStore = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.entries[0].name, "persist");

        // Can still decrypt after serialization
        let kp = loaded.unlock_key("persist", "pass").unwrap();
        assert!(!kp.public_key_bytes().is_empty());
    }

    #[test]
    fn test_file_save_and_load() {
        let dir = std::env::temp_dir().join("evaporchain_wallet_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test_keystore.json");

        let mut store = KeyStore::new();
        let addr = store.generate_key("file_test", "filepass").unwrap();
        store.save(&path).unwrap();

        let loaded = KeyStore::load(&path).unwrap();
        let kp = loaded.unlock_key("file_test", "filepass").unwrap();
        assert_eq!(derive_address(&kp.public_key_bytes()), addr);

        // Cleanup
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_import_key() {
        let kp = MlDsaKeypair::generate();
        let pk = kp.public_key_bytes();
        let sk = kp.secret_key();
        let expected_addr = derive_address(&pk);

        let mut store = KeyStore::new();
        let addr = store.import_key("imported", "pass", &pk, &sk).unwrap();
        assert_eq!(addr, expected_addr);

        // Unlock and verify round-trip
        let unlocked = store.unlock_key("imported", "pass").unwrap();
        assert_eq!(unlocked.public_key_bytes(), pk);
    }

    // ─── Hybrid Keystore Tests ──────────────────────────────────────

    #[test]
    fn test_generate_hybrid_and_unlock() {
        use evaporchain_crypto::signatures::{HybridVerifier, Verifier};

        let mut store = KeyStore::new();
        let addr = store.generate_hybrid_key("hybrid_test", "pass").unwrap();
        assert_ne!(addr, [0u8; 32]);

        let kp = store.unlock_hybrid_key("hybrid_test", "pass").unwrap();
        let pk = kp.public_key_bytes();
        assert_eq!(derive_address(&pk), addr);
        assert!(HybridVerifier::is_hybrid_pk(&pk));

        let msg = b"hybrid keystore roundtrip";
        let sig = kp.sign(msg);
        assert!(HybridVerifier::verify(msg, &sig, &pk));
    }

    #[test]
    fn test_hybrid_wrong_password_fails() {
        let mut store = KeyStore::new();
        store.generate_hybrid_key("hybrid_wp", "correct").unwrap();
        assert!(store.unlock_hybrid_key("hybrid_wp", "wrong").is_err());
    }

    #[test]
    fn test_hybrid_json_roundtrip() {
        use evaporchain_crypto::signatures::{HybridVerifier, Verifier};

        let mut store = KeyStore::new();
        store.generate_hybrid_key("hybrid_json", "pass").unwrap();

        let json = serde_json::to_string_pretty(&store).unwrap();
        let loaded: KeyStore = serde_json::from_str(&json).unwrap();

        let kp = loaded.unlock_hybrid_key("hybrid_json", "pass").unwrap();
        let msg = b"json roundtrip";
        let sig = kp.sign(msg);
        assert!(HybridVerifier::verify(msg, &sig, &kp.public_key_bytes()));
    }

    #[test]
    fn test_mixed_keys_in_store() {
        let mut store = KeyStore::new();
        store.generate_key("mldsa_key", "pass1").unwrap();
        store.generate_hybrid_key("hybrid_key", "pass2").unwrap();
        assert_eq!(store.len(), 2);

        let mldsa = store.unlock_key("mldsa_key", "pass1").unwrap();
        let hybrid = store.unlock_hybrid_key("hybrid_key", "pass2").unwrap();
        assert_ne!(mldsa.public_key_bytes().len(), hybrid.public_key_bytes().len());
    }
}
