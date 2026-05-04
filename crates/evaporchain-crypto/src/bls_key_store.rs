//! Optional encrypted-at-rest format for the validator BLS secret key.
//!
//! Two on-disk shapes are supported and auto-detected by length:
//! - **plaintext (32 bytes):** the historical raw `BlsSecretKey` bytes,
//!   protected only by file mode 0600.
//! - **encrypted (92 bytes):** Argon2id-derived key + XChaCha20-Poly1305
//!   AEAD over the 32-byte secret. Layout:
//!   `magic(4="EVK1") || salt(16) || nonce(24) || ciphertext(32+16=48)`.
//!
//! Encryption is opt-in for the running node binary: an operator enables
//! it by setting the `EVAPORCHAIN_VALIDATOR_KEY_PASS` environment
//! variable. When set, new keys are written encrypted and existing
//! encrypted keys can be decrypted with the same passphrase. When unset,
//! the historical plaintext path is used and a warning is logged.
//!
//! Operators with an existing plaintext `bls_key.bin` can migrate to
//! the encrypted format with the `evaporchain encrypt-bls-key` CLI
//! subcommand instead of regenerating the validator identity.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand::RngCore;

const MAGIC: &[u8; 4] = b"EVK1";
/// **Audit fix HIGH (crypto)**: EVK2 envelope adds a 1-byte
/// `version` / `algorithm` discriminator immediately after the magic.
/// Layout: `magic("EVK2") || version(1) || salt(16) || nonce(24) ||
/// ciphertext(48)` = 93 bytes. Reserves headroom for future
/// migrations (different KDF, different AEAD, hardware-backed keys)
/// without breaking compat — the version byte selects the algorithm.
///
/// New writes go to EVK2 v1 (Argon2id + XChaCha20-Poly1305, same as
/// EVK1). EVK1 reads remain supported indefinitely so operators can
/// migrate at their own pace via key rotation.
const MAGIC_V2: &[u8; 4] = b"EVK2";
const ALG_VERSION_ARGON2ID_XCHACHA20: u8 = 1;
/// Crypto-6 (re-audit 2026-05-02): magic header for the new
/// plaintext-with-magic format. Closes the audit's "32-byte
/// ciphertext fragment can be silently misclassified as plaintext"
/// concern. Layout: `b"EVPL" || 32 raw bytes` = 36 bytes total.
const PLAINTEXT_MAGIC: &[u8; 4] = b"EVPL";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const PLAINTEXT_LEN: usize = 32;
const TAG_LEN: usize = 16;
pub const ENCRYPTED_LEN: usize = MAGIC.len() + SALT_LEN + NONCE_LEN + PLAINTEXT_LEN + TAG_LEN; // 92
pub const ENCRYPTED_LEN_V2: usize =
    MAGIC_V2.len() + 1 + SALT_LEN + NONCE_LEN + PLAINTEXT_LEN + TAG_LEN; // 93
pub const PLAINTEXT_MAGIC_LEN: usize = PLAINTEXT_MAGIC.len() + PLAINTEXT_LEN; // 36

pub const ENV_PASSPHRASE: &str = "EVAPORCHAIN_VALIDATOR_KEY_PASS";
/// Path-to-passphrase env var. Preferred over `ENV_PASSPHRASE` because
/// the env value of `ENV_PASSPHRASE` is visible to any process owned by
/// the same user via `/proc/<pid>/environ` on Linux. With this set we
/// read the file (mode-0600 expected), trim the trailing newline, and
/// optionally clear the env var the caller put it in.
pub const ENV_PASSPHRASE_FILE: &str = "EVAPORCHAIN_VALIDATOR_KEY_PASS_FILE";

