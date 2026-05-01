//! Threshold encrypted mempool for MEV protection.
//!
//! Transactions go through a commit-reveal scheme:
//! 1. **COMMIT** — user submits encrypted transaction. Validators order it
//!    without seeing contents (ordered by commitment hash — deterministic,
//!    unmanipulable).
//! 2. **REVEAL** — after ordering is committed (reveal_delay epochs later),
//!    transactions are decrypted and executed. No validator can front-run
//!    because they can't read what they're ordering.
//!
//! Uses AES-256-GCM for encryption and BLAKE3 for commitments.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use evaporchain_crypto::hash::blake3_hash;
use evaporchain_types::Transaction;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ─────────────────────── Errors ──────────────────────────────────────────

#[derive(Debug, Error)]
pub enum MevError {
    #[error("commitment mismatch: expected {expected}, got {actual}")]
    CommitmentMismatch { expected: String, actual: String },
    #[error("decryption failed: {0}")]
    DecryptionFailed(String),
    #[error("reveal too early: current epoch {current}, reveal epoch {reveal_at}")]
    RevealTooEarly { current: u64, reveal_at: u64 },
    #[error("deserialization failed: {0}")]
    DeserializationFailed(String),
    #[error("nonce hash mismatch")]
    NonceHashMismatch,
}

// ─────────────────────── Types ───────────────────────────────────────────

/// An encrypted transaction in the commit phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedTransaction {
    /// BLAKE3 hash of (plaintext_bytes || nonce) — the commitment.
    pub commitment: [u8; 32],
    /// AES-256-GCM encrypted transaction payload.
    pub encrypted_payload: Vec<u8>,
    /// BLAKE3 hash of the nonce (for verification after reveal).
    pub nonce_hash: [u8; 32],
    /// Epoch when this transaction was submitted.
    pub submitted_epoch: u64,
}

/// Transaction envelope — can hold plaintext, encrypted, or revealed tx.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransactionEnvelope {
    /// Standard unencrypted transaction.
    Plaintext(Transaction),
    /// MEV-protected encrypted transaction (commit phase).
    Encrypted(EncryptedTransaction),
    /// Revealed transaction (after commit phase).
    Revealed {
        encrypted: EncryptedTransaction,
        plaintext: Transaction,
        nonce: [u8; 32],
    },
}

// ─────────────────────── Encrypt / Decrypt ───────────────────────────────

/// Encrypt a transaction for MEV protection.
///
/// The nonce is used to:
/// 1. Derive the AES-256-GCM key (BLAKE3 hash of "EvaporChain_MEV_Key" || nonce)
/// 2. Compute the commitment (BLAKE3 hash of plaintext_bytes || nonce)
/// 3. Store nonce_hash for post-reveal verification
pub fn encrypt_transaction(
    tx: &Transaction,
    nonce: &[u8; 32],
    current_epoch: u64,
) -> EncryptedTransaction {
    let plaintext_bytes =
        serde_json::to_vec(tx).expect("transaction serialization should not fail");

    // Commitment = H(plaintext || nonce)
    let mut commit_input = plaintext_bytes.clone();
    commit_input.extend_from_slice(nonce);
    let commitment = blake3_hash(&commit_input);

    // Derive AES key from nonce
    let aes_key = derive_aes_key(nonce);

    // Encrypt with AES-256-GCM
    let cipher = Aes256Gcm::new_from_slice(&aes_key).expect("valid key length");
    // Use first 12 bytes of nonce hash as GCM nonce
    let nonce_hash = blake3_hash(nonce);
    let gcm_nonce = Nonce::from_slice(&nonce_hash[..12]);
    let encrypted_payload = cipher
        .encrypt(gcm_nonce, plaintext_bytes.as_ref())
        .expect("encryption should not fail");

    EncryptedTransaction {
        commitment,
        encrypted_payload,
        nonce_hash,
        submitted_epoch: current_epoch,
    }
}

