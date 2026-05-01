//! Mnemonic seed phrase system for key recovery.
//!
//! Generates BIP-39 compatible 24-word mnemonic phrases that serve as
//! **encryption keys** for ML-DSA keypair backups. Since post-quantum
//! signature schemes (pqcrypto-dilithium) don't support seeded keypair
//! generation, the mnemonic encrypts the raw keypair bytes rather than
//! deriving the keypair deterministically.
//!
//! # Recovery Flow
//!
//! ```text
//! mnemonic (24 words)
//!   → entropy (256 bits)
//!     → seed = BLAKE3("evaporchain-seed" || entropy)
//!       → encryption_key = seed (32 bytes, AES-256-GCM key)
//!         → decrypt(encrypted_keypair) → ML-DSA keypair
//! ```
//!
//! # Backup Flow
//!
//! ```text
//! ML-DSA keypair + mnemonic
//!   → seed = derive_seed(mnemonic)
//!     → encrypt(pk || sk, seed) → MnemonicBackup
//! ```
//!
//! The wordlist is the standard BIP-39 English list (2048 words).

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

// ──────────────────────────── Constants ────────────────────────────────

/// Number of words in a mnemonic phrase.
pub const MNEMONIC_WORD_COUNT: usize = 24;

/// BIP-39 English wordlist (2048 words).
const WORDLIST: &str = include_str!("bip39_english.txt");

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum MnemonicError {
    #[error("invalid word count: expected {}, got {got}", MNEMONIC_WORD_COUNT)]
    InvalidWordCount { got: usize },
    #[error("unknown word in mnemonic: '{0}'")]
    UnknownWord(String),
    #[error("invalid checksum")]
    InvalidChecksum,
    #[error("key derivation failed: {0}")]
    DerivationFailed(String),
}

// ──────────────────────────── Mnemonic ─────────────────────────────────

/// A 24-word mnemonic phrase for wallet recovery.
#[derive(Clone)]
pub struct Mnemonic {
    words: Vec<String>,
    entropy: Vec<u8>,
}

impl Mnemonic {
    /// Generate a new random 24-word mnemonic.
    pub fn generate() -> Self {
        let mut entropy = vec![0u8; 32]; // 256 bits
        rand::thread_rng().fill_bytes(&mut entropy);
        Self::from_entropy(&entropy)
    }

    /// Create a mnemonic from raw 32-byte entropy.
    pub fn from_entropy(entropy: &[u8]) -> Self {
        let wordlist = load_wordlist();
        let checksum = blake3::hash(entropy);
        let checksum_byte = checksum.as_bytes()[0];

        // 256 bits of entropy + 8 bits of checksum = 264 bits = 24 words × 11 bits
        let mut bits = Vec::with_capacity(264);
        for byte in entropy {
            for i in (0..8).rev() {
                bits.push((byte >> i) & 1);
            }
        }
        // Add 8 checksum bits
        for i in (0..8).rev() {
            bits.push((checksum_byte >> i) & 1);
        }

        let mut words = Vec::with_capacity(MNEMONIC_WORD_COUNT);
        for chunk in bits.chunks(11) {
            let mut index: usize = 0;
            for &bit in chunk {
                index = (index << 1) | (bit as usize);
            }
            words.push(wordlist[index].clone());
        }

        Self {
            words,
            entropy: entropy.to_vec(),
        }
    }

    /// Parse a mnemonic from a space-separated phrase.
    pub fn from_phrase(phrase: &str) -> Result<Self, MnemonicError> {
        let words: Vec<String> = phrase
            .split_whitespace()
            .map(|w| w.to_lowercase())
            .collect();

        if words.len() != MNEMONIC_WORD_COUNT {
            return Err(MnemonicError::InvalidWordCount { got: words.len() });
        }

        let wordlist = load_wordlist();

        // Convert words back to indices
        let mut indices = Vec::with_capacity(MNEMONIC_WORD_COUNT);
        for word in &words {
            let idx = wordlist
                .iter()
                .position(|w| w == word)
                .ok_or_else(|| MnemonicError::UnknownWord(word.clone()))?;
            indices.push(idx);
        }

        // Convert indices back to bits
        let mut bits = Vec::with_capacity(264);
        for idx in &indices {
            for i in (0..11).rev() {
                bits.push(((idx >> i) & 1) as u8);
            }
        }

        // Extract entropy (first 256 bits) and checksum (last 8 bits)
        let entropy_bits = &bits[..256];
        let checksum_bits = &bits[256..264];

        let mut entropy = vec![0u8; 32];
        for (i, chunk) in entropy_bits.chunks(8).enumerate() {
            let mut byte = 0u8;
            for &bit in chunk {
                byte = (byte << 1) | bit;
            }
            entropy[i] = byte;
        }

        let mut checksum_byte = 0u8;
        for &bit in checksum_bits {
            checksum_byte = (checksum_byte << 1) | bit;
        }

        // Verify checksum
        let expected_checksum = blake3::hash(&entropy).as_bytes()[0];
        if checksum_byte != expected_checksum {
            return Err(MnemonicError::InvalidChecksum);
        }

        Ok(Self { words, entropy })
    }

