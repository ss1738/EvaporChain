use pqcrypto_dilithium::dilithium3;
use pqcrypto_traits::sign::{
    DetachedSignature, PublicKey as PqPublicKey, SecretKey as PqSecretKey,
};

// ─────────────────────────── Traits ─────────────────────────────────────

/// Signing half of a signature scheme. Holds the secret key.
pub trait Signer: Send + Sync {
    /// Sign a message, returning the raw signature bytes.
    fn sign(&self, msg: &[u8]) -> Vec<u8>;
    /// Return the public key bytes corresponding to this signer.
    fn public_key_bytes(&self) -> Vec<u8>;
}

/// Stateless verification of signatures.
pub trait Verifier: Send + Sync {
    /// Verify `signature` over `msg` against `public_key`.
    fn verify(msg: &[u8], signature: &[u8], public_key: &[u8]) -> bool;
}

// ─────────────────────── ML-DSA (Dilithium3) ────────────────────────────

/// ML-DSA keypair for post-quantum transaction signing.
///
/// Uses NIST security level 3 (Dilithium3).
/// Public key: 1952 bytes, Signature: 3293 bytes.
pub struct MlDsaKeypair {
    pk: dilithium3::PublicKey,
    sk: dilithium3::SecretKey,
}

impl MlDsaKeypair {
    /// Generate a new random keypair.
    pub fn generate() -> Self {
        let (pk, sk) = dilithium3::keypair();
        Self { pk, sk }
    }

    /// Reconstruct a keypair from raw bytes.
    pub fn from_bytes(pk_bytes: &[u8], sk_bytes: &[u8]) -> Result<Self, MlDsaError> {
        let pk = dilithium3::PublicKey::from_bytes(pk_bytes)
            .map_err(|_| MlDsaError::InvalidPublicKey)?;
        let sk = dilithium3::SecretKey::from_bytes(sk_bytes)
            .map_err(|_| MlDsaError::InvalidSecretKey)?;
        Ok(Self { pk, sk })
    }

    /// Raw public key bytes.
    pub fn public_key(&self) -> &[u8] {
        self.pk.as_bytes()
    }

    /// Raw secret key bytes.
    pub fn secret_key(&self) -> &[u8] {
        self.sk.as_bytes()
    }
}

impl Signer for MlDsaKeypair {
    fn sign(&self, msg: &[u8]) -> Vec<u8> {
        let sig = dilithium3::detached_sign(msg, &self.sk);
        sig.as_bytes().to_vec()
    }

    fn public_key_bytes(&self) -> Vec<u8> {
        self.pk.as_bytes().to_vec()
    }
}

/// Stateless ML-DSA signature verifier.
pub struct MlDsaVerifier;

impl Verifier for MlDsaVerifier {
    fn verify(msg: &[u8], signature: &[u8], public_key: &[u8]) -> bool {
        let pk = match dilithium3::PublicKey::from_bytes(public_key) {
            Ok(pk) => pk,
            Err(_) => return false,
        };
        let sig = match dilithium3::DetachedSignature::from_bytes(signature) {
            Ok(sig) => sig,
            Err(_) => return false,
        };
        dilithium3::verify_detached_signature(&sig, msg, &pk).is_ok()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MlDsaError {
    #[error("invalid public key bytes")]
    InvalidPublicKey,
    #[error("invalid secret key bytes")]
    InvalidSecretKey,
}

// ──────────────────── BLS12-381 Stubs (future) ──────────────────────────

/// BLS12-381 public key for consensus attestation aggregation.
///
/// Stub — actual implementation using the `blst` crate will be added
/// when consensus attestation signing is implemented.
#[derive(Debug, Clone)]
pub struct BlsPublicKey(pub Vec<u8>);

/// BLS12-381 secret key.
#[derive(Debug, Clone)]
pub struct BlsSecretKey(pub Vec<u8>);

/// BLS12-381 signature (can be aggregated).
#[derive(Debug, Clone)]
pub struct BlsSignature(pub Vec<u8>);

/// Trait for BLS aggregate signature operations.
///
/// Will be implemented with the `blst` crate for consensus attestations.
/// Aggregation allows combining N validator signatures into one, reducing
/// on-chain footprint from O(N) to O(1).
pub trait BlsScheme: Send + Sync {
    fn generate_keypair() -> (BlsPublicKey, BlsSecretKey);
    fn sign(msg: &[u8], sk: &BlsSecretKey) -> BlsSignature;
    fn verify(msg: &[u8], sig: &BlsSignature, pk: &BlsPublicKey) -> bool;
    fn aggregate_signatures(sigs: &[BlsSignature]) -> BlsSignature;
    fn aggregate_verify(msg: &[u8], agg_sig: &BlsSignature, pks: &[BlsPublicKey]) -> bool;
}

// ─────────────────────────── Tests ───────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mldsa_sign_verify_roundtrip() {
        let kp = MlDsaKeypair::generate();
        let msg = b"transfer 100 tokens from Alice to Bob";
        let sig = kp.sign(msg);
        let pk = kp.public_key_bytes();
        assert!(MlDsaVerifier::verify(msg, &sig, &pk));
    }