/// Verify commitment and decrypt an encrypted transaction.
pub fn verify_and_decrypt(
    encrypted: &EncryptedTransaction,
    nonce: &[u8; 32],
) -> Result<Transaction, MevError> {
    // Verify nonce hash
    let computed_nonce_hash = blake3_hash(nonce);
    if computed_nonce_hash != encrypted.nonce_hash {
        return Err(MevError::NonceHashMismatch);
    }

    // Derive AES key and decrypt
    let aes_key = derive_aes_key(nonce);
    let cipher = Aes256Gcm::new_from_slice(&aes_key)
        .map_err(|e| MevError::DecryptionFailed(e.to_string()))?;
    let gcm_nonce = Nonce::from_slice(&encrypted.nonce_hash[..12]);

    let plaintext_bytes = cipher
        .decrypt(gcm_nonce, encrypted.encrypted_payload.as_ref())
        .map_err(|e| MevError::DecryptionFailed(e.to_string()))?;

    // Verify commitment
    let mut commit_input = plaintext_bytes.clone();
    commit_input.extend_from_slice(nonce);
    let computed_commitment = blake3_hash(&commit_input);

    if computed_commitment != encrypted.commitment {
        return Err(MevError::CommitmentMismatch {
            expected: hex::encode(encrypted.commitment),
            actual: hex::encode(computed_commitment),
        });
    }

    // Deserialize
    let tx: Transaction = serde_json::from_slice(&plaintext_bytes)
        .map_err(|e| MevError::DeserializationFailed(e.to_string()))?;

    Ok(tx)
}

/// Derive AES-256 key from a nonce using BLAKE3.
fn derive_aes_key(nonce: &[u8; 32]) -> [u8; 32] {
    let mut key_input = Vec::with_capacity(32 + 22);
    key_input.extend_from_slice(b"EvaporChain_MEV_Key_");
    key_input.extend_from_slice(nonce);
    blake3_hash(&key_input)
}

// ─────────────────────── EncryptedMempool ─────────────────────────────────

/// MEV-protected mempool with commit-reveal scheme.
pub struct EncryptedMempool {
    /// Encrypted transactions waiting for reveal.
    pending_encrypted: Vec<EncryptedTransaction>,
    /// Standard plaintext transactions.
    pending_plaintext: Vec<Transaction>,
    /// Number of epochs between commit and reveal.
    reveal_delay: u64,
}

impl EncryptedMempool {
    /// Create a new encrypted mempool.
    pub fn new(reveal_delay: u64) -> Self {
        Self {
            pending_encrypted: Vec::new(),
            pending_plaintext: Vec::new(),
            reveal_delay,
        }
    }

    /// Submit an encrypted transaction (commit phase).
    pub fn submit_encrypted(&mut self, encrypted_tx: EncryptedTransaction) {
        self.pending_encrypted.push(encrypted_tx);
    }

    /// Submit a standard plaintext transaction.
    pub fn submit_plaintext(&mut self, tx: Transaction) {
        self.pending_plaintext.push(tx);
    }

    /// Get the committed ordering for block production at the given epoch.
    ///
    /// Returns envelopes ordered deterministically:
    /// - Encrypted txs sorted by commitment hash (unmanipulable ordering)
    /// - Plaintext txs appended after encrypted ones (FIFO)
    pub fn get_committed_ordering(&self, _epoch: u64) -> Vec<TransactionEnvelope> {
        let mut envelopes = Vec::new();

        // Encrypted txs sorted by commitment hash — deterministic, unmanipulable
        let mut encrypted_sorted = self.pending_encrypted.clone();
        encrypted_sorted.sort_by_key(|a| a.commitment);

        for enc in encrypted_sorted {
            envelopes.push(TransactionEnvelope::Encrypted(enc));
        }

        // Plaintext txs in submission order
        for tx in &self.pending_plaintext {
            envelopes.push(TransactionEnvelope::Plaintext(tx.clone()));
        }

        envelopes
    }

    /// Reveal an encrypted transaction by providing its nonce.
    ///
    /// Verifies the commitment matches and returns the plaintext transaction.
    /// Fails if the reveal is attempted before the reveal delay.
    pub fn reveal(
        &self,
        encrypted: &EncryptedTransaction,
        nonce: &[u8; 32],
        current_epoch: u64,
    ) -> Result<Transaction, MevError> {
        let reveal_at = encrypted.submitted_epoch + self.reveal_delay;
        if current_epoch < reveal_at {
            return Err(MevError::RevealTooEarly {
                current: current_epoch,
                reveal_at,
            });
        }

        verify_and_decrypt(encrypted, nonce)
    }