    /// Get the mnemonic as a space-separated phrase.
    pub fn phrase(&self) -> String {
        self.words.join(" ")
    }

    /// Get the individual words.
    pub fn words(&self) -> &[String] {
        &self.words
    }

    /// Derive a 32-byte seed from the mnemonic.
    /// Uses BLAKE3 keyed hash: seed = BLAKE3("evaporchain-seed" || entropy).
    pub fn derive_seed(&self) -> [u8; 32] {
        let mut input = Vec::with_capacity(16 + self.entropy.len());
        input.extend_from_slice(b"evaporchain-seed");
        input.extend_from_slice(&self.entropy);
        let hash = blake3::hash(&input);
        input.zeroize();
        *hash.as_bytes()
    }

    /// Derive an encryption key for a specific account index.
    /// key = BLAKE3("evaporchain-seed" || entropy || index_le_bytes)
    pub fn derive_key_at(&self, index: u32) -> [u8; 32] {
        let mut input = Vec::with_capacity(16 + self.entropy.len() + 4);
        input.extend_from_slice(b"evaporchain-seed");
        input.extend_from_slice(&self.entropy);
        input.extend_from_slice(&index.to_le_bytes());
        let hash = blake3::hash(&input);
        input.zeroize();
        *hash.as_bytes()
    }

    /// Encrypt an ML-DSA keypair under this mnemonic's seed.
    /// Returns a serializable backup that can be recovered with the same mnemonic.
    pub fn backup_keypair(
        &self,
        keypair: &evaporchain_crypto::signatures::MlDsaKeypair,
    ) -> Result<MnemonicBackup, MnemonicError> {
        self.backup_keypair_at(keypair, 0)
    }

    /// Encrypt an ML-DSA keypair under this mnemonic at a specific account index.
    pub fn backup_keypair_at(
        &self,
        keypair: &evaporchain_crypto::signatures::MlDsaKeypair,
        index: u32,
    ) -> Result<MnemonicBackup, MnemonicError> {
        use evaporchain_crypto::signatures::Signer;

        let key = self.derive_key_at(index);

        // Serialize: pk_len (4 bytes LE) || pk || sk
        let pk = keypair.public_key_bytes();
        let sk = keypair.secret_key();
        let mut plaintext = Vec::with_capacity(4 + pk.len() + sk.len());
        plaintext.extend_from_slice(&(pk.len() as u32).to_le_bytes());
        plaintext.extend_from_slice(&pk);
        plaintext.extend_from_slice(sk);

        // Encrypt with AES-256-GCM
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);

        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|e| MnemonicError::DerivationFailed(e.to_string()))?;
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_ref())
            .map_err(|e| MnemonicError::DerivationFailed(e.to_string()))?;

        plaintext.zeroize();

        Ok(MnemonicBackup {
            version: 1,
            account_index: index,
            encrypted_keypair: hex::encode(&ciphertext),
            nonce: hex::encode(nonce_bytes),
            address: {
                let addr = crate::address::derive_address(&pk);
                crate::address::format_address(&addr)
            },
        })
    }

    /// Recover an ML-DSA keypair from a mnemonic backup.
    pub fn recover_keypair(
        &self,
        backup: &MnemonicBackup,
    ) -> Result<evaporchain_crypto::signatures::MlDsaKeypair, MnemonicError> {
        let key = self.derive_key_at(backup.account_index);

        let ciphertext = hex::decode(&backup.encrypted_keypair)
            .map_err(|e| MnemonicError::DerivationFailed(e.to_string()))?;
        let nonce_bytes = hex::decode(&backup.nonce)
            .map_err(|e| MnemonicError::DerivationFailed(e.to_string()))?;

        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|e| MnemonicError::DerivationFailed(e.to_string()))?;
        let mut plaintext = cipher
            .decrypt(Nonce::from_slice(&nonce_bytes), ciphertext.as_ref())
            .map_err(|_| {
                MnemonicError::DerivationFailed("wrong mnemonic or corrupted backup".into())
            })?;

        // Deserialize: pk_len (4 bytes LE) || pk || sk
        if plaintext.len() < 4 {
            return Err(MnemonicError::DerivationFailed("backup too short".into()));
        }
        let pk_len =
            u32::from_le_bytes([plaintext[0], plaintext[1], plaintext[2], plaintext[3]]) as usize;
        if plaintext.len() < 4 + pk_len {
            return Err(MnemonicError::DerivationFailed("backup truncated".into()));
        }
        let pk = &plaintext[4..4 + pk_len];
        let sk = &plaintext[4 + pk_len..];

        let result = evaporchain_crypto::signatures::MlDsaKeypair::from_bytes(pk, sk)
            .map_err(|e| MnemonicError::DerivationFailed(e.to_string()));

        plaintext.zeroize();
        result
    }
}