    #[test]
    fn test_mldsa_different_messages() {
        let kp = MlDsaKeypair::generate();
        let sig = kp.sign(b"message A");
        let pk = kp.public_key_bytes();

        // Signature for "message A" should NOT verify against "message B"
        assert!(!MlDsaVerifier::verify(b"message B", &sig, &pk));
    }

    #[test]
    fn test_mldsa_wrong_key_rejects() {
        let kp1 = MlDsaKeypair::generate();
        let kp2 = MlDsaKeypair::generate();
        let msg = b"hello";
        let sig = kp1.sign(msg);

        // Signature from kp1 should NOT verify with kp2's public key
        assert!(!MlDsaVerifier::verify(msg, &sig, &kp2.public_key_bytes()));
    }

    #[test]
    fn test_mldsa_invalid_signature_bytes() {
        let kp = MlDsaKeypair::generate();
        let msg = b"hello";
        let garbage_sig = vec![0xFFu8; 100]; // wrong length

        assert!(!MlDsaVerifier::verify(msg, &garbage_sig, &kp.public_key_bytes()));
    }

    #[test]
    fn test_mldsa_invalid_pubkey_bytes() {
        let kp = MlDsaKeypair::generate();
        let msg = b"hello";
        let sig = kp.sign(msg);
        let garbage_pk = vec![0x00u8; 10];

        assert!(!MlDsaVerifier::verify(msg, &sig, &garbage_pk));
    }

    #[test]
    fn test_mldsa_tampered_signature() {
        let kp = MlDsaKeypair::generate();
        let msg = b"hello";
        let mut sig = kp.sign(msg);
        // Flip a byte
        sig[0] ^= 0xFF;

        assert!(!MlDsaVerifier::verify(msg, &sig, &kp.public_key_bytes()));
    }

    #[test]
    fn test_mldsa_empty_message() {
        let kp = MlDsaKeypair::generate();
        let sig = kp.sign(b"");
        assert!(MlDsaVerifier::verify(b"", &sig, &kp.public_key_bytes()));
    }

    #[test]
    fn test_mldsa_large_message() {
        let kp = MlDsaKeypair::generate();
        let msg = vec![0xABu8; 10_000];
        let sig = kp.sign(&msg);
        assert!(MlDsaVerifier::verify(&msg, &sig, &kp.public_key_bytes()));
    }

    #[test]
    fn test_mldsa_keypair_from_bytes_roundtrip() {
        let kp = MlDsaKeypair::generate();
        let pk_bytes = kp.public_key().to_vec();
        let sk_bytes = kp.secret_key().to_vec();

        let kp2 = MlDsaKeypair::from_bytes(&pk_bytes, &sk_bytes).unwrap();

        let msg = b"roundtrip test";
        let sig = kp2.sign(msg);
        assert!(MlDsaVerifier::verify(msg, &sig, &pk_bytes));
    }

    #[test]
    fn test_mldsa_signer_trait_object() {
        let kp = MlDsaKeypair::generate();
        let signer: &dyn Signer = &kp;

        let msg = b"trait object signing";
        let sig = signer.sign(msg);
        let pk = signer.public_key_bytes();
        assert!(MlDsaVerifier::verify(msg, &sig, &pk));
    }
}