    /// Process all reveals for the given epoch.
    ///
    /// Returns plaintext transactions for all encrypted txs whose reveal
    /// delay has passed, given a list of (encrypted_tx, nonce) pairs.
    /// Also drains the plaintext pool.
    pub fn process_reveals(
        &mut self,
        current_epoch: u64,
        nonces: &[([u8; 32], [u8; 32])], // (commitment, nonce) pairs
    ) -> Vec<Transaction> {
        let mut revealed_txs = Vec::new();

        // Build a map from commitment to nonce for quick lookup
        let nonce_map: std::collections::HashMap<[u8; 32], [u8; 32]> =
            nonces.iter().cloned().collect();

        // Process encrypted txs that are ready for reveal
        let mut remaining = Vec::new();
        for enc in self.pending_encrypted.drain(..) {
            let reveal_at = enc.submitted_epoch + self.reveal_delay;
            if current_epoch >= reveal_at {
                if let Some(nonce) = nonce_map.get(&enc.commitment) {
                    match verify_and_decrypt(&enc, nonce) {
                        Ok(tx) => revealed_txs.push(tx),
                        Err(_) => {
                            // Failed to decrypt — drop the tx (invalid reveal)
                        }
                    }
                } else {
                    // No nonce provided — tx expires (user didn't reveal)
                }
            } else {
                // Not ready yet — keep in pool
                remaining.push(enc);
            }
        }
        self.pending_encrypted = remaining;

        // Drain plaintext pool too
        revealed_txs.append(&mut self.pending_plaintext);

        revealed_txs
    }

    /// Count of pending transactions.
    pub fn pending_count(&self) -> (usize, usize) {
        (self.pending_encrypted.len(), self.pending_plaintext.len())
    }

    /// Total pending count.
    pub fn len(&self) -> usize {
        self.pending_encrypted.len() + self.pending_plaintext.len()
    }

    /// Whether the mempool is empty.
    pub fn is_empty(&self) -> bool {
        self.pending_encrypted.is_empty() && self.pending_plaintext.is_empty()
    }

    /// Reveal delay in epochs.
    pub fn reveal_delay(&self) -> u64 {
        self.reveal_delay
    }
}

impl Default for EncryptedMempool {
    fn default() -> Self {
        Self::new(2)
    }
}

