use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use libp2p::PeerId;
use tracing::{info, warn};

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
#[derive(Clone)]
#[derive(Debug)]
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
        info!("Peer authority enforcing allowlist with {} peers", peers.len());
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

    std::fs::write(&cert_path, cert.pem()).map_err(|e| format!("write CA cert: {e}"))?;
    std::fs::write(&key_path, key_pair.serialize_pem()).map_err(|e| format!("write CA key: {e}"))?;

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
    let params =
        CertificateParams::new(vec![san]).map_err(|e| format!("validator params: {e}"))?;
    let validator_key = KeyPair::generate().map_err(|e| format!("validator keygen: {e}"))?;

    let validator_cert = params
        .signed_by(&validator_key, &ca_cert, &ca_key)
        .map_err(|e| format!("sign validator cert: {e}"))?;

    let cert_path = output_dir.join(format!("{validator_name}-cert.pem"));
    let key_path = output_dir.join(format!("{validator_name}-key.pem"));

    std::fs::write(&cert_path, validator_cert.pem())
        .map_err(|e| format!("write validator cert: {e}"))?;
    std::fs::write(&key_path, validator_key.serialize_pem())
        .map_err(|e| format!("write validator key: {e}"))?;

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
        let ca_key = std::fs::read_to_string(dir.join("ca-key.pem")).unwrap();

        generate_validator_cert("validator0", &ca_cert, &ca_key, &dir)
            .expect("validator cert generation");
        assert!(dir.join("validator0-cert.pem").exists());
        assert!(dir.join("validator0-key.pem").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
