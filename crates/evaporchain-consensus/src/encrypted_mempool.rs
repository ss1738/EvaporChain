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

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use evaporchain_crypto::hash::blake3_hash;
use evaporchain_types::Transaction;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// PRIV-N5 (audit 2026-05-15) domain-separation tags.
//
// `MEV_AAD` is bound into the AES-256-GCM AEAD so that any post-seal
// tamper of `submitted_epoch` or `nonce_hash` flips the auth tag and
// the envelope refuses to decrypt at reveal time. Without this, a
// gossip relay could rewrite `submitted_epoch` to push `reveal_at`
// forward indefinitely (the envelope just sits in the pool — the
// AEAD doesn't currently bind it).
//
// `MEV_ADM` is the structural admission-id: a hash over the fields
// the validator can actually re-derive at admission
// (`encrypted_payload`, `nonce_hash`, `submitted_epoch`). It is
// independent of the claimed `commitment` field. The commitment-
// based dedup (PRIV-N6) is kept for the standard duplicate case;
// the admission-id dedup catches the case where an attacker leaves
// ciphertext intact but tampers the `commitment` field to bypass
// PRIV-N6's check (and consume a pool slot under a freshly-claimed
// commitment hash).
const MEV_AAD_DST: &[u8] = b"EVAPORCHAIN_V1_MEV_AAD\0";
const MEV_ADM_DST: &[u8] = b"EVAPORCHAIN_V1_MEV_ADM\0";

/// Build the AEAD AAD bound to an encrypted envelope's
/// `(submitted_epoch, nonce_hash)` pair.  Used by both encrypt and
/// decrypt — they must agree byte-for-byte or the AEAD tag check
/// fails.  Layout: `DST || submitted_epoch (LE u64) || nonce_hash`.
fn build_mev_aad(submitted_epoch: u64, nonce_hash: &[u8; 32]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(MEV_AAD_DST.len() + 8 + 32);
    aad.extend_from_slice(MEV_AAD_DST);
    aad.extend_from_slice(&submitted_epoch.to_le_bytes());
    aad.extend_from_slice(nonce_hash);
    aad
}

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

impl EncryptedTransaction {
    /// PRIV-N5 (audit 2026-05-15): structural admission-id over the
    /// fields the validator can re-derive locally.  Independent of
    /// the claimed `commitment` field, so an attacker who keeps the
    /// ciphertext intact but rewrites `commitment` cannot bypass
    /// admission dedup.
    pub fn derived_admission_id(&self) -> [u8; 32] {
        let mut input =
            Vec::with_capacity(MEV_ADM_DST.len() + 8 + 32 + self.encrypted_payload.len() + 8);
        input.extend_from_slice(MEV_ADM_DST);
        input.extend_from_slice(&self.submitted_epoch.to_le_bytes());
        input.extend_from_slice(&self.nonce_hash);
        // length-prefix the payload so it can't collide with the
        // adjacent fields under variable-length concatenation.
        input.extend_from_slice(&(self.encrypted_payload.len() as u64).to_le_bytes());
        input.extend_from_slice(&self.encrypted_payload);
        blake3_hash(&input)
    }
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
    // PRIV-N5 (audit 2026-05-15): bind submitted_epoch + nonce_hash
    // into the AEAD via additional authenticated data. Tampering with
    // either field post-seal flips the GCM tag and decryption fails
    // at reveal — closing the "rewrite submitted_epoch to defer
    // reveal_at indefinitely" attack on gossiped envelopes.
    let aad = build_mev_aad(current_epoch, &nonce_hash);
    let encrypted_payload = cipher
        .encrypt(
            gcm_nonce,
            Payload {
                msg: plaintext_bytes.as_ref(),
                aad: aad.as_ref(),
            },
        )
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

