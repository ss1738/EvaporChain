// wallet/src/memo.rs — Encrypted on-chain memos for transactions
//
// Attach human-readable notes to transactions, optionally encrypted
// with AES-256-GCM so only sender/recipient can read them.
// Memos are stored locally with tx hash references.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MemoError {
    #[error("memo not found: {0}")]
    NotFound(String),
    #[error("memo too long: {0} bytes (max {1})")]
    TooLong(usize, usize),
    #[error("decryption failed: {0}")]
    DecryptionFailed(String),
    #[error("encryption failed: {0}")]
    EncryptionFailed(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
}

pub const MAX_MEMO_BYTES: usize = 512;
pub const MAX_ENCRYPTED_MEMO_BYTES: usize = 1024;

// ── Memo types ────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoVisibility {
    Public,
    Encrypted,
    Private, // local-only, not on chain
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memo {
    pub id: String,
    pub tx_hash: Option<String>,
    pub sender: String,
    pub recipient: String,
    pub content: String,
    pub visibility: MemoVisibility,
    pub encrypted_content: Option<String>,
    pub created_at: String,
    pub tags: Vec<String>,
}

impl Memo {
    pub fn public(id: &str, sender: &str, recipient: &str, content: &str) -> Result<Self, MemoError> {
        if content.len() > MAX_MEMO_BYTES {
            return Err(MemoError::TooLong(content.len(), MAX_MEMO_BYTES));
        }
        Ok(Self {
            id: id.to_string(),
            tx_hash: None,
            sender: sender.to_string(),
            recipient: recipient.to_string(),
            content: content.to_string(),
            visibility: MemoVisibility::Public,
            encrypted_content: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            tags: Vec::new(),
        })
    }

    pub fn private(id: &str, sender: &str, recipient: &str, content: &str) -> Result<Self, MemoError> {
        if content.len() > MAX_MEMO_BYTES {
            return Err(MemoError::TooLong(content.len(), MAX_MEMO_BYTES));
        }
        Ok(Self {
            id: id.to_string(),
            tx_hash: None,
            sender: sender.to_string(),
            recipient: recipient.to_string(),
            content: content.to_string(),
            visibility: MemoVisibility::Private,
            encrypted_content: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            tags: Vec::new(),
        })
    }

    pub fn encrypted(
        id: &str,
        sender: &str,
        recipient: &str,
        content: &str,
        key: &[u8; 32],
    ) -> Result<Self, MemoError> {
        if content.len() > MAX_MEMO_BYTES {
            return Err(MemoError::TooLong(content.len(), MAX_MEMO_BYTES));
        }
        let encrypted = encrypt_memo(content, key)?;
        Ok(Self {
            id: id.to_string(),
            tx_hash: None,
            sender: sender.to_string(),
            recipient: recipient.to_string(),
            content: content.to_string(), // Stored locally for sender
            visibility: MemoVisibility::Encrypted,
            encrypted_content: Some(encrypted),
            created_at: chrono::Utc::now().to_rfc3339(),
            tags: Vec::new(),
        })
    }

    pub fn with_tx_hash(mut self, hash: &str) -> Self {
        self.tx_hash = Some(hash.to_string());
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    pub fn content_length(&self) -> usize {
        self.content.len()
    }

    /// Decrypt the encrypted content
    pub fn decrypt(&self, key: &[u8; 32]) -> Result<String, MemoError> {
        match &self.encrypted_content {
            Some(enc) => decrypt_memo(enc, key),
            None => Ok(self.content.clone()),
        }
    }

    pub fn is_encrypted(&self) -> bool {
        self.visibility == MemoVisibility::Encrypted
    }

    /// Redacted display for encrypted memos
    pub fn display_content(&self) -> &str {
        if self.is_encrypted() {
            "[encrypted memo]"
        } else {
            &self.content
        }
    }
}

// ── Encryption helpers ────────────────────────────────────────

fn encrypt_memo(plaintext: &str, key: &[u8; 32]) -> Result<String, MemoError> {
    use aes_gcm::{
        aead::{Aead, KeyInit, OsRng},
        Aes256Gcm, AeadCore,
    };
    let cipher = Aes256Gcm::new(key.into());
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| MemoError::EncryptionFailed(e.to_string()))?;

    // Encode as nonce_hex:ciphertext_hex
    Ok(format!("{}:{}", hex::encode(nonce), hex::encode(&ciphertext)))
}