/// Returns the operator-supplied passphrase. Tries (in order):
///   1. `EVAPORCHAIN_VALIDATOR_KEY_PASS_FILE=<path>` — read the file
///      contents (recommended for production; the on-disk file is
///      protected by mode 0600, not visible in `ps -E` / `/proc/.../environ`).
///   2. `EVAPORCHAIN_VALIDATOR_KEY_PASS=<value>` — direct env var
///      (legacy; visible to other same-user processes on Linux).
///
/// Returns None when neither is set; caller decides plaintext fallback.
/// Re-audit (2026-05-02): added the `_FILE` variant to close the
/// passphrase-via-env exposure surface.
pub fn passphrase_from_env() -> Option<Vec<u8>> {
    if let Ok(path) = std::env::var(ENV_PASSPHRASE_FILE) {
        if !path.is_empty() {
            match std::fs::read(&path) {
                Ok(mut bytes) => {
                    // Trim a single trailing \n or \r\n — common for
                    // `echo > pass.txt` style files.
                    if bytes.last() == Some(&b'\n') {
                        bytes.pop();
                    }
                    if bytes.last() == Some(&b'\r') {
                        bytes.pop();
                    }
                    if !bytes.is_empty() {
                        return Some(bytes);
                    }
                }
                Err(e) => {
                    eprintln!(
                        "\x1b[33m⚠ {}={} read failed: {}; falling back to {}\x1b[0m",
                        ENV_PASSPHRASE_FILE, path, e, ENV_PASSPHRASE
                    );
                }
            }
        }
    }
    match std::env::var(ENV_PASSPHRASE) {
        Ok(p) if !p.is_empty() => Some(p.into_bytes()),
        _ => None,
    }
}

/// Argon2id parameters — pinned to OWASP 2024 baseline. If a future
/// argon2 crate release silently weakens its defaults, these
/// per-parameter constants make the regression a compile-time / code-
/// review event instead of a quiet operational downgrade.
/// Re-audit (2026-05-02): explicit pinning per audit Crypto-8.
pub const ARGON2_M_COST_KIB: u32 = 64 * 1024; // 64 MiB
pub const ARGON2_T_COST: u32 = 3;
pub const ARGON2_P_COST: u32 = 1;
pub const ARGON2_OUT_LEN: usize = 32;