impl Drop for Mnemonic {
    fn drop(&mut self) {
        self.entropy.zeroize();
        for word in &mut self.words {
            word.zeroize();
        }
    }
}

// ──────────────────────────── Backup Format ───────────────────────────

/// Encrypted keypair backup protected by a mnemonic phrase.
///
/// The mnemonic-derived seed is used as the AES-256-GCM encryption key.
/// Recovery: mnemonic → seed → decrypt → ML-DSA keypair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MnemonicBackup {
    /// Format version.
    pub version: u32,
    /// Account index (for multi-account wallets).
    pub account_index: u32,
    /// AES-256-GCM encrypted keypair (hex-encoded).
    pub encrypted_keypair: String,
    /// AES-256-GCM nonce (hex-encoded, 12 bytes).
    pub nonce: String,
    /// Address derived from the public key (for identification).
    pub address: String,
}

impl MnemonicBackup {
    /// Serialize the backup to JSON.
    pub fn to_json(&self) -> Result<String, MnemonicError> {
        serde_json::to_string_pretty(self)
            .map_err(|e| MnemonicError::DerivationFailed(e.to_string()))
    }

    /// Deserialize a backup from JSON.
    pub fn from_json(json: &str) -> Result<Self, MnemonicError> {
        serde_json::from_str(json).map_err(|e| MnemonicError::DerivationFailed(e.to_string()))
    }
}

// ──────────────────────────── Wordlist ──────────────────────────────────

fn load_wordlist() -> Vec<String> {
    WORDLIST
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.trim().to_string())
        .collect()
}