    // PRIV-N5: reconstruct the AAD from the envelope's claimed
    // `(submitted_epoch, nonce_hash)`. If either field was tampered
    // post-seal, the AEAD tag check fails here.
    let aad = build_mev_aad(encrypted.submitted_epoch, &encrypted.nonce_hash);
    let plaintext_bytes = cipher
        .decrypt(
            gcm_nonce,
            Payload {
                msg: encrypted.encrypted_payload.as_ref(),
                aad: aad.as_ref(),
            },
        )
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

/// Maximum number of pending encrypted transactions. Anti-DoS cap —
/// without this, an attacker could flood the pool with arbitrary
/// commit submissions (the plaintext is opaque to validators at
/// admission, so per-tx content validation isn't possible until
/// reveal). T0.7 vector 4 (encrypted-mempool reveal flood).
///
/// Matched to the regular mempool's `MAX_MEMPOOL_SIZE = 10_000`
/// for consistency — both pools have the same per-validator memory
/// budget under load.
const MAX_ENCRYPTED_PENDING: usize = 10_000;

/// Maximum number of pending plaintext transactions in the
/// encrypted-mempool's plaintext side-pool. Same anti-DoS rationale;
/// same cap as MAX_ENCRYPTED_PENDING.
const MAX_PLAINTEXT_PENDING: usize = 10_000;

/// MEV-protected mempool with commit-reveal scheme.
pub struct EncryptedMempool {
    /// Encrypted transactions waiting for reveal.
    pending_encrypted: Vec<EncryptedTransaction>,
    /// O(1) commitment-dedup index (PRIV-N6).
    seen_commitments: std::collections::HashSet<[u8; 32]>,
    /// O(1) admission-id-dedup index (PRIV-N5).
    seen_admission_ids: std::collections::HashSet<[u8; 32]>,
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
            seen_commitments: std::collections::HashSet::new(),
            seen_admission_ids: std::collections::HashSet::new(),
            pending_plaintext: Vec::new(),
            reveal_delay,
        }
    }

    /// Submit an encrypted transaction (commit phase). Returns `true`
    /// if accepted, `false` if rejected because:
    /// - the encrypted pool is at `MAX_ENCRYPTED_PENDING` (T0.7
    ///   vector 4 — reveal flood admission cap), OR
    /// - the claimed commitment is already in the pool
    ///   (PRIV-N6 dedup), OR
    /// - the structural admission-id collides with an existing
    ///   envelope (PRIV-N5 admission re-derive).
    ///
    /// PRIV-N6 (audit 2026-05-15): without dedup, an attacker could
    /// submit the same envelope `MAX_ENCRYPTED_PENDING` times to
    /// crowd out honest commits; ordering by commitment then yields
    /// N copies of the same slot.
    ///
    /// PRIV-N5 (audit 2026-05-15): commitment-only dedup misses the
    /// "intact ciphertext, tampered commitment-field" duplicate. An
    /// attacker who replays the same `(encrypted_payload, nonce_hash,
    /// submitted_epoch)` with a fresh-looking but fake `commitment`
    /// passes PRIV-N6's check and consumes a fresh slot.  Dedup by
    /// `derived_admission_id()` (which doesn't read the claimed
    /// commitment) closes that gap.
    ///
    /// The O(n) linear scans here are fine at the 10k cap
    /// (~20k pointer compares per submit ≈ μs); a `HashSet<[u8;32]>`
    /// would be faster but adds a parallel data structure to keep in
    /// sync — not worth the complexity for the current envelope
    /// shape.
    pub fn submit_encrypted(&mut self, encrypted_tx: EncryptedTransaction) -> bool {
        // Cap check first (cheapest).
        if self.pending_encrypted.len() >= MAX_ENCRYPTED_PENDING {
            return false;
        }
        // O(1) commitment dedup (PRIV-N6).
        if self.seen_commitments.contains(&encrypted_tx.commitment) {
            return false;
        }
        // O(1) admission-id dedup (PRIV-N5).
        let incoming_admission_id = encrypted_tx.derived_admission_id();
        if self.seen_admission_ids.contains(&incoming_admission_id) {
            return false;
        }
        self.seen_commitments.insert(encrypted_tx.commitment);
        self.seen_admission_ids.insert(incoming_admission_id);
        self.pending_encrypted.push(encrypted_tx);
        true
    }

    /// Submit a standard plaintext transaction. Returns `true` if
    /// accepted, `false` if rejected because the plaintext pool is at
    /// `MAX_PLAINTEXT_PENDING`.
    pub fn submit_plaintext(&mut self, tx: Transaction) -> bool {
        if self.pending_plaintext.len() >= MAX_PLAINTEXT_PENDING {
            return false;
        }
        self.pending_plaintext.push(tx);
        true
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
        // Rebuild the O(1) dedup indices to match the surviving pool.
        self.seen_commitments = remaining.iter().map(|e| e.commitment).collect();
        self.seen_admission_ids = remaining.iter().map(|e| e.derived_admission_id()).collect();
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

// ─────────────────────── MevPool substrate seam ─────────────────────────
//
// Lane G.4. Today the chain holds a concrete `EncryptedMempool` field on
// `TendermintConsensus`. This trait names the abstract MEV-protection
// contract so alternative impls (threshold-encrypted with DKG, Ferveo /
// Shutter-style sealed mempools, sealed-bid orderflow auction pools)
// can plug into the consensus hot path without changing consensus code.
//
// Method set is the minimum that `tendermint.rs` calls on
// `self.encrypted_mempool` today:
//
// - `submit_encrypted` — admit a sealed tx into the commit phase.
// - `process_reveals` — drain the reveal-ready txs at a given epoch and
//   return the plaintext set for execution. Drains the plaintext pool
//   too as a side-effect (matches today's behaviour).
// - `pending_count` — `(encrypted, plaintext)` size for status RPCs.
//
// `len` / `is_empty` are convenience defaults derived from
// `pending_count`. The migration of `TendermintConsensus.encrypted_mempool`
// from `EncryptedMempool` to `Box<dyn MevPool>` is Lane G.5 work
// (separate commit) — this commit lands the trait + the blanket impl on
// `EncryptedMempool` so other impls can be written against a stable seam.
//
// Doctrine link: INVENTION_STACK.md §A1.4 (MEV defense via encrypted
// ordering) + the Layer 4 antichain-mempool work in DOCTRINE_PUNCH_LIST.md.

/// Substrate contract for MEV-protected mempools. `Send + Sync` so
/// consensus engines can hold this behind locks; `EncryptedMempool` is
/// already `Send + Sync` by virtue of its fields.
pub trait MevPool: Send + Sync {
    /// Admit an encrypted transaction into the commit phase. The
    /// validator orders this tx without seeing its contents — ordering
    /// is by commitment hash, deterministic and unmanipulable.
    /// Returns `true` if accepted, `false` if rejected due to pool
    /// capacity (T0.7 vector 4 — encrypted-mempool reveal-flood DoS).
    fn submit_encrypted(&mut self, encrypted_tx: EncryptedTransaction) -> bool;

    /// Process all reveals for the given epoch. Returns plaintext
    /// transactions for all encrypted txs whose reveal delay has passed,
    /// given a list of `(commitment, nonce)` pairs. Also drains the
    /// plaintext pool as a side-effect.
    fn process_reveals(
        &mut self,
        current_epoch: u64,
        nonces: &[([u8; 32], [u8; 32])],
    ) -> Vec<Transaction>;

    /// `(encrypted_count, plaintext_count)` — for status RPCs.
    fn pending_count(&self) -> (usize, usize);

    /// Total pending count. Default sums the two halves of `pending_count`.
    fn len(&self) -> usize {
        let (e, p) = self.pending_count();
        e + p
    }

    /// True iff `len() == 0`.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Lane G.4 blanket impl. Pure delegation to `EncryptedMempool`'s
/// inherent methods — no behaviour change.
impl MevPool for EncryptedMempool {
    fn submit_encrypted(&mut self, encrypted_tx: EncryptedTransaction) -> bool {
        EncryptedMempool::submit_encrypted(self, encrypted_tx)
    }

    fn process_reveals(
        &mut self,
        current_epoch: u64,
        nonces: &[([u8; 32], [u8; 32])],
    ) -> Vec<Transaction> {
        EncryptedMempool::process_reveals(self, current_epoch, nonces)
    }

    fn pending_count(&self) -> (usize, usize) {
        EncryptedMempool::pending_count(self)
    }

    fn len(&self) -> usize {
        EncryptedMempool::len(self)
    }

    fn is_empty(&self) -> bool {
        EncryptedMempool::is_empty(self)
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
            mev_refund_eligible: None,
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

    #[test]
    fn mev_pool_trait_delegates_to_encrypted_mempool() {
        // Lane G.4: trait dispatch parity. Hold an EncryptedMempool
        // behind `&mut dyn MevPool` and verify the four trait methods
        // produce results bit-equal to the inherent calls. Locks in
        // the substrate seam — alternative MEV pools (threshold-encrypted,
        // Ferveo-style, sealed-bid) can swap in here without consensus
        // code changes.
        let mut pool = EncryptedMempool::new(2);
        // Trait dispatch handle.
        let mp: &mut dyn MevPool = &mut pool;

        // Empty defaults.
        assert!(mp.is_empty());
        assert_eq!(mp.len(), 0);
        assert_eq!(mp.pending_count(), (0, 0));

        // Submit a sealed tx.
        let tx = dummy_tx(123);
        let nonce = random_nonce();
        let enc = encrypt_transaction(&tx, &nonce, 1);
        mp.submit_encrypted(enc.clone());

        // Counts now show one encrypted, zero plaintext.
        assert!(!mp.is_empty());
        assert_eq!(mp.len(), 1);
        assert_eq!(mp.pending_count(), (1, 0));

        // Reveal at the right epoch through the trait — the tx should
        // emerge as plaintext bit-equal to the original input.
        let nonces = vec![(enc.commitment, nonce)];
        let revealed = mp.process_reveals(3, &nonces);
        assert_eq!(revealed.len(), 1);
        // Compare via canonical bytes — the Transaction struct lacks
        // PartialEq for the embedded signature/public_key Option<Vec<u8>>
        // bookkeeping, but JSON roundtrip is deterministic for our test
        // shape.
        let orig = serde_json::to_vec(&tx).unwrap();
        let got = serde_json::to_vec(&revealed[0]).unwrap();
        assert_eq!(orig, got);
        // Pool is now drained.
        assert!(mp.is_empty());
    }

    // ── Lane T0.7 vector 4 — encrypted-mempool reveal-flood cap ─────
    //
    // The parallel-session `dos_resistance.rs` file ships vectors 1-3
    // and explicitly TODOs vector 4 ("Encrypted mempool reveal flood
    // — separate test file"). The underlying gap was that
    // `EncryptedMempool::submit_encrypted` had NO capacity bound, so
    // an attacker could flood arbitrary opaque commitments and exhaust
    // the validator's memory. Cap added here + tested below.

    /// Hitting MAX_ENCRYPTED_PENDING rejects further commits.
    ///
    /// PRIV-N5/PRIV-N6 (audit 2026-05-15): commitment-dedup +
    /// admission-id-dedup both reject identical envelopes, so the
    /// flood loop must vary the nonce across iterations to produce
    /// unique commitments / admission-ids and reach the cap.
    /// (Companion fix to commit 6360fcc5 which covered the parallel
    /// `dos_resistance::dos_v4_..._fires_on_flood` test.)
    #[test]
    fn encrypted_pool_rejects_when_at_capacity() {
        let mut pool = EncryptedMempool::new(1);
        let tx = dummy_tx(0);

        // Fill to capacity — every submit accepts. Use a unique
        // 32-byte nonce per iteration so commitments don't collide
        // with PRIV-N6 dedup / admission-ids don't collide with
        // PRIV-N5 dedup.
        for i in 0..MAX_ENCRYPTED_PENDING {
            let mut nonce = [0u8; 32];
            nonce[..8].copy_from_slice(&(i as u64).to_le_bytes());
            let enc = encrypt_transaction(&tx, &nonce, 1);
            assert!(pool.submit_encrypted(enc), "below cap must accept");
        }
        assert_eq!(pool.pending_count().0, MAX_ENCRYPTED_PENDING);

        // The (cap+1)-th is rejected (under-cap dedup wouldn't fire
        // because we use a fresh nonce here too — only the cap can
        // reject this one).
        let mut over_nonce = [0u8; 32];
        over_nonce[..8].copy_from_slice(&(MAX_ENCRYPTED_PENDING as u64).to_le_bytes());
        let over = encrypt_transaction(&tx, &over_nonce, 1);
        assert!(
            !pool.submit_encrypted(over),
            "at-cap commit must be rejected (T0.7 vector 4 — reveal-flood DoS)"
        );
        // Pool size unchanged.
        assert_eq!(pool.pending_count().0, MAX_ENCRYPTED_PENDING);
    }

    /// Symmetric for the plaintext side-pool.
    #[test]
    fn plaintext_pool_rejects_when_at_capacity() {
        let mut pool = EncryptedMempool::new(1);

        for nonce in 0..MAX_PLAINTEXT_PENDING as u64 {
            assert!(
                pool.submit_plaintext(dummy_tx(nonce)),
                "below cap must accept"
            );
        }
        assert_eq!(pool.pending_count().1, MAX_PLAINTEXT_PENDING);

        let over = pool.submit_plaintext(dummy_tx(MAX_PLAINTEXT_PENDING as u64));
        assert!(!over, "at-cap plaintext submit must be rejected");
        assert_eq!(pool.pending_count().1, MAX_PLAINTEXT_PENDING);
    }

    /// T1.20 — MevPool trait default `len()` and `is_empty()`
    /// route through pending_count. Previously the blanket-impl
    /// fall-through defaults were unreached (lines 383-391).
    #[test]
    fn t1_20_mev_pool_default_len_and_is_empty() {
        let pool: Box<dyn MevPool> = Box::new(EncryptedMempool::new(1));
        assert!(pool.is_empty());
        assert_eq!(pool.len(), 0);

        let mut pool: Box<dyn MevPool> = Box::new(EncryptedMempool::new(1));
        let nonce = [1u8; 32];
        let tx = dummy_tx(42);
        let enc = encrypt_transaction(&tx, &nonce, 1);
        pool.submit_encrypted(enc);
        assert!(!pool.is_empty());
        assert_eq!(pool.len(), 1);
    }

    /// T1.20 — EncryptedMempool::default() routes through
    /// new(reveal_delay=2). Covers lines 327-329.
    #[test]
    fn t1_20_encrypted_mempool_default_uses_delay_2() {
        let pool: EncryptedMempool = Default::default();
        assert_eq!(pool.reveal_delay(), 2);
    }

    /// T1.20 — verify_and_decrypt with mismatched commitment hash
    /// rejects via CommitmentMismatch. Covers lines 134-137.
    #[test]
    fn t1_20_verify_decrypt_commitment_mismatch_rejected() {
        let nonce = [9u8; 32];
        let tx = dummy_tx(7);
        let mut enc = encrypt_transaction(&tx, &nonce, 1);
        // Mutate the commitment so it no longer matches.
        enc.commitment[0] ^= 0xFF;
        let r = verify_and_decrypt(&enc, &nonce);
        match r {
            Err(MevError::CommitmentMismatch { .. }) => {}
            other => panic!("expected CommitmentMismatch, got {:?}", other),
        }
    }

    // ─────────── PRIV-N5 (audit 2026-05-15): AAD binding + admission re-derive ───────────

    /// PRIV-N5: tampering with `submitted_epoch` after seal must
    /// fail the AEAD tag check (DecryptionFailed), not silently
    /// allow late-revealed txs.  Without AAD binding, an attacker
    /// who controls a gossip relay could rewrite `submitted_epoch`
    /// to push `reveal_at = submitted_epoch + reveal_delay` past
    /// the user's intended reveal window.
    #[test]
    fn priv_n5_tampered_submitted_epoch_fails_decryption() {
        let nonce = random_nonce();
        let tx = dummy_tx(42);
        let mut enc = encrypt_transaction(&tx, &nonce, 100);

        // Move the envelope to a later "submission" epoch.
        enc.submitted_epoch = 100_000;

        let r = verify_and_decrypt(&enc, &nonce);
        match r {
            Err(MevError::DecryptionFailed(_)) => {}
            other => panic!(
                "expected DecryptionFailed from AAD tag mismatch, got {:?}",
                other
            ),
        }
    }

    /// PRIV-N5: tampering with `nonce_hash` after seal must also
    /// fail decryption.  `nonce_hash` doubles as the GCM nonce
    /// prefix and is part of the bound AAD — flipping it breaks
    /// both.  The AAD bind in particular guarantees the failure
    /// even if the GCM nonce coincidentally aligned.
    #[test]
    fn priv_n5_tampered_nonce_hash_fails_decryption() {
        let nonce = random_nonce();
        let tx = dummy_tx(42);
        let mut enc = encrypt_transaction(&tx, &nonce, 100);

        enc.nonce_hash[0] ^= 0xFF;

        let r = verify_and_decrypt(&enc, &nonce);
        // Could surface as NonceHashMismatch (caught first) OR
        // DecryptionFailed if the mismatch slips past the early
        // check on a degenerate fixture. Both are acceptable —
        // both reject.
        match r {
            Err(MevError::NonceHashMismatch) | Err(MevError::DecryptionFailed(_)) => {}
            other => panic!("expected rejection, got {:?}", other),
        }
    }

    /// PRIV-N5: untampered envelope still round-trips under AAD.
    /// Sanity that AAD binding didn't break the happy path.
    #[test]
    fn priv_n5_untampered_envelope_decrypts() {
        let nonce = random_nonce();
        let tx = dummy_tx(42);
        let enc = encrypt_transaction(&tx, &nonce, 100);
        let decrypted = verify_and_decrypt(&enc, &nonce).unwrap();
        let orig_bytes = serde_json::to_vec(&tx).unwrap();
        let dec_bytes = serde_json::to_vec(&decrypted).unwrap();
        assert_eq!(orig_bytes, dec_bytes);
    }

    /// PRIV-N5: structural admission-id is independent of the
    /// claimed `commitment` field.  Two envelopes that differ
    /// only in the (attacker-tampered) commitment field MUST
    /// produce the same admission-id.
    #[test]
    fn priv_n5_admission_id_ignores_commitment_field() {
        let nonce = random_nonce();
        let tx = dummy_tx(42);
        let enc = encrypt_transaction(&tx, &nonce, 100);
        let id_a = enc.derived_admission_id();

        let mut tampered = enc.clone();
        tampered.commitment[0] ^= 0xFF;
        let id_b = tampered.derived_admission_id();

        assert_eq!(id_a, id_b, "admission_id must not depend on commitment");
    }

    /// PRIV-N5: different `submitted_epoch` MUST produce a
    /// different admission-id.  Catches an honest user resubmitting
    /// at a new epoch (correctly admitted) vs. a duplicate replay
    /// (correctly rejected) and vice-versa.
    #[test]
    fn priv_n5_admission_id_depends_on_submitted_epoch() {
        let nonce = random_nonce();
        let tx = dummy_tx(42);
        let enc_a = encrypt_transaction(&tx, &nonce, 100);
        let enc_b = encrypt_transaction(&tx, &nonce, 101);

        assert_ne!(
            enc_a.derived_admission_id(),
            enc_b.derived_admission_id(),
            "different epoch must change admission_id"
        );
    }

    /// PRIV-N5: submit_encrypted rejects an envelope whose
    /// `commitment` field has been tampered but whose ciphertext,
    /// nonce_hash, and submitted_epoch are intact (PRIV-N6's
    /// commitment-only dedup would miss this).
    #[test]
    fn priv_n5_submit_rejects_commitment_tampered_duplicate() {
        let mut pool = EncryptedMempool::new(2);
        let nonce = random_nonce();
        let tx = dummy_tx(42);
        let enc = encrypt_transaction(&tx, &nonce, 100);

        assert!(pool.submit_encrypted(enc.clone()));

        // Same ciphertext + nonce_hash + epoch, but attacker tampers
        // the claimed commitment so PRIV-N6's commitment-dedup
        // wouldn't catch it.
        let mut tampered = enc.clone();
        tampered.commitment[0] ^= 0xFF;

        assert!(
            !pool.submit_encrypted(tampered),
            "admission-id dedup must reject commitment-tampered duplicate"
        );
        assert_eq!(pool.pending_count().0, 1);
    }

    /// PRIV-N5: legitimate resubmission at a *new* submitted_epoch
    /// must be accepted (admission-id differs).  This guards
    /// against over-rejection of users who re-encrypt for a fresh
    /// commit window.
    #[test]
    fn priv_n5_submit_accepts_resubmit_at_new_epoch() {
        let mut pool = EncryptedMempool::new(2);
        let tx = dummy_tx(42);
        let nonce_1 = random_nonce();
        let nonce_2 = random_nonce();

        let enc_a = encrypt_transaction(&tx, &nonce_1, 100);
        let enc_b = encrypt_transaction(&tx, &nonce_2, 200);

        assert!(pool.submit_encrypted(enc_a));
        assert!(
            pool.submit_encrypted(enc_b),
            "fresh re-encryption at later epoch must be admitted"
        );
        assert_eq!(pool.pending_count().0, 2);
    }
}
