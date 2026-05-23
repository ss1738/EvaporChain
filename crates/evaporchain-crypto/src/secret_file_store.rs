//! Variable-length encrypted-at-rest format for sensitive secret blobs.
//!
//! `bls_key_store` (EVK1) handles the validator's 32-byte BLS secret with
//! a fixed-length envelope. Other secrets — TLS PEM keys, PoP-only signing
//! material, future operator credentials — are variable-length, so they
//! need an envelope that doesn't bake the plaintext length into the format.
//!
//! On-disk layout (`EVKV` = "EVaporchain Key, Variable"):
//!
//! ```text
//!   magic(4="EVKV") || salt(16) || nonce(24) || len_be(4) || ciphertext(len + 16)
//! ```
//!
//! The 4-byte big-endian `len` field is plaintext metadata so we can refuse
//! truncated blobs without a decrypt attempt. Authentication still binds
//! the length: tampering with `len` shifts ciphertext bounds and causes
//! the AEAD tag to fail.
//!
//! Primitives are identical to EVK1 (Argon2id KDF + XChaCha20-Poly1305
//! AEAD), so an operator who already trusts the bls_key_store posture
//! gets the same guarantees here.
//!
//! Closes punch-list item #3 (`docs/MAINNET_PUNCHLIST.md`).

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand::RngCore;
use zeroize::Zeroizing;

const MAGIC: &[u8; 4] = b"EVKV";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const LEN_FIELD: usize = 4;
const TAG_LEN: usize = 16;
/// Smallest possible blob: header + 0-byte plaintext + tag.
const MIN_ENVELOPE_LEN: usize = MAGIC.len() + SALT_LEN + NONCE_LEN + LEN_FIELD + TAG_LEN;
/// Cap on plaintext length, defending against malicious `len_be` values
/// that would force massive allocations on decrypt. 16 MiB is far above
/// any reasonable PEM private key (~3 KiB even for RSA-8192) and any
/// sensible operator credential.
const MAX_PLAINTEXT_LEN: usize = 16 * 1024 * 1024;

/// Returns true if `bytes` looks like an EVKV envelope. Used by callers
/// that auto-detect encrypted-vs-plaintext on read.
pub fn is_evkv(bytes: &[u8]) -> bool {
    bytes.len() >= MAGIC.len() && &bytes[..MAGIC.len()] == MAGIC
}