/// Derive a 32-byte symmetric key from a passphrase using Argon2id.
fn kdf(passphrase: &[u8], salt: &[u8]) -> Result<[u8; 32], String> {
    let params = Params::new(
        ARGON2_M_COST_KIB,
        ARGON2_T_COST,
        ARGON2_P_COST,
        Some(ARGON2_OUT_LEN),
    )
    .map_err(|e| format!("argon2 params: {e}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    argon
        .hash_password_into(passphrase, salt, &mut out)
        .map_err(|e| format!("argon2 derive: {e}"))?;
    Ok(out)
}

/// Encrypt a 32-byte BLS secret with a passphrase. Returns 92 bytes.
///
/// Equivalent to `encrypt_bls_secret_with_aad(secret, passphrase, &[])`.
/// Prefer the `_with_aad` form and pass `path_aad(file_path)` as the AAD
/// so a ciphertext can't be silently relocated to a different file path
/// (H5 — audit 2026-05-02). Old EVK1 blobs without AAD remain decodable.
pub fn encrypt_bls_secret(secret: &[u8], passphrase: &[u8]) -> Result<Vec<u8>, String> {
    encrypt_bls_secret_with_aad(secret, passphrase, &[])
}

/// Encrypt a 32-byte BLS secret with a passphrase and bind it to AAD.
///
/// H5 (audit 2026-05-02): pass `path_aad(file_path)` for production
/// writes. The AAD is authenticated by XChaCha20-Poly1305 — any
/// attempt to decrypt the same ciphertext under a different AAD
/// (i.e., a swapped file path) yields an authentication failure.
pub fn encrypt_bls_secret_with_aad(
    secret: &[u8],
    passphrase: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, String> {
    use chacha20poly1305::aead::Payload;
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
        .encrypt(nonce, Payload { msg: secret, aad })
        .map_err(|e| format!("encrypt: {e}"))?;

    // **Audit fix HIGH (crypto)**: write the new EVK2 envelope with a
    // 1-byte version discriminator. Operators reading old EVK1 blobs
    // continue to work via the legacy decrypt path; new writes go to
    // EVK2 only. Reserves migration headroom.
    let mut out = Vec::with_capacity(ENCRYPTED_LEN_V2);
    out.extend_from_slice(MAGIC_V2);
    out.push(ALG_VERSION_ARGON2ID_XCHACHA20);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    debug_assert_eq!(out.len(), ENCRYPTED_LEN_V2);
    Ok(out)
}

/// Decrypt a 92-byte encrypted blob with the supplied passphrase.
pub fn decrypt_bls_secret(blob: &[u8], passphrase: &[u8]) -> Result<[u8; 32], String> {
    decrypt_bls_secret_with_aad(blob, passphrase, &[])
}

/// Decrypt with explicit AAD. Use the same `aad` value that was passed
/// to `encrypt_bls_secret_with_aad` (e.g. `path_aad(file_path)`) — a
/// mismatch yields a generic `decrypt failed` error indistinguishable
/// from a wrong passphrase, by design.
pub fn decrypt_bls_secret_with_aad(
    blob: &[u8],
    passphrase: &[u8],
    aad: &[u8],
) -> Result<[u8; 32], String> {
    use chacha20poly1305::aead::Payload;

    // Detect EVK1 (legacy) vs EVK2 (current) by length + magic.
    let (salt, nonce_bytes, ciphertext) = if blob.len() == ENCRYPTED_LEN_V2
        && blob.len() >= MAGIC_V2.len()
        && &blob[..MAGIC_V2.len()] == MAGIC_V2
    {
        // EVK2: magic(4) | version(1) | salt(16) | nonce(24) | ciphertext(48)
        let version = blob[MAGIC_V2.len()];
        if version != ALG_VERSION_ARGON2ID_XCHACHA20 {
            return Err(format!(
                "unsupported EVK2 algorithm version: {version} (this build supports {})",
                ALG_VERSION_ARGON2ID_XCHACHA20
            ));
        }
        let header = MAGIC_V2.len() + 1;
        let salt = &blob[header..header + SALT_LEN];
        let nonce_bytes = &blob[header + SALT_LEN..header + SALT_LEN + NONCE_LEN];
        let ciphertext = &blob[header + SALT_LEN + NONCE_LEN..];
        (salt, nonce_bytes, ciphertext)
    } else if blob.len() == ENCRYPTED_LEN
        && blob.len() >= MAGIC.len()
        && &blob[..MAGIC.len()] == MAGIC
    {
        // EVK1 (legacy): magic(4) | salt(16) | nonce(24) | ciphertext(48).
        // Read-only — new writes go to EVK2.
        let salt = &blob[MAGIC.len()..MAGIC.len() + SALT_LEN];
        let nonce_bytes = &blob[MAGIC.len() + SALT_LEN..MAGIC.len() + SALT_LEN + NONCE_LEN];
        let ciphertext = &blob[MAGIC.len() + SALT_LEN + NONCE_LEN..];
        (salt, nonce_bytes, ciphertext)
    } else {
        return Err(format!(
            "expected EVK1 ({} bytes) or EVK2 ({} bytes) encrypted blob, got {} bytes",
            ENCRYPTED_LEN,
            ENCRYPTED_LEN_V2,
            blob.len()
        ));
    };

    let key = kdf(passphrase, salt)?;
    let cipher =
        XChaCha20Poly1305::new_from_slice(&key).map_err(|e| format!("cipher init: {e}"))?;
    let nonce = XNonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, Payload { msg: ciphertext, aad })
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

/// Produce a deterministic AAD bytes from a canonical file path.
/// Use the `Path::canonicalize()` result (or `path.to_string_lossy()`
/// on Linux) so the AAD is stable across symlinks and relative-vs-
/// absolute path expressions.
pub fn path_aad(path_bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(path_bytes).as_bytes()
}

// ─────────────────────── Plaintext-with-magic format ─────────────────────

/// Wrap a 32-byte plaintext BLS secret with the `EVPL` magic header.
/// Output is 36 bytes: `b"EVPL" || secret`. New writes use this
/// format; readers accept both new and legacy raw-32-byte forms.
pub fn format_plaintext_for_disk(secret: &[u8]) -> Result<Vec<u8>, String> {
    if secret.len() != PLAINTEXT_LEN {
        return Err(format!(
            "BLS secret must be {} bytes, got {}",
            PLAINTEXT_LEN,
            secret.len()
        ));
    }
    let mut out = Vec::with_capacity(PLAINTEXT_MAGIC_LEN);
    out.extend_from_slice(PLAINTEXT_MAGIC);
    out.extend_from_slice(secret);
    Ok(out)
}

/// On-disk format detected from a BLS-key file's bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlsKeyFormat {
    /// 36 bytes: `b"EVPL" || 32 raw secret bytes`.
    PlaintextMagic,
    /// 92 bytes: `b"EVK1" || salt || nonce || ciphertext` (legacy).
    Encrypted,
    /// 93 bytes: `b"EVK2" || version || salt || nonce || ciphertext`.
    /// **Audit fix HIGH (crypto)**: current encrypted format with a
    /// 1-byte algorithm/version discriminator.
    EncryptedV2,
    /// 32 bytes raw secret (no header). Legacy — accepted with a
    /// deprecation warning by `extract_plaintext`.
    LegacyRaw,
    /// Anything else.
    Unknown,
}

/// Classify a BLS-key file's bytes by magic header + length. O(1) and
/// allocation-free; safe to call on attacker-controlled input.
pub fn detect_bls_key_format(bytes: &[u8]) -> BlsKeyFormat {
    if bytes.len() == PLAINTEXT_MAGIC_LEN && &bytes[..PLAINTEXT_MAGIC.len()] == PLAINTEXT_MAGIC {
        BlsKeyFormat::PlaintextMagic
    } else if bytes.len() == ENCRYPTED_LEN_V2
        && bytes.len() >= MAGIC_V2.len()
        && &bytes[..MAGIC_V2.len()] == MAGIC_V2
    {
        BlsKeyFormat::EncryptedV2
    } else if bytes.len() == ENCRYPTED_LEN && &bytes[..MAGIC.len()] == MAGIC {
        BlsKeyFormat::Encrypted
    } else if bytes.len() == PLAINTEXT_LEN {
        BlsKeyFormat::LegacyRaw
    } else {
        BlsKeyFormat::Unknown
    }
}

/// Extract the 32 raw secret bytes from a `PlaintextMagic` or
/// `LegacyRaw` blob. Returns `Err` for any other format. Caller
/// should already have called `detect_bls_key_format` and chosen
/// this path only when the plaintext branch matched.
pub fn extract_plaintext(bytes: &[u8]) -> Result<[u8; PLAINTEXT_LEN], String> {
    match detect_bls_key_format(bytes) {
        BlsKeyFormat::PlaintextMagic => {
            let mut out = [0u8; PLAINTEXT_LEN];
            out.copy_from_slice(&bytes[PLAINTEXT_MAGIC.len()..]);
            Ok(out)
        }
        BlsKeyFormat::LegacyRaw => {
            let mut out = [0u8; PLAINTEXT_LEN];
            out.copy_from_slice(bytes);
            Ok(out)
        }
        BlsKeyFormat::Encrypted | BlsKeyFormat::EncryptedV2 => Err(
            "blob is encrypted — call decrypt_bls_secret_with_aad instead".into(),
        ),
        BlsKeyFormat::Unknown => Err(format!(
            "unrecognised BLS key format ({} bytes; expected 32 raw, 36 EVPL+raw, 92 EVK1, or 93 EVK2)",
            bytes.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_encrypt_decrypt() {
        let secret = [7u8; 32];
        let pass = b"correct-horse-battery-staple";
        let blob = encrypt_bls_secret(&secret, pass).unwrap();
        // **Audit fix HIGH (crypto)**: new writes use EVK2 (93 bytes).
        assert_eq!(blob.len(), ENCRYPTED_LEN_V2);
        assert_eq!(&blob[..MAGIC_V2.len()], MAGIC_V2);
        assert_eq!(blob[MAGIC_V2.len()], ALG_VERSION_ARGON2ID_XCHACHA20);
        let out = decrypt_bls_secret(&blob, pass).unwrap();
        assert_eq!(out, secret);
    }

    /// Legacy EVK1 blobs must remain decryptable so operators don't
    /// need a forced migration. Hand-build an EVK1 blob via the old
    /// layout and confirm `decrypt_bls_secret` accepts it.
    #[test]
    fn legacy_evk1_blob_still_decrypts() {
        let secret = [9u8; 32];
        let pass = b"legacy-pass";
        // Replicate the EVK1 path (no version byte) by hand:
        let mut salt = [0u8; SALT_LEN];
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut salt);
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let key = kdf(pass, &salt).unwrap();
        let cipher = XChaCha20Poly1305::new_from_slice(&key).unwrap();
        let nonce = XNonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(
                nonce,
                chacha20poly1305::aead::Payload {
                    msg: &secret,
                    aad: &[],
                },
            )
            .unwrap();
        let mut blob = Vec::with_capacity(ENCRYPTED_LEN);
        blob.extend_from_slice(MAGIC);
        blob.extend_from_slice(&salt);
        blob.extend_from_slice(&nonce_bytes);
        blob.extend_from_slice(&ciphertext);
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
        // Corrupted EVK2 magic falls through to "expected EVK1 or EVK2"
        // shape error; the magic check is now embedded in the format
        // dispatcher rather than a separate `bad magic` line.
        let err = decrypt_bls_secret(&blob, b"pw").unwrap_err();
        assert!(
            err.contains("expected") || err.contains("magic"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_wrong_length() {
        let err = decrypt_bls_secret(&[0u8; 50], b"pw").unwrap_err();
        assert!(err.contains("expected"), "unexpected error: {err}");
    }
}
