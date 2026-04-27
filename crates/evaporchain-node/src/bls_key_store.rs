//! Optional encrypted-at-rest format for the validator BLS secret key.
//!
//! Two on-disk shapes are supported and auto-detected by length:
//! - **plaintext (32 bytes):** the historical raw `BlsSecretKey` bytes,
//!   protected only by file mode 0600.
//! - **encrypted (92 bytes):** Argon2id-derived key + XChaCha20-Poly1305
//!   AEAD over the 32-byte secret. Layout:
//!   `magic(4="EVK1") || salt(16) || nonce(24) || ciphertext(32+16=48)`.
//!
//! Encryption is opt-in: a node operator enables it by setting the
//! `EVAPORCHAIN_VALIDATOR_KEY_PASS` environment variable (or supplying a
//! passphrase through future CLI plumbing). When set, new keys are
//! written encrypted and existing encrypted keys can be decrypted with
//! the same passphrase. When unset, the historical plaintext path is
//! used and a warning is logged so the operator notices.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand::RngCore;

const MAGIC: &[u8; 4] = b"EVK1";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const PLAINTEXT_LEN: usize = 32;
const TAG_LEN: usize = 16;
pub const ENCRYPTED_LEN: usize = MAGIC.len() + SALT_LEN + NONCE_LEN + PLAINTEXT_LEN + TAG_LEN; // 92

pub const ENV_PASSPHRASE: &str = "EVAPORCHAIN_VALIDATOR_KEY_PASS";

/// Returns the operator-supplied passphrase (env var only for now).
/// Returns None when not set; caller must decide whether to fall back to
/// plaintext or fail.
pub fn passphrase_from_env() -> Option<Vec<u8>> {
    match std::env::var(ENV_PASSPHRASE) {
        Ok(p) if !p.is_empty() => Some(p.into_bytes()),
        _ => None,
    }
}

/// Derive a 32-byte symmetric key from a passphrase using Argon2id.
fn kdf(passphrase: &[u8], salt: &[u8]) -> Result<[u8; 32], String> {
    // m=64 MiB, t=3, p=1 — OWASP-recommended baseline for 2024+.
    let params = Params::new(64 * 1024, 3, 1, Some(32))
        .map_err(|e| format!("argon2 params: {e}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    argon
        .hash_password_into(passphrase, salt, &mut out)
        .map_err(|e| format!("argon2 derive: {e}"))?;
    Ok(out)
}

/// Encrypt a 32-byte BLS secret with a passphrase. Returns 92 bytes.
pub fn encrypt_bls_secret(secret: &[u8], passphrase: &[u8]) -> Result<Vec<u8>, String> {
    if secret.len() != PLAINTEXT_LEN {
        return Err(format!(
            "BLS secret must be {} bytes, got {}",
            PLAINTEXT_LEN,
            secret.len()
        ));
    }
    let mut salt = [0u8; SALT_LEN];
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);

    let key = kdf(passphrase, &salt)?;
    let cipher =
        XChaCha20Poly1305::new_from_slice(&key).map_err(|e| format!("cipher init: {e}"))?;
    let nonce = XNonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, secret)
        .map_err(|e| format!("encrypt: {e}"))?;

    let mut out = Vec::with_capacity(ENCRYPTED_LEN);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    debug_assert_eq!(out.len(), ENCRYPTED_LEN);
    Ok(out)
}

/// Decrypt a 92-byte encrypted blob with the supplied passphrase.
pub fn decrypt_bls_secret(blob: &[u8], passphrase: &[u8]) -> Result<[u8; 32], String> {
    if blob.len() != ENCRYPTED_LEN {
        return Err(format!(
            "expected {} encrypted bytes, got {}",
            ENCRYPTED_LEN,
            blob.len()
        ));
    }
    if &blob[..MAGIC.len()] != MAGIC {
        return Err("bad magic header (not an EVK1 encrypted key)".into());
    }
    let salt = &blob[MAGIC.len()..MAGIC.len() + SALT_LEN];
    let nonce_bytes = &blob[MAGIC.len() + SALT_LEN..MAGIC.len() + SALT_LEN + NONCE_LEN];
    let ciphertext = &blob[MAGIC.len() + SALT_LEN + NONCE_LEN..];

    let key = kdf(passphrase, salt)?;
    let cipher =
        XChaCha20Poly1305::new_from_slice(&key).map_err(|e| format!("cipher init: {e}"))?;
    let nonce = XNonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| "decrypt failed — wrong passphrase or corrupt blob".to_string())?;
    if plaintext.len() != PLAINTEXT_LEN {
        return Err(format!(
            "decrypted payload wrong length: {}",
            plaintext.len()
        ));
    }
    let mut out = [0u8; PLAINTEXT_LEN];
    out.copy_from_slice(&plaintext);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_encrypt_decrypt() {
        let secret = [7u8; 32];
        let pass = b"correct-horse-battery-staple";
        let blob = encrypt_bls_secret(&secret, pass).unwrap();
        assert_eq!(blob.len(), ENCRYPTED_LEN);
        let out = decrypt_bls_secret(&blob, pass).unwrap();
        assert_eq!(out, secret);
    }

    #[test]
    fn wrong_passphrase_rejected() {
        let secret = [3u8; 32];
        let blob = encrypt_bls_secret(&secret, b"right-pass").unwrap();
        let err = decrypt_bls_secret(&blob, b"wrong-pass").unwrap_err();
        assert!(err.contains("decrypt failed"), "unexpected error: {err}");
    }

    #[test]
    fn nonce_is_unique_per_encryption() {
        let secret = [1u8; 32];
        let pass = b"same-pass";
        let blob1 = encrypt_bls_secret(&secret, pass).unwrap();
        let blob2 = encrypt_bls_secret(&secret, pass).unwrap();
        // Different nonce + salt => different ciphertext for the same plaintext
        assert_ne!(blob1, blob2);
    }

    #[test]
    fn rejects_wrong_magic() {
        let secret = [2u8; 32];
        let mut blob = encrypt_bls_secret(&secret, b"pw").unwrap();
        blob[0] = b'X';
        let err = decrypt_bls_secret(&blob, b"pw").unwrap_err();
        assert!(err.contains("bad magic"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_wrong_length() {
        let err = decrypt_bls_secret(&[0u8; 50], b"pw").unwrap_err();
        assert!(err.contains("expected"), "unexpected error: {err}");
    }
}
