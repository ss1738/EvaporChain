use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use libp2p::PeerId;
use tracing::{info, warn};

use evaporchain_crypto::secret_file_store;

/// Operator-supplied passphrase for encrypted-at-rest TLS private keys.
/// Reuses the same env var as `bls_key_store::ENV_PASSPHRASE` so a single
/// passphrase covers every validator-side secret on disk.
fn passphrase_from_env() -> Option<Vec<u8>> {
    evaporchain_crypto::bls_key_store::passphrase_from_env()
}

/// Write a secret file with mode 0600. Mirrors the helper in
/// `node::main::write_secret_file`.
fn write_secret_file(path: &Path, data: &[u8]) -> Result<(), String> {
    std::fs::write(path, data).map_err(|e| format!("write {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Persist a PEM blob to disk. Inner version: caller supplies an explicit
/// optional passphrase rather than reading from env. Useful for tests
/// that must not pollute process-wide env state.
///
/// Behaviour: with `Some(pass)` the PEM is wrapped in an EVKV envelope
/// (Argon2id + XChaCha20-Poly1305) before write. With `None` the PEM is
/// written plaintext and a warning is logged. File mode is always 0600
/// on Unix.
fn write_pem_secret_inner(
    path: &Path,
    pem: &str,
    label: &str,
    passphrase: Option<&[u8]>,
) -> Result<(), String> {
    match passphrase {
        Some(pass) => match secret_file_store::encrypt_blob(pem.as_bytes(), pass) {
            Ok(blob) => {
                write_secret_file(path, &blob)?;
                info!(
                    label,
                    path = %path.display(),
                    "TLS secret encrypted at rest (EVKV: Argon2id + XChaCha20-Poly1305)"
                );
                Ok(())
            }
            Err(e) => {
                warn!(
                    label,
                    error = %e,
                    "Failed to encrypt TLS secret; falling back to plaintext"
                );
                write_secret_file(path, pem.as_bytes())
            }
        },
        None => {
            write_secret_file(path, pem.as_bytes())?;
            warn!(
                label,
                env = evaporchain_crypto::bls_key_store::ENV_PASSPHRASE,
                "TLS secret written in plaintext. Set the env var to enable encrypted-at-rest storage."
            );
            Ok(())
        }
    }
}

/// Persist a PEM blob to disk. When `EVAPORCHAIN_VALIDATOR_KEY_PASS` is set,
/// the PEM is wrapped in an EVKV envelope before write. Otherwise the
/// historical plaintext path is taken and a warning is logged. File mode
/// is always 0600 on Unix.
///
/// Closes punch-list item #3b. The shape mirrors the BLS validator key
/// path in `node::main::write_bls_secret`.
fn write_pem_secret(path: &Path, pem: &str, label: &str) -> Result<(), String> {
    let pass = passphrase_from_env();
    write_pem_secret_inner(path, pem, label, pass.as_deref())
}

/// Inner reader — caller supplies the passphrase explicitly. Used by tests.
///
/// `migrate_plaintext`: when true and the file on disk is plaintext PEM
/// (`-----BEGIN ` prefix) AND a passphrase is available, the file is
/// rewritten in EVKV-wrapped form before the decoded PEM is returned.
/// Used by the public `read_pem_secret` path so a node started with the
/// passphrase env set silently upgrades any legacy plaintext keys it finds.
fn read_pem_secret_inner_with_migration(
    path: &Path,
    passphrase: Option<&[u8]>,
    migrate_plaintext: bool,
) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if secret_file_store::is_evkv(&bytes) {
        let pass = passphrase.ok_or_else(|| {
            format!(
                "{} is encrypted (EVKV) but {} is not set",
                path.display(),
                evaporchain_crypto::bls_key_store::ENV_PASSPHRASE
            )
        })?;
        let plain = secret_file_store::decrypt_blob(&bytes, pass)
            .map_err(|e| format!("decrypt {}: {e}", path.display()))?;
        String::from_utf8(plain).map_err(|e| format!("decrypted PEM not valid UTF-8: {e}"))
    } else {
        let pem = String::from_utf8(bytes.clone())
            .map_err(|e| format!("plaintext PEM not valid UTF-8: {e}"))?;
        // Auto-migrate legacy plaintext keys when a passphrase is available.
        // Detection: `-----BEGIN ` prefix is the canonical PEM marker; it
        // can never collide with the EVKV magic checked above.
        if migrate_plaintext && pem.starts_with("-----BEGIN ") {
            if let Some(pass) = passphrase {
                match secret_file_store::encrypt_blob(pem.as_bytes(), pass) {
                    Ok(blob) => {
                        if let Err(e) = write_secret_file(path, &blob) {
                            warn!(
                                path = %path.display(),
                                error = %e,
                                "tls: failed to migrate plaintext PEM to EVKV envelope"
                            );
                        } else {
                            info!(
                                "tls: migrated {} to encrypted format (envelope EVKV v1)",
                                path.display()
                            );
                        }
                    }
                    Err(e) => warn!(
                        path = %path.display(),
                        error = %e,
                        "tls: failed to encrypt plaintext PEM during migration"
                    ),
                }
            }
        }
        Ok(pem)
    }
}

/// Inner reader — caller supplies the passphrase explicitly. Used by tests.
/// Migration is disabled by default to keep tests deterministic.
fn read_pem_secret_inner(path: &Path, passphrase: Option<&[u8]>) -> Result<String, String> {
    read_pem_secret_inner_with_migration(path, passphrase, false)
}

/// Load a TLS private key PEM from disk, transparently decrypting when the
/// file is in EVKV envelope format. Returns the raw PEM string.
///
/// Auto-detection is by magic prefix (4-byte `EVKV`). A plaintext PEM file
/// will always start with `-----BEGIN`, which never collides with the
/// EVKV magic, so the detection is unambiguous.
///
/// If a plaintext PEM is encountered AND
/// `EVAPORCHAIN_VALIDATOR_KEY_PASS` is set, the file is auto-migrated
/// in-place to the EVKV envelope (Argon2id + XChaCha20-Poly1305) and a
/// one-line migration log is emitted. This closes the K-09 audit item:
/// nodes restarted with the passphrase env set silently upgrade any
/// legacy plaintext keys on disk.
pub fn read_pem_secret(path: &Path) -> Result<String, String> {
    read_pem_secret_inner_with_migration(path, passphrase_from_env().as_deref(), true)
}

/// Returns true if the file at `path` is in EVKV envelope format.
/// Used by mainnet strict-mode startup to refuse plaintext TLS keys.
pub fn is_pem_encrypted(path: &Path) -> Result<bool, String> {
    let mut buf = [0u8; 4];
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let n = f
        .read(&mut buf)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(n == 4 && secret_file_store::is_evkv(&buf))
}

/// TLS configuration for validator-to-validator communication.
#[derive(Debug, Clone)]
pub struct TlsConfig {
    /// Path to the validator's TLS certificate (PEM).
    pub cert_path: PathBuf,
    /// Path to the validator's TLS private key (PEM).
    pub key_path: PathBuf,
    /// Path to the CA certificate for verifying peers (PEM).
    /// If set, only peers presenting certs signed by this CA are accepted.
    pub ca_path: Option<PathBuf>,
}

/// Controls which peers are authorized to connect.
///
/// In permissioned mode (validator networks), only peers in the allowlist
/// can establish connections. In permissionless mode, all peers are accepted.
#[derive(Clone, Debug)]
pub struct PeerAuthority {
    allowlist: Arc<RwLock<HashSet<PeerId>>>,
    enforcing: bool,
}

impl PeerAuthority {
    pub fn permissionless() -> Self {
        Self {
            allowlist: Arc::new(RwLock::new(HashSet::new())),
            enforcing: false,
        }
    }

    pub fn with_allowlist(peers: Vec<PeerId>) -> Self {
        info!(
            "Peer authority enforcing allowlist with {} peers",
            peers.len()
        );
        Self {
            allowlist: Arc::new(RwLock::new(peers.into_iter().collect())),
            enforcing: true,
        }
    }

    pub fn is_authorized(&self, peer: &PeerId) -> bool {
        if !self.enforcing {
            return true;
        }
        self.allowlist
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .contains(peer)
    }

    pub fn add_peer(&self, peer: PeerId) {
        if self.enforcing {
            let mut list = self.allowlist.write().unwrap_or_else(|p| p.into_inner());
            if list.insert(peer) {
                info!("Added {peer} to peer allowlist (total: {})", list.len());
            }
        }
    }

    pub fn remove_peer(&self, peer: &PeerId) {
        if self.enforcing {
            let mut list = self.allowlist.write().unwrap_or_else(|p| p.into_inner());
            if list.remove(peer) {
                warn!("Removed {peer} from peer allowlist (total: {})", list.len());
            }
        }
    }

    pub fn peer_count(&self) -> usize {
        self.allowlist
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }
}

/// Generate a self-signed CA certificate and key for a validator network.
pub fn generate_ca(output_dir: &Path) -> Result<(), String> {
    use rcgen::{CertificateParams, KeyPair};

    let mut params = CertificateParams::new(vec!["EvaporChain Validator CA".to_string()])
        .map_err(|e| format!("CA params: {e}"))?;
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);

    let key_pair = KeyPair::generate().map_err(|e| format!("CA keygen: {e}"))?;
    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| format!("CA self-sign: {e}"))?;

    let cert_path = output_dir.join("ca-cert.pem");
    let key_path = output_dir.join("ca-key.pem");

    // Cert PEM is public key material — written plaintext.
    std::fs::write(&cert_path, cert.pem()).map_err(|e| format!("write CA cert: {e}"))?;
    // CA private key is encrypted at rest when EVAPORCHAIN_VALIDATOR_KEY_PASS is set.
    write_pem_secret(&key_path, &key_pair.serialize_pem(), "ca-key")?;

    info!("Generated CA certificate at {}", cert_path.display());
    Ok(())
}