/// Derive a 32-byte symmetric key from a passphrase using Argon2id.
/// Same parameters as `bls_key_store::kdf` so an operator who's tuned
/// one envelope's cost has tuned both.
///
/// M-1 (audit 2026-05-17): bumped `t` from 3 to 4 to match the EVK1
/// envelope's GEN-N5 closure. Same threat model (same-host adversary
/// with disk image) → same OWASP-2026 KDF cost.
fn kdf(passphrase: &[u8], salt: &[u8]) -> Result<[u8; 32], String> {
    let params =
        Params::new(64 * 1024, 4, 1, Some(32)).map_err(|e| format!("argon2 params: {e}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    argon
        .hash_password_into(passphrase, salt, &mut out)
        .map_err(|e| format!("argon2 derive: {e}"))?;
    Ok(out)
}

/// Encrypt `plaintext` with `passphrase`, returning the EVKV envelope.
/// Plaintext may be any length up to `MAX_PLAINTEXT_LEN` (16 MiB).
pub fn encrypt_blob(plaintext: &[u8], passphrase: &[u8]) -> Result<Vec<u8>, String> {
    if plaintext.len() > MAX_PLAINTEXT_LEN {
        return Err(format!(
            "plaintext too large: {} bytes (limit {})",
            plaintext.len(),
            MAX_PLAINTEXT_LEN
        ));
    }
    if passphrase.is_empty() {
        return Err("empty passphrase rejected".into());
    }

    let mut salt = [0u8; SALT_LEN];
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);

    // M-2 (audit 2026-05-17): wrap KDF-derived key in Zeroizing so the
    // 32-byte material is overwritten on drop. Matches the CLI-KDF-001
    // closure already applied to EVK1.
    let key = Zeroizing::new(kdf(passphrase, &salt)?);
    let cipher = XChaCha20Poly1305::new_from_slice(key.as_ref())
        .map_err(|e| format!("cipher init: {e}"))?;
    let nonce = XNonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| format!("encrypt: {e}"))?;

    let mut out = Vec::with_capacity(MIN_ENVELOPE_LEN + plaintext.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&(plaintext.len() as u32).to_be_bytes());
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Decrypt an EVKV envelope with `passphrase`. Returns the plaintext.
pub fn decrypt_blob(blob: &[u8], passphrase: &[u8]) -> Result<Vec<u8>, String> {
    if blob.len() < MIN_ENVELOPE_LEN {
        return Err(format!(
            "blob too short for EVKV envelope: {} < {}",
            blob.len(),
            MIN_ENVELOPE_LEN
        ));
    }
    if &blob[..MAGIC.len()] != MAGIC {
        return Err("bad magic header (not an EVKV encrypted blob)".into());
    }

    let mut cursor = MAGIC.len();
    let salt = &blob[cursor..cursor + SALT_LEN];
    cursor += SALT_LEN;
    let nonce_bytes = &blob[cursor..cursor + NONCE_LEN];
    cursor += NONCE_LEN;
    let len_bytes: [u8; 4] = blob[cursor..cursor + LEN_FIELD]
        .try_into()
        .map_err(|_| "len field unreadable".to_string())?;
    cursor += LEN_FIELD;
    let claimed_len = u32::from_be_bytes(len_bytes) as usize;

    if claimed_len > MAX_PLAINTEXT_LEN {
        return Err(format!(
            "claimed plaintext length {} exceeds cap {}",
            claimed_len, MAX_PLAINTEXT_LEN
        ));
    }

    let expected_ct_len = claimed_len + TAG_LEN;
    if blob.len() - cursor != expected_ct_len {
        return Err(format!(
            "ciphertext length mismatch: got {}, expected {}",
            blob.len() - cursor,
            expected_ct_len
        ));
    }
    let ciphertext = &blob[cursor..];

    let key = Zeroizing::new(kdf(passphrase, salt)?);
    let cipher = XChaCha20Poly1305::new_from_slice(key.as_ref())
        .map_err(|e| format!("cipher init: {e}"))?;
    let nonce = XNonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| "decrypt failed — wrong passphrase or corrupt blob".to_string())?;

    if plaintext.len() != claimed_len {
        return Err(format!(
            "decrypted payload length mismatch: got {}, claimed {}",
            plaintext.len(),
            claimed_len
        ));
    }
    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_fixed_size() {
        let plaintext = b"hello evaporchain";
        let pass = b"correct-horse-battery-staple";
        let blob = encrypt_blob(plaintext, pass).unwrap();
        assert!(is_evkv(&blob));
        let out = decrypt_blob(&blob, pass).unwrap();
        assert_eq!(&out, plaintext);
    }

    #[test]
    fn roundtrip_pem_sized() {
        // Realistic ECDSA-P256 PEM body — ~240 bytes.
        let pem = b"-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgABCDEFGHIJKLMNOP\n-----END PRIVATE KEY-----\n";
        let pass = b"validator-tls-pass";
        let blob = encrypt_blob(pem, pass).unwrap();
        let out = decrypt_blob(&blob, pass).unwrap();
        assert_eq!(&out, pem);
    }

    #[test]
    fn roundtrip_empty_payload() {
        let blob = encrypt_blob(b"", b"pw").unwrap();
        let out = decrypt_blob(&blob, b"pw").unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn wrong_passphrase_rejected() {
        let blob = encrypt_blob(b"secret", b"right-pass").unwrap();
        let err = decrypt_blob(&blob, b"wrong-pass").unwrap_err();
        assert!(err.contains("decrypt failed"), "unexpected error: {err}");
    }

    #[test]
    fn nonce_unique_across_encryptions() {
        let pass = b"same";
        let blob1 = encrypt_blob(b"x", pass).unwrap();
        let blob2 = encrypt_blob(b"x", pass).unwrap();
        // Salt + nonce both random => identical plaintexts produce
        // distinct ciphertexts.
        assert_ne!(blob1, blob2);
    }

    #[test]
    fn rejects_wrong_magic() {
        let mut blob = encrypt_blob(b"x", b"pw").unwrap();
        blob[0] = b'Z';
        let err = decrypt_blob(&blob, b"pw").unwrap_err();
        assert!(err.contains("bad magic"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_truncated_envelope() {
        let blob = encrypt_blob(b"hello", b"pw").unwrap();
        let truncated = &blob[..blob.len() - 1];
        let err = decrypt_blob(truncated, b"pw").unwrap_err();
        assert!(err.contains("ciphertext length"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_oversized_len_field() {
        let mut blob = encrypt_blob(b"hello", b"pw").unwrap();
        // Stomp the len field with a huge value.
        let len_offset = MAGIC.len() + SALT_LEN + NONCE_LEN;
        let bogus = (MAX_PLAINTEXT_LEN as u32 + 1).to_be_bytes();
        blob[len_offset..len_offset + LEN_FIELD].copy_from_slice(&bogus);
        let err = decrypt_blob(&blob, b"pw").unwrap_err();
        assert!(err.contains("exceeds cap"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_empty_passphrase() {
        let err = encrypt_blob(b"x", b"").unwrap_err();
        assert!(err.contains("empty passphrase"), "unexpected error: {err}");
    }

    #[test]
    fn is_evkv_only_matches_real_envelope() {
        let blob = encrypt_blob(b"x", b"pw").unwrap();
        assert!(is_evkv(&blob));
        // Plaintext PEM should not look like an envelope.
        assert!(!is_evkv(b"-----BEGIN PRIVATE KEY-----"));
        // Empty input is too short to match.
        assert!(!is_evkv(b""));
    }

    #[test]
    fn tag_failure_on_tampered_len() {
        // Even though the len field is plaintext, modifying it shifts
        // ciphertext boundaries; the AEAD tag will fail authentication.
        let mut blob = encrypt_blob(b"some payload", b"pw").unwrap();
        let len_offset = MAGIC.len() + SALT_LEN + NONCE_LEN;
        // Tamper to a slightly smaller len that's still plausible.
        blob[len_offset + 3] = blob[len_offset + 3].wrapping_sub(1);
        let err = decrypt_blob(&blob, b"pw").unwrap_err();
        // Either the length-mismatch check or the AEAD tag check fires —
        // both are acceptable rejections.
        assert!(
            err.contains("length") || err.contains("decrypt failed"),
            "unexpected error: {err}"
        );
    }

    /// T1.20 — adversarial: tampering with the SALT bytes changes
    /// the Argon2id-derived key, so even with the correct passphrase
    /// decrypt fails. Pins that the salt is integrity-critical: an
    /// attacker who flips a salt byte can't trick the decryptor
    /// into using a known key.
    #[test]
    fn t1_20_decrypt_rejects_tampered_salt() {
        let mut blob = encrypt_blob(b"sensitive payload", b"pw").unwrap();
        // Salt lives at [4..20] (after MAGIC).
        let salt_offset = MAGIC.len();
        blob[salt_offset] ^= 0xFF;
        let err = decrypt_blob(&blob, b"pw").unwrap_err();
        assert!(
            err.contains("decrypt failed"),
            "salt tampering must yield decrypt-failed (the wrong-KDF-key path); got: {err}"
        );
    }

    /// T1.20 — adversarial: flipping a byte inside the ciphertext
    /// (post-header) trips the AEAD tag check. Pins ciphertext
    /// integrity — distinct from len-field tampering (which can be
    /// caught at the length-mismatch gate before AEAD runs).
    #[test]
    fn t1_20_decrypt_rejects_tampered_ciphertext_byte() {
        let mut blob = encrypt_blob(b"ciphertext-integrity-test", b"pw").unwrap();
        // Pick a byte in the ciphertext body — after MAGIC+SALT+NONCE+LEN.
        let ct_offset = MAGIC.len() + SALT_LEN + NONCE_LEN + LEN_FIELD;
        blob[ct_offset] ^= 0x01;
        let err = decrypt_blob(&blob, b"pw").unwrap_err();
        assert!(
            err.contains("decrypt failed"),
            "ciphertext byte flip must trip AEAD tag; got: {err}"
        );
    }

    /// T1.20 — `is_evkv` boundary: inputs shorter than the 4-byte
    /// magic return false without panicking on the slice. Existing
    /// `is_evkv_only_matches_real_envelope` covers empty input;
    /// this pins the 1/2/3-byte intermediate boundary explicitly.
    #[test]
    fn t1_20_is_evkv_short_input_returns_false() {
        assert!(!is_evkv(b""));
        assert!(!is_evkv(b"E"));
        assert!(!is_evkv(b"EV"));
        assert!(!is_evkv(b"EVK"));
        // Exactly 4 bytes but wrong: still false.
        assert!(!is_evkv(b"EVKW"));
        // Exactly 4 bytes correct: true.
        assert!(is_evkv(b"EVKV"));
    }

    /// T1.20 — large-payload round-trip (64 KiB). The existing
    /// happy-path tests use <300-byte payloads; this pins that the
    /// envelope handles realistic operator-credential sizes (large
    /// PEM bundles, multi-key JSON) cleanly. Well under
    /// MAX_PLAINTEXT_LEN (16 MiB), still exercises non-trivial
    /// AEAD throughput.
    #[test]
    fn t1_20_large_payload_round_trips() {
        let big: Vec<u8> = (0..65_536u32).map(|i| (i & 0xFF) as u8).collect();
        let pass = b"operator-credential-pass";
        let blob = encrypt_blob(&big, pass).unwrap();
        let out = decrypt_blob(&blob, pass).unwrap();
        assert_eq!(out.len(), big.len());
        assert_eq!(out, big);
    }
}