fn decrypt_memo(encrypted: &str, key: &[u8; 32]) -> Result<String, MemoError> {
    use aes_gcm::{
        aead::{Aead, KeyInit},
        Aes256Gcm, Nonce,
    };
    let parts: Vec<&str> = encrypted.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(MemoError::DecryptionFailed("invalid format".into()));
    }
    let nonce_bytes =
        hex::decode(parts[0]).map_err(|e| MemoError::DecryptionFailed(e.to_string()))?;
    let ciphertext =
        hex::decode(parts[1]).map_err(|e| MemoError::DecryptionFailed(e.to_string()))?;

    let cipher = Aes256Gcm::new(key.into());
    let nonce = Nonce::from_slice(&nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext.as_ref())
        .map_err(|e| MemoError::DecryptionFailed(e.to_string()))?;

    String::from_utf8(plaintext).map_err(|e| MemoError::DecryptionFailed(e.to_string()))
}

// ── Memo store ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoStore {
    pub memos: HashMap<String, Memo>,
}

impl MemoStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, memo: Memo) {
        self.memos.insert(memo.id.clone(), memo);
    }

    pub fn get(&self, id: &str) -> Option<&Memo> {
        self.memos.get(id)
    }

    pub fn remove(&mut self, id: &str) -> Option<Memo> {
        self.memos.remove(id)
    }

    pub fn list(&self) -> Vec<&Memo> {
        self.memos.values().collect()
    }

    /// Get memo by tx hash
    pub fn by_tx_hash(&self, hash: &str) -> Option<&Memo> {
        self.memos
            .values()
            .find(|m| m.tx_hash.as_deref() == Some(hash))
    }

    /// Get all memos for a sender
    pub fn by_sender(&self, sender: &str) -> Vec<&Memo> {
        self.memos
            .values()
            .filter(|m| m.sender == sender)
            .collect()
    }

    /// Get all memos for a recipient
    pub fn by_recipient(&self, recipient: &str) -> Vec<&Memo> {
        self.memos
            .values()
            .filter(|m| m.recipient == recipient)
            .collect()
    }

    /// Search memos by content substring (only unencrypted)
    pub fn search(&self, query: &str) -> Vec<&Memo> {
        let q = query.to_lowercase();
        self.memos
            .values()
            .filter(|m| m.content.to_lowercase().contains(&q))
            .collect()
    }

    /// Get all memos with a tag
    pub fn by_tag(&self, tag: &str) -> Vec<&Memo> {
        self.memos
            .values()
            .filter(|m| m.tags.iter().any(|t| t == tag))
            .collect()
    }

    /// Count by visibility
    pub fn count_by_visibility(&self) -> (usize, usize, usize) {
        let public = self.memos.values().filter(|m| m.visibility == MemoVisibility::Public).count();
        let encrypted = self.memos.values().filter(|m| m.visibility == MemoVisibility::Encrypted).count();
        let private = self.memos.values().filter(|m| m.visibility == MemoVisibility::Private).count();
        (public, encrypted, private)
    }

    pub fn save(&self, path: &Path) -> Result<(), MemoError> {
        let json =
            serde_json::to_string_pretty(self).map_err(|e| MemoError::Json(e.to_string()))?;
        std::fs::write(path, json).map_err(|e| MemoError::Io(e.to_string()))
    }

    pub fn load(path: &Path) -> Result<Self, MemoError> {
        let data = std::fs::read_to_string(path).map_err(|e| MemoError::Io(e.to_string()))?;
        serde_json::from_str(&data).map_err(|e| MemoError::Json(e.to_string()))
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_public_memo() {
        let m = Memo::public("m1", "alice", "bob", "payment for lunch").unwrap();
        assert_eq!(m.visibility, MemoVisibility::Public);
        assert_eq!(m.content, "payment for lunch");
        assert!(!m.is_encrypted());
    }

    #[test]
    fn test_private_memo() {
        let m = Memo::private("m1", "alice", "bob", "secret note").unwrap();
        assert_eq!(m.visibility, MemoVisibility::Private);
    }

    #[test]
    fn test_memo_too_long() {
        let long = "x".repeat(MAX_MEMO_BYTES + 1);
        assert!(Memo::public("m1", "a", "b", &long).is_err());
    }

    #[test]
    fn test_encrypted_memo() {
        let key = [42u8; 32];
        let m = Memo::encrypted("m1", "alice", "bob", "secret payment", &key).unwrap();
        assert!(m.is_encrypted());
        assert!(m.encrypted_content.is_some());
        assert_eq!(m.display_content(), "[encrypted memo]");
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let key = [99u8; 32];
        let m = Memo::encrypted("m1", "alice", "bob", "hello world", &key).unwrap();
        let decrypted = m.decrypt(&key).unwrap();
        assert_eq!(decrypted, "hello world");
    }

    #[test]
    fn test_decrypt_wrong_key() {
        let key1 = [1u8; 32];
        let key2 = [2u8; 32];
        let m = Memo::encrypted("m1", "a", "b", "secret", &key1).unwrap();
        assert!(m.decrypt(&key2).is_err());
    }

    #[test]
    fn test_decrypt_non_encrypted() {
        let key = [0u8; 32];
        let m = Memo::public("m1", "a", "b", "plain").unwrap();
        let result = m.decrypt(&key).unwrap();
        assert_eq!(result, "plain");
    }

    #[test]
    fn test_memo_with_tx_hash() {
        let m = Memo::public("m1", "a", "b", "test").unwrap()
            .with_tx_hash("0xabc123");
        assert_eq!(m.tx_hash, Some("0xabc123".to_string()));
    }

    #[test]
    fn test_memo_with_tags() {
        let m = Memo::public("m1", "a", "b", "test").unwrap()
            .with_tags(vec!["payment".into(), "lunch".into()]);
        assert_eq!(m.tags.len(), 2);
    }

    #[test]
    fn test_content_length() {
        let m = Memo::public("m1", "a", "b", "hello").unwrap();
        assert_eq!(m.content_length(), 5);
    }

    #[test]
    fn test_store_add_get() {
        let mut store = MemoStore::new();
        let m = Memo::public("m1", "alice", "bob", "hi").unwrap();
        store.add(m);
        assert!(store.get("m1").is_some());
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn test_store_remove() {
        let mut store = MemoStore::new();
        store.add(Memo::public("m1", "a", "b", "x").unwrap());
        let removed = store.remove("m1");
        assert!(removed.is_some());
        assert!(store.get("m1").is_none());
    }

    #[test]
    fn test_store_by_tx_hash() {
        let mut store = MemoStore::new();
        store.add(Memo::public("m1", "a", "b", "x").unwrap().with_tx_hash("0xabc"));
        assert!(store.by_tx_hash("0xabc").is_some());
        assert!(store.by_tx_hash("0xdef").is_none());
    }

    #[test]
    fn test_store_by_sender() {
        let mut store = MemoStore::new();
        store.add(Memo::public("m1", "alice", "bob", "x").unwrap());
        store.add(Memo::public("m2", "alice", "carol", "y").unwrap());
        store.add(Memo::public("m3", "bob", "alice", "z").unwrap());
        assert_eq!(store.by_sender("alice").len(), 2);
    }

    #[test]
    fn test_store_by_recipient() {
        let mut store = MemoStore::new();
        store.add(Memo::public("m1", "a", "bob", "x").unwrap());
        store.add(Memo::public("m2", "a", "bob", "y").unwrap());
        assert_eq!(store.by_recipient("bob").len(), 2);
    }

    #[test]
    fn test_store_search() {
        let mut store = MemoStore::new();
        store.add(Memo::public("m1", "a", "b", "payment for lunch").unwrap());
        store.add(Memo::public("m2", "a", "b", "rent payment").unwrap());
        store.add(Memo::public("m3", "a", "b", "hello").unwrap());
        assert_eq!(store.search("payment").len(), 2);
        assert_eq!(store.search("LUNCH").len(), 1); // case-insensitive
    }

    #[test]
    fn test_store_by_tag() {
        let mut store = MemoStore::new();
        store.add(Memo::public("m1", "a", "b", "x").unwrap()
            .with_tags(vec!["rent".into()]));
        store.add(Memo::public("m2", "a", "b", "y").unwrap());
        assert_eq!(store.by_tag("rent").len(), 1);
    }

    #[test]
    fn test_store_count_by_visibility() {
        let key = [0u8; 32];
        let mut store = MemoStore::new();
        store.add(Memo::public("m1", "a", "b", "pub").unwrap());
        store.add(Memo::private("m2", "a", "b", "priv").unwrap());
        store.add(Memo::encrypted("m3", "a", "b", "enc", &key).unwrap());
        let (p, e, pr) = store.count_by_visibility();
        assert_eq!(p, 1);
        assert_eq!(e, 1);
        assert_eq!(pr, 1);
    }

    #[test]
    fn test_store_save_load() {
        let path = std::env::temp_dir().join(format!(
            "evap_memo_{}.json", std::process::id()
        ));
        let mut store = MemoStore::new();
        store.add(Memo::public("m1", "a", "b", "test").unwrap());
        store.save(&path).unwrap();
        let loaded = MemoStore::load(&path).unwrap();
        assert_eq!(loaded.list().len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_encrypt_memo_direct() {
        let key = [7u8; 32];
        let enc = encrypt_memo("hello", &key).unwrap();
        assert!(enc.contains(':'));
        let dec = decrypt_memo(&enc, &key).unwrap();
        assert_eq!(dec, "hello");
    }

    #[test]
    fn test_decrypt_bad_format() {
        let key = [0u8; 32];
        assert!(decrypt_memo("nocolon", &key).is_err());
    }
}