// ──────────────────────────── Tests ────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_crypto::signatures::Signer;

    #[test]
    fn test_wordlist_has_2048_words() {
        let wl = load_wordlist();
        assert_eq!(wl.len(), 2048);
    }

    #[test]
    fn test_generate_mnemonic_24_words() {
        let m = Mnemonic::generate();
        assert_eq!(m.words().len(), 24);
    }

    #[test]
    fn test_mnemonic_phrase_roundtrip() {
        let m = Mnemonic::generate();
        let phrase = m.phrase();
        let parsed = Mnemonic::from_phrase(&phrase).unwrap();
        assert_eq!(parsed.phrase(), phrase);
    }

    #[test]
    fn test_mnemonic_from_entropy_deterministic() {
        let entropy = [42u8; 32];
        let m1 = Mnemonic::from_entropy(&entropy);
        let m2 = Mnemonic::from_entropy(&entropy);
        assert_eq!(m1.phrase(), m2.phrase());
    }

    #[test]
    fn test_mnemonic_different_entropy_different_phrase() {
        let m1 = Mnemonic::from_entropy(&[1u8; 32]);
        let m2 = Mnemonic::from_entropy(&[2u8; 32]);
        assert_ne!(m1.phrase(), m2.phrase());
    }

    #[test]
    fn test_derive_seed_deterministic() {
        let entropy = [42u8; 32];
        let m1 = Mnemonic::from_entropy(&entropy);
        let m2 = Mnemonic::from_entropy(&entropy);
        assert_eq!(m1.derive_seed(), m2.derive_seed());
    }

    #[test]
    fn test_derive_seed_different_for_different_mnemonics() {
        let m1 = Mnemonic::from_entropy(&[1u8; 32]);
        let m2 = Mnemonic::from_entropy(&[2u8; 32]);
        assert_ne!(m1.derive_seed(), m2.derive_seed());
    }

    #[test]
    fn test_derive_key_at_different_indices() {
        let m = Mnemonic::generate();
        let key0 = m.derive_key_at(0);
        let key1 = m.derive_key_at(1);
        assert_ne!(key0, key1);
    }

    #[test]
    fn test_derive_key_at_deterministic() {
        let entropy = [42u8; 32];
        let m1 = Mnemonic::from_entropy(&entropy);
        let m2 = Mnemonic::from_entropy(&entropy);
        assert_eq!(m1.derive_key_at(0), m2.derive_key_at(0));
        assert_eq!(m1.derive_key_at(5), m2.derive_key_at(5));
    }

    #[test]
    fn test_backup_and_recover_keypair() {
        use evaporchain_crypto::signatures::MlDsaKeypair;

        let m = Mnemonic::generate();
        let kp = MlDsaKeypair::generate();
        let original_pk = kp.public_key_bytes();

        let backup = m.backup_keypair(&kp).unwrap();
        let recovered = m.recover_keypair(&backup).unwrap();

        assert_eq!(recovered.public_key_bytes(), original_pk);
    }

    #[test]
    fn test_wrong_mnemonic_fails_recovery() {
        use evaporchain_crypto::signatures::MlDsaKeypair;

        let m1 = Mnemonic::generate();
        let m2 = Mnemonic::generate();
        let kp = MlDsaKeypair::generate();

        let backup = m1.backup_keypair(&kp).unwrap();
        let result = m2.recover_keypair(&backup);
        assert!(result.is_err());
    }

    #[test]
    fn test_backup_at_different_indices() {
        use evaporchain_crypto::signatures::MlDsaKeypair;

        let m = Mnemonic::generate();
        let kp = MlDsaKeypair::generate();

        let backup0 = m.backup_keypair_at(&kp, 0).unwrap();
        let backup1 = m.backup_keypair_at(&kp, 1).unwrap();

        // Different encryptions
        assert_ne!(backup0.encrypted_keypair, backup1.encrypted_keypair);

        // Both recover correctly
        let r0 = m.recover_keypair(&backup0).unwrap();
        let r1 = m.recover_keypair(&backup1).unwrap();
        assert_eq!(r0.public_key_bytes(), kp.public_key_bytes());
        assert_eq!(r1.public_key_bytes(), kp.public_key_bytes());
    }

    #[test]
    fn test_backup_json_roundtrip() {
        use evaporchain_crypto::signatures::MlDsaKeypair;

        let m = Mnemonic::generate();
        let kp = MlDsaKeypair::generate();

        let backup = m.backup_keypair(&kp).unwrap();
        let json = backup.to_json().unwrap();
        let parsed = MnemonicBackup::from_json(&json).unwrap();

        let recovered = m.recover_keypair(&parsed).unwrap();
        assert_eq!(recovered.public_key_bytes(), kp.public_key_bytes());
    }

    #[test]
    fn test_invalid_word_count() {
        let result = Mnemonic::from_phrase("hello world");
        assert!(matches!(
            result,
            Err(MnemonicError::InvalidWordCount { .. })
        ));
    }

    #[test]
    fn test_unknown_word() {
        let m = Mnemonic::generate();
        let mut words = m.words().to_vec();
        words[0] = "xyzzyplugh".to_string();
        let phrase = words.join(" ");
        let result = Mnemonic::from_phrase(&phrase);
        assert!(matches!(result, Err(MnemonicError::UnknownWord(_))));
    }

    #[test]
    fn test_invalid_checksum() {
        let m = Mnemonic::generate();
        let mut words = m.words().to_vec();
        // Swap last word to break checksum
        let wl = load_wordlist();
        let current = &words[23];
        let replacement = if current == &wl[0] { &wl[1] } else { &wl[0] };
        words[23] = replacement.to_string();
        let phrase = words.join(" ");
        let result = Mnemonic::from_phrase(&phrase);
        assert!(matches!(result, Err(MnemonicError::InvalidChecksum)));
    }

    #[test]
    fn test_all_words_in_wordlist() {
        let m = Mnemonic::generate();
        let wl = load_wordlist();
        for word in m.words() {
            assert!(wl.contains(word), "word '{}' not in wordlist", word);
        }
    }

    #[test]
    fn test_phrase_case_insensitive() {
        let m = Mnemonic::generate();
        let upper_phrase = m.phrase().to_uppercase();
        let parsed = Mnemonic::from_phrase(&upper_phrase).unwrap();
        assert_eq!(parsed.phrase(), m.phrase());
    }
}