/// Generate a validator certificate signed by the CA.
///
/// Requires the CA cert and key PEM strings from `generate_ca()`.
pub fn generate_validator_cert(
    validator_name: &str,
    _ca_cert_pem: &str,
    ca_key_pem: &str,
    output_dir: &Path,
) -> Result<(), String> {
    use rcgen::{CertificateParams, KeyPair};

    // Reconstruct the CA from its key (we re-sign with the same key)
    let ca_key = KeyPair::from_pem(ca_key_pem).map_err(|e| format!("parse CA key: {e}"))?;

    let mut ca_params = CertificateParams::new(vec!["EvaporChain Validator CA".to_string()])
        .map_err(|e| format!("CA params: {e}"))?;
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_cert = ca_params
        .self_signed(&ca_key)
        .map_err(|e| format!("reconstruct CA: {e}"))?;

    let san = format!("{validator_name}.evaporchain.local");
    let params = CertificateParams::new(vec![san]).map_err(|e| format!("validator params: {e}"))?;
    let validator_key = KeyPair::generate().map_err(|e| format!("validator keygen: {e}"))?;

    let validator_cert = params
        .signed_by(&validator_key, &ca_cert, &ca_key)
        .map_err(|e| format!("sign validator cert: {e}"))?;

    let cert_path = output_dir.join(format!("{validator_name}-cert.pem"));
    let key_path = output_dir.join(format!("{validator_name}-key.pem"));

    // Public cert PEM — plaintext.
    std::fs::write(&cert_path, validator_cert.pem())
        .map_err(|e| format!("write validator cert: {e}"))?;
    // Private key — encrypted at rest when the validator passphrase env is set.
    let key_label = format!("{validator_name}-key");
    write_pem_secret(&key_path, &validator_key.serialize_pem(), &key_label)?;

    info!(
        "Generated validator certificate for '{validator_name}' at {}",
        cert_path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::identity::Keypair;

    #[test]
    fn permissionless_allows_all() {
        let auth = PeerAuthority::permissionless();
        let kp = Keypair::generate_ed25519();
        let peer = kp.public().to_peer_id();
        assert!(auth.is_authorized(&peer));
    }

    #[test]
    fn allowlist_blocks_unknown() {
        let kp1 = Keypair::generate_ed25519();
        let kp2 = Keypair::generate_ed25519();
        let peer1 = kp1.public().to_peer_id();
        let peer2 = kp2.public().to_peer_id();

        let auth = PeerAuthority::with_allowlist(vec![peer1]);
        assert!(auth.is_authorized(&peer1));
        assert!(!auth.is_authorized(&peer2));
    }

    #[test]
    fn add_remove_peer() {
        let kp = Keypair::generate_ed25519();
        let peer = kp.public().to_peer_id();

        let auth = PeerAuthority::with_allowlist(vec![]);
        assert!(!auth.is_authorized(&peer));

        auth.add_peer(peer);
        assert!(auth.is_authorized(&peer));

        auth.remove_peer(&peer);
        assert!(!auth.is_authorized(&peer));
    }

    #[test]
    fn generate_ca_and_validator_certs() {
        let dir = std::env::temp_dir().join("evaporchain_tls_test");
        let _ = std::fs::create_dir_all(&dir);

        generate_ca(&dir).expect("CA generation");
        assert!(dir.join("ca-cert.pem").exists());
        assert!(dir.join("ca-key.pem").exists());

        let ca_cert = std::fs::read_to_string(dir.join("ca-cert.pem")).unwrap();
        // Cert files are public — read_to_string is fine. Private keys
        // must go through `read_pem_secret_inner` to handle the EVKV path.
        let ca_key = read_pem_secret_inner(&dir.join("ca-key.pem"), None).expect("read CA key");

        generate_validator_cert("validator0", &ca_cert, &ca_key, &dir)
            .expect("validator cert generation");
        assert!(dir.join("validator0-cert.pem").exists());
        assert!(dir.join("validator0-key.pem").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Round-trip the encrypt-at-rest TLS path without touching process env:
    /// write a PEM with an explicit passphrase, read it back through the
    /// EVKV-aware reader, confirm bytes match. Closes punch-list item #3b.
    #[test]
    fn pem_secret_encrypted_at_rest_roundtrip() {
        let dir = std::env::temp_dir().join("evaporchain_tls_evkv_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("secret-key.pem");

        let pem = "-----BEGIN PRIVATE KEY-----\n\
                   MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg\n\
                   -----END PRIVATE KEY-----\n";
        let pass: &[u8] = b"strict-mainnet-passphrase";

        write_pem_secret_inner(&path, pem, "test-key", Some(pass)).expect("write encrypted");

        // File on disk must NOT start with the PEM marker — i.e. it's encrypted.
        let raw = std::fs::read(&path).unwrap();
        assert!(
            secret_file_store::is_evkv(&raw),
            "file should be EVKV-wrapped, got: {:?}",
            &raw[..raw.len().min(8)]
        );
        assert!(is_pem_encrypted(&path).unwrap());

        // Reader returns the original PEM bytes-for-bytes when the
        // passphrase matches.
        let recovered = read_pem_secret_inner(&path, Some(pass)).expect("decrypt");
        assert_eq!(recovered, pem);

        // Wrong passphrase fails cleanly.
        let err = read_pem_secret_inner(&path, Some(b"wrong")).unwrap_err();
        assert!(err.contains("decrypt"), "unexpected error: {err}");

        // Reader without passphrase against an encrypted file refuses
        // rather than handing back garbage bytes.
        let err = read_pem_secret_inner(&path, None).unwrap_err();
        assert!(err.contains("EVKV"), "unexpected error: {err}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Auto-migration path: a plaintext PEM on disk is rewritten as an
    /// EVKV envelope when the reader is invoked with a passphrase AND
    /// migration is enabled. The decoded bytes returned to the caller
    /// stay byte-identical to the original PEM.
    #[test]
    fn pem_secret_plaintext_migrates_to_evkv() {
        let dir = std::env::temp_dir().join("evaporchain_tls_migrate_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("legacy-key.pem");

        let pem = "-----BEGIN PRIVATE KEY-----\nlegacydata\n-----END PRIVATE KEY-----\n";
        // Write plaintext directly — simulates a key persisted by an
        // older node version that ran before the EVKV envelope existed.
        std::fs::write(&path, pem).unwrap();
        assert!(!is_pem_encrypted(&path).unwrap());

        let pass: &[u8] = b"strict-mainnet-passphrase";
        let recovered =
            read_pem_secret_inner_with_migration(&path, Some(pass), true).expect("read+migrate");
        assert_eq!(recovered, pem);

        // Post-read: file on disk is now EVKV-wrapped.
        assert!(is_pem_encrypted(&path).unwrap());

        // And a follow-up read with the same passphrase still returns the
        // original PEM bytes-for-bytes.
        let again = read_pem_secret_inner(&path, Some(pass)).expect("re-read");
        assert_eq!(again, pem);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Migration is a no-op when no passphrase is supplied — the file
    /// stays plaintext and the reader still returns the bytes. This
    /// preserves the legacy fallback for non-mainnet-strict nodes.
    #[test]
    fn pem_secret_plaintext_no_migration_without_passphrase() {
        let dir = std::env::temp_dir().join("evaporchain_tls_no_migrate_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("legacy-key.pem");

        let pem = "-----BEGIN PRIVATE KEY-----\nlegacydata\n-----END PRIVATE KEY-----\n";
        std::fs::write(&path, pem).unwrap();

        let recovered =
            read_pem_secret_inner_with_migration(&path, None, true).expect("read no-migrate");
        assert_eq!(recovered, pem);
        // File untouched.
        assert!(!is_pem_encrypted(&path).unwrap());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Plaintext PEM (legacy path) is still readable via the same helper.
    #[test]
    fn pem_secret_plaintext_passthrough() {
        let dir = std::env::temp_dir().join("evaporchain_tls_plaintext_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("plain-key.pem");

        let pem = "-----BEGIN PRIVATE KEY-----\nplaintext\n-----END PRIVATE KEY-----\n";
        write_pem_secret_inner(&path, pem, "test-plain", None).expect("write");

        // No EVKV magic — file is raw PEM.
        let raw = std::fs::read(&path).unwrap();
        assert!(!secret_file_store::is_evkv(&raw));
        assert!(!is_pem_encrypted(&path).unwrap());

        let recovered = read_pem_secret_inner(&path, None).expect("read");
        assert_eq!(recovered, pem);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