// ─────────────────────────── Tests ───────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_types::TransferTx;
    use rand::RngCore;

    fn dummy_tx(amount: u64) -> Transaction {
        Transaction::Transfer(TransferTx {
            from: [1u8; 32],
            to: [2u8; 32],
            amount,
            nonce: 0,
            signature: None,
            public_key: None,
        })
    }

    fn random_nonce() -> [u8; 32] {
        let mut nonce = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut nonce);
        nonce
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let tx = dummy_tx(500);
        let nonce = random_nonce();

        let encrypted = encrypt_transaction(&tx, &nonce, 10);
        let decrypted = verify_and_decrypt(&encrypted, &nonce).unwrap();

        // Roundtrip: decrypted tx should match original
        let orig_bytes = serde_json::to_vec(&tx).unwrap();
        let dec_bytes = serde_json::to_vec(&decrypted).unwrap();
        assert_eq!(orig_bytes, dec_bytes);
    }

    #[test]
    fn test_commitment_matches_plaintext_and_nonce() {
        let tx = dummy_tx(100);
        let nonce = random_nonce();

        let encrypted = encrypt_transaction(&tx, &nonce, 1);

        // Manually compute commitment
        let plaintext_bytes = serde_json::to_vec(&tx).unwrap();
        let mut commit_input = plaintext_bytes;
        commit_input.extend_from_slice(&nonce);
        let expected_commitment = blake3_hash(&commit_input);

        assert_eq!(encrypted.commitment, expected_commitment);
    }

    #[test]
    fn test_wrong_nonce_fails_verification() {
        let tx = dummy_tx(200);
        let nonce = random_nonce();
        let wrong_nonce = random_nonce();

        let encrypted = encrypt_transaction(&tx, &nonce, 1);
        let result = verify_and_decrypt(&encrypted, &wrong_nonce);

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            MevError::NonceHashMismatch | MevError::DecryptionFailed(_)
        ));
    }

    #[test]
    fn test_tampered_ciphertext_fails_decryption() {
        let tx = dummy_tx(300);
        let nonce = random_nonce();

        let mut encrypted = encrypt_transaction(&tx, &nonce, 1);
        // Tamper with the ciphertext
        if let Some(byte) = encrypted.encrypted_payload.first_mut() {
            *byte ^= 0xFF;
        }

        let result = verify_and_decrypt(&encrypted, &nonce);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), MevError::DecryptionFailed(_)));
    }

    #[test]
    fn test_ordering_is_deterministic_by_commitment() {
        let mut pool = EncryptedMempool::new(2);

        // Submit 5 encrypted txs with different nonces
        let mut commitments = Vec::new();
        for i in 0..5u64 {
            let tx = dummy_tx(i * 100);
            let nonce = random_nonce();
            let enc = encrypt_transaction(&tx, &nonce, 1);
            commitments.push(enc.commitment);
            pool.submit_encrypted(enc);
        }

        let ordering = pool.get_committed_ordering(1);

        // Extract commitment hashes from ordering
        let ordered_commitments: Vec<[u8; 32]> = ordering
            .iter()
            .filter_map(|env| match env {
                TransactionEnvelope::Encrypted(enc) => Some(enc.commitment),
                _ => None,
            })
            .collect();

        // Should be sorted by commitment hash
        let mut expected = ordered_commitments.clone();
        expected.sort();
        assert_eq!(ordered_commitments, expected);

        // Running again should produce same order
        let ordering2 = pool.get_committed_ordering(1);
        let ordered2: Vec<[u8; 32]> = ordering2
            .iter()
            .filter_map(|env| match env {
                TransactionEnvelope::Encrypted(enc) => Some(enc.commitment),
                _ => None,
            })
            .collect();
        assert_eq!(ordered_commitments, ordered2);
    }

    #[test]
    fn test_reveal_after_delay_succeeds() {
        let pool = EncryptedMempool::new(2);
        let tx = dummy_tx(400);
        let nonce = random_nonce();

        let encrypted = encrypt_transaction(&tx, &nonce, 10);

        // Reveal at epoch 12 (submitted 10, delay 2) — should succeed
        let result = pool.reveal(&encrypted, &nonce, 12);
        assert!(result.is_ok());

        // Reveal at epoch 13 — also fine (past delay)
        let result = pool.reveal(&encrypted, &nonce, 13);
        assert!(result.is_ok());
    }

    #[test]
    fn test_early_reveal_rejected() {
        let pool = EncryptedMempool::new(2);
        let tx = dummy_tx(400);
        let nonce = random_nonce();

        let encrypted = encrypt_transaction(&tx, &nonce, 10);

        // Try reveal at epoch 11 (need epoch 12) — should fail
        let result = pool.reveal(&encrypted, &nonce, 11);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            MevError::RevealTooEarly { .. }
        ));

        // Try reveal at submitted epoch — should fail
        let result = pool.reveal(&encrypted, &nonce, 10);
        assert!(result.is_err());
    }

    #[test]
    fn test_mixed_encrypted_and_plaintext_mempool() {
        let mut pool = EncryptedMempool::new(2);

        // Submit 2 plaintext
        pool.submit_plaintext(dummy_tx(100));
        pool.submit_plaintext(dummy_tx(200));

        // Submit 2 encrypted
        let nonce1 = random_nonce();
        let nonce2 = random_nonce();
        pool.submit_encrypted(encrypt_transaction(&dummy_tx(300), &nonce1, 1));
        pool.submit_encrypted(encrypt_transaction(&dummy_tx(400), &nonce2, 1));

        assert_eq!(pool.pending_count(), (2, 2));
        assert_eq!(pool.len(), 4);

        let ordering = pool.get_committed_ordering(1);
        assert_eq!(ordering.len(), 4);

        // First 2 should be encrypted (sorted by commitment), last 2 plaintext
        assert!(matches!(ordering[0], TransactionEnvelope::Encrypted(_)));
        assert!(matches!(ordering[1], TransactionEnvelope::Encrypted(_)));
        assert!(matches!(ordering[2], TransactionEnvelope::Plaintext(_)));
        assert!(matches!(ordering[3], TransactionEnvelope::Plaintext(_)));
    }

    #[test]
    fn test_full_cycle_encrypt_submit_order_reveal_execute() {
        let mut pool = EncryptedMempool::new(2);
        let tx = dummy_tx(999);
        let nonce = random_nonce();

        // 1. Encrypt
        let encrypted = encrypt_transaction(&tx, &nonce, 5);
        let commitment = encrypted.commitment;

        // 2. Submit
        pool.submit_encrypted(encrypted);
        assert_eq!(pool.pending_count(), (1, 0));

        // 3. Order (commit phase)
        let ordering = pool.get_committed_ordering(5);
        assert_eq!(ordering.len(), 1);

        // 4. Reveal (after delay)
        let revealed_txs = pool.process_reveals(7, &[(commitment, nonce)]);
        assert_eq!(revealed_txs.len(), 1);

        // 5. Verify decrypted tx matches original
        let revealed_bytes = serde_json::to_vec(&revealed_txs[0]).unwrap();
        let orig_bytes = serde_json::to_vec(&tx).unwrap();
        assert_eq!(revealed_bytes, orig_bytes);

        // Pool should be empty now
        assert!(pool.is_empty());
    }

    #[test]
    fn test_multiple_encrypted_txs_maintain_order() {
        let mut pool = EncryptedMempool::new(1);
        let mut nonces = Vec::new();
        let mut commitments = Vec::new();

        for i in 0..10u64 {
            let tx = dummy_tx(i * 100);
            let nonce = random_nonce();
            let enc = encrypt_transaction(&tx, &nonce, 1);
            commitments.push(enc.commitment);
            nonces.push((enc.commitment, nonce));
            pool.submit_encrypted(enc);
        }

        // Process reveals at epoch 2 (delay=1, submitted at 1)
        let revealed = pool.process_reveals(2, &nonces);
        assert_eq!(revealed.len(), 10);
        assert!(pool.is_empty());
    }

    #[test]
    fn test_pending_count_tracks_correctly() {
        let mut pool = EncryptedMempool::new(2);
        assert_eq!(pool.pending_count(), (0, 0));
        assert!(pool.is_empty());

        pool.submit_plaintext(dummy_tx(1));
        assert_eq!(pool.pending_count(), (0, 1));

        let nonce = random_nonce();
        pool.submit_encrypted(encrypt_transaction(&dummy_tx(2), &nonce, 1));
        assert_eq!(pool.pending_count(), (1, 1));
        assert_eq!(pool.len(), 2);
        assert!(!pool.is_empty());
    }

    #[test]
    fn test_process_reveals_keeps_unrevealed_if_too_early() {
        let mut pool = EncryptedMempool::new(3);
        let nonce = random_nonce();
        let enc = encrypt_transaction(&dummy_tx(100), &nonce, 10);
        let commitment = enc.commitment;
        pool.submit_encrypted(enc);

        // Try reveal at epoch 12 (need 13) — too early, should keep in pool
        let revealed = pool.process_reveals(12, &[(commitment, nonce)]);
        assert!(revealed.is_empty());
        assert_eq!(pool.pending_count(), (1, 0));

        // At epoch 13 — should reveal
        let revealed = pool.process_reveals(13, &[(commitment, nonce)]);
        assert_eq!(revealed.len(), 1);
        assert!(pool.is_empty());
    }

    #[test]
    fn test_nonce_hash_stored_correctly() {
        let tx = dummy_tx(42);
        let nonce = random_nonce();

        let encrypted = encrypt_transaction(&tx, &nonce, 1);

        // nonce_hash should be BLAKE3(nonce)
        let expected = blake3_hash(&nonce);
        assert_eq!(encrypted.nonce_hash, expected);
    }

    #[test]
    fn test_different_txs_different_commitments() {
        let nonce = random_nonce();
        let enc1 = encrypt_transaction(&dummy_tx(100), &nonce, 1);

        let nonce2 = random_nonce();
        let enc2 = encrypt_transaction(&dummy_tx(200), &nonce2, 1);

        assert_ne!(enc1.commitment, enc2.commitment);
    }

    #[test]
    fn test_same_tx_different_nonce_different_commitment() {
        let tx = dummy_tx(500);
        let nonce1 = random_nonce();
        let nonce2 = random_nonce();

        let enc1 = encrypt_transaction(&tx, &nonce1, 1);
        let enc2 = encrypt_transaction(&tx, &nonce2, 1);

        assert_ne!(enc1.commitment, enc2.commitment);
        assert_ne!(enc1.encrypted_payload, enc2.encrypted_payload);
    }
}
