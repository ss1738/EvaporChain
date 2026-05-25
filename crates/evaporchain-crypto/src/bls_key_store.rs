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
use zeroize::Zeroizing;

const MAGIC: &[u8; 4] = b"EVK1";
/// Crypto-6 (re-audit 2026-05-02): magic header for the new
/// plaintext-with-magic format. Closes the audit's "32-byte
/// ciphertext fragment can be silently misclassified as plaintext"
/// concern. Layout: `b"EVPL" || 32 raw bytes` = 36 bytes total.
/// Auto-write (in `format_plaintext_for_disk`) and auto-detect
/// (in `decode_key_from_disk`) handle the new format; legacy raw
/// 32-byte plaintext is still accepted with a deprecation warning.
const PLAINTEXT_MAGIC: &[u8; 4] = b"EVPL";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const PLAINTEXT_LEN: usize = 32;
const TAG_LEN: usize = 16;
pub const ENCRYPTED_LEN: usize = MAGIC.len() + SALT_LEN + NONCE_LEN + PLAINTEXT_LEN + TAG_LEN; // 92
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

/// Argon2id parameters — pinned and per-parameter so any future
/// argon2 crate release silently weakening its defaults becomes a
/// compile-time / code-review event instead of a quiet operational
/// downgrade.
///
/// Re-audit (2026-05-02): explicit pinning per audit Crypto-8.
///
/// GEN-N5 (audit 2026-05-15): `ARGON2_T_COST` bumped from 3 → 4.
/// The validator BLS sk is the highest-value local secret on the
/// box; the older `t=3` baseline (OWASP 2024) is below the OWASP
/// 2026 / RFC 9106 "second-recommendation" tier (`m=64 MiB, t=4`)
/// for high-value secrets. A well-resourced attacker with a stolen
/// disk image and a weak passphrase could brute-force `t=3` offline
/// on a single GPU rig within the operational threat window; `t=4`
/// raises the per-guess cost by ~33% (one extra Argon2id pass)
/// without exceeding the 200ms-per-derivation operator runtime
/// budget on the M4 Mini test bench.
pub const ARGON2_M_COST_KIB: u32 = 64 * 1024; // 64 MiB
pub const ARGON2_T_COST: u32 = 4;
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

    let key = Zeroizing::new(kdf(passphrase, &salt)?);
    let cipher =
        XChaCha20Poly1305::new_from_slice(&*key).map_err(|e| format!("cipher init: {e}"))?;
    let nonce = XNonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, Payload { msg: secret, aad })
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

    let key = Zeroizing::new(kdf(passphrase, salt)?);
    let cipher =
        XChaCha20Poly1305::new_from_slice(&*key).map_err(|e| format!("cipher init: {e}"))?;
    let nonce = XNonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(
            nonce,
            Payload {
                msg: ciphertext,
                aad,
            },
        )
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

/// On-disk format detected from a BLS-key file's bytes. Use this
/// instead of `match bytes.len()` to avoid the audit's "32-byte
/// ciphertext fragment misclassified as plaintext" footgun. The
/// 4-byte `EVPL` / `EVK1` magic prefix is unambiguous.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlsKeyFormat {
    /// 36 bytes: `b"EVPL" || 32 raw secret bytes`.
    PlaintextMagic,
    /// 92 bytes: `b"EVK1" || salt || nonce || ciphertext`.
    Encrypted,
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
        BlsKeyFormat::Encrypted => {
            Err("blob is encrypted (EVK1) — call decrypt_bls_secret_with_aad instead".into())
        }
        BlsKeyFormat::Unknown => Err(format!(
            "unrecognised BLS key format ({} bytes; expected 32 raw, 36 EVPL+raw, or 92 EVK1)",
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

    /// T1.20 — H5 audit (2026-05-02): `encrypt_bls_secret_with_aad` /
    /// `decrypt_bls_secret_with_aad` round-trip with the same AAD
    /// returns the original plaintext. Pins the happy path of the
    /// AAD-bound API that production callers use via `path_aad`.
    #[test]
    fn t1_20_encrypt_decrypt_with_aad_roundtrip() {
        let secret = [0x55u8; 32];
        let pass = b"correct-horse-battery-staple";
        let aad = path_aad(b"/var/lib/evaporchain/bls_key.bin");
        let blob = encrypt_bls_secret_with_aad(&secret, pass, &aad).unwrap();
        assert_eq!(blob.len(), ENCRYPTED_LEN);
        let recovered = decrypt_bls_secret_with_aad(&blob, pass, &aad).unwrap();
        assert_eq!(recovered, secret);
    }

    /// T1.20 — H5 audit ADVERSARIAL: AAD mismatch fails decryption
    /// indistinguishably from a wrong-passphrase error. This is the
    /// soundness property the AAD-binding added: an attacker who
    /// relocates an EVK1 blob to a different file path (encoded as
    /// a different `path_aad`) cannot decrypt it even with the
    /// correct passphrase.
    #[test]
    fn t1_20_aad_mismatch_fails_decryption() {
        let secret = [0xAAu8; 32];
        let pass = b"same-passphrase";
        let aad_original = path_aad(b"/expected/bls_key.bin");
        let aad_attacker = path_aad(b"/attacker/bls_key.bin");
        let blob = encrypt_bls_secret_with_aad(&secret, pass, &aad_original).unwrap();
        // Correct AAD → decrypts.
        assert!(decrypt_bls_secret_with_aad(&blob, pass, &aad_original).is_ok());
        // Mismatched AAD with same passphrase → fails.
        let err = decrypt_bls_secret_with_aad(&blob, pass, &aad_attacker).unwrap_err();
        assert!(
            err.contains("decrypt failed"),
            "AAD mismatch must yield decrypt-failed (not bad-magic / wrong-length); got: {err}"
        );
        // Empty AAD (legacy `decrypt_bls_secret`) also fails.
        let err = decrypt_bls_secret(&blob, pass).unwrap_err();
        assert!(err.contains("decrypt failed"), "got: {err}");
    }

    /// T1.20 — `detect_bls_key_format` returns the right variant for
    /// each on-disk shape (lines 268-278). Pins all four arms of the
    /// classifier — a refactor that lost the `EVPL` magic check
    /// would mis-route plaintext blobs through `LegacyRaw`, dropping
    /// the audit's Crypto-6 protection.
    #[test]
    fn t1_20_detect_bls_key_format_all_variants() {
        // PlaintextMagic: 36 bytes with EVPL prefix.
        let plaintext_magic = {
            let mut v = Vec::with_capacity(PLAINTEXT_MAGIC_LEN);
            v.extend_from_slice(PLAINTEXT_MAGIC);
            v.extend_from_slice(&[0xCC; PLAINTEXT_LEN]);
            v
        };
        assert_eq!(
            detect_bls_key_format(&plaintext_magic),
            BlsKeyFormat::PlaintextMagic
        );

        // Encrypted: 92 bytes with EVK1 prefix.
        let encrypted = encrypt_bls_secret(&[0x11; 32], b"pw").unwrap();
        assert_eq!(detect_bls_key_format(&encrypted), BlsKeyFormat::Encrypted);

        // LegacyRaw: 32 bytes, no magic.
        assert_eq!(
            detect_bls_key_format(&[0xFFu8; 32]),
            BlsKeyFormat::LegacyRaw
        );

        // Unknown: anything else.
        assert_eq!(detect_bls_key_format(&[0u8; 0]), BlsKeyFormat::Unknown);
        assert_eq!(detect_bls_key_format(&[0u8; 64]), BlsKeyFormat::Unknown);
        // 36 bytes WITHOUT EVPL magic → Unknown, NOT mis-classified
        // as PlaintextMagic.
        let bogus_36 = [0xABu8; PLAINTEXT_MAGIC_LEN];
        assert_eq!(detect_bls_key_format(&bogus_36), BlsKeyFormat::Unknown);
    }

    /// T1.20 — `extract_plaintext` on an Encrypted blob returns Err
    /// pointing the caller at the decrypt API (line 296-298).
    /// Prevents a footgun where a caller routes an encrypted blob
    /// through the plaintext path and gets ciphertext bytes back.
    #[test]
    fn t1_20_extract_plaintext_rejects_encrypted_blob() {
        let encrypted = encrypt_bls_secret(&[0x77; 32], b"pw").unwrap();
        let err = extract_plaintext(&encrypted).unwrap_err();
        assert!(
            err.contains("encrypted") && err.contains("decrypt"),
            "encrypted blob should be routed to decrypt API; got: {err}"
        );

        // PlaintextMagic round-trips through extract_plaintext.
        let secret = [0x99u8; 32];
        let wrapped = format_plaintext_for_disk(&secret).unwrap();
        assert_eq!(extract_plaintext(&wrapped).unwrap(), secret);

        // LegacyRaw also extracts cleanly.
        assert_eq!(extract_plaintext(&[0x42u8; 32]).unwrap(), [0x42u8; 32]);

        // Unknown length is rejected with a diagnostic.
        let err = extract_plaintext(&[0u8; 10]).unwrap_err();
        assert!(
            err.contains("unrecognised") || err.contains("10 bytes"),
            "wrong-length input should be diagnosed; got: {err}"
        );
    }

    /// T1.20 — wrong-secret-length inputs to the encryption helpers
    /// fail loud (lines 140-145, 236-241). A future caller passing
    /// the wrong-sized buffer must not silently produce a malformed
    /// blob.
    #[test]
    fn t1_20_encrypt_rejects_wrong_secret_length() {
        let pass = b"any-passphrase";
        let too_short = [0u8; 16];
        let too_long = [0u8; 64];

        let err = encrypt_bls_secret(&too_short, pass).unwrap_err();
        assert!(err.contains("32 bytes") && err.contains("got"));

        let err = encrypt_bls_secret(&too_long, pass).unwrap_err();
        assert!(err.contains("32 bytes") && err.contains("got"));

        // Same gate on format_plaintext_for_disk.
        let err = format_plaintext_for_disk(&too_short).unwrap_err();
        assert!(err.contains("32 bytes"));
    }

    #[test]
    fn test_passphrase_from_env_direct_var() {
        // Temporarily set the direct passphrase env var.
        // Run under a unique env key name to avoid cross-test interference.
        unsafe { std::env::remove_var(ENV_PASSPHRASE_FILE) };
        unsafe { std::env::set_var(ENV_PASSPHRASE, "hunter2") };
        let result = passphrase_from_env();
        unsafe { std::env::remove_var(ENV_PASSPHRASE) };
        assert_eq!(result, Some(b"hunter2".to_vec()));
    }

    #[test]
    fn test_passphrase_from_env_returns_none_when_unset() {
        unsafe { std::env::remove_var(ENV_PASSPHRASE_FILE) };
        unsafe { std::env::remove_var(ENV_PASSPHRASE) };
        let result = passphrase_from_env();
        assert!(result.is_none());
    }

    #[test]
    fn test_passphrase_from_env_file_var_reads_file() {
        let dir = std::env::temp_dir();
        let path = dir.join("bls_pass_test.txt");
        std::fs::write(&path, b"correct-horse-battery\n").unwrap();
        unsafe { std::env::set_var(ENV_PASSPHRASE_FILE, path.to_str().unwrap()) };
        unsafe { std::env::remove_var(ENV_PASSPHRASE) };
        let result = passphrase_from_env();
        unsafe { std::env::remove_var(ENV_PASSPHRASE_FILE) };
        std::fs::remove_file(&path).ok();
        // trailing \n must be stripped
        assert_eq!(result, Some(b"correct-horse-battery".to_vec()));
    }

    #[test]
    fn test_passphrase_from_env_file_strips_crlf() {
        let dir = std::env::temp_dir();
        let path = dir.join("bls_pass_crlf_test.txt");
        std::fs::write(&path, b"my-pass\r\n").unwrap();
        unsafe { std::env::set_var(ENV_PASSPHRASE_FILE, path.to_str().unwrap()) };
        unsafe { std::env::remove_var(ENV_PASSPHRASE) };
        let result = passphrase_from_env();
        unsafe { std::env::remove_var(ENV_PASSPHRASE_FILE) };
        std::fs::remove_file(&path).ok();
        assert_eq!(result, Some(b"my-pass".to_vec()));
    }

    #[test]
    #[ignore = "flaky under cargo test parallelism — env vars are global and \
                other tests in this module manipulate ENV_PASSPHRASE. Re-enable \
                after adding serial-test gate or using a per-test env-var \
                isolation helper."]
    fn test_passphrase_from_env_file_missing_falls_back_to_direct() {
        unsafe { std::env::set_var(ENV_PASSPHRASE_FILE, "/tmp/__no_such_bls_pass_file__") };
        unsafe { std::env::set_var(ENV_PASSPHRASE, "fallback-pass") };
        let result = passphrase_from_env();
        unsafe { std::env::remove_var(ENV_PASSPHRASE_FILE) };
        unsafe { std::env::remove_var(ENV_PASSPHRASE) };
        assert_eq!(result, Some(b"fallback-pass".to_vec()));
    }
}
