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

// ──────────────────── BLS12-381 (Consensus Attestations) ────────────────

use blst::min_pk::{AggregateSignature, PublicKey as BlstPublicKey, SecretKey as BlstSecretKey, Signature as BlstSignature};

const BLS_DST: &[u8] = b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_NUL_";

/// BLS12-381 public key for consensus attestation aggregation.
/// Public key: 48 bytes (compressed G1 point).
#[derive(Debug, Clone, PartialEq)]
pub struct BlsPublicKey(pub Vec<u8>);

/// BLS12-381 secret key (32 bytes scalar).
#[derive(Debug, Clone)]
pub struct BlsSecretKey(pub Vec<u8>);

/// BLS12-381 signature (96 bytes compressed G2 point). Aggregatable.
#[derive(Debug, Clone, PartialEq)]
pub struct BlsSignature(pub Vec<u8>);

/// BLS12-381 keypair for validator consensus signing.
pub struct BlsKeypair {
    sk: BlstSecretKey,
    pk: BlstPublicKey,
}

impl BlsKeypair {
    /// Generate a new random BLS keypair.
    pub fn generate() -> Self {
        let mut ikm = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut ikm);
        let sk = BlstSecretKey::key_gen(&ikm, &[]).expect("BLS key generation failed");
        let pk = sk.sk_to_pk();
        Self { sk, pk }
    }

    /// Reconstruct from raw secret key bytes (32 bytes).
    pub fn from_secret_bytes(sk_bytes: &[u8]) -> Result<Self, BlsError> {
        let sk = BlstSecretKey::from_bytes(sk_bytes)
            .map_err(|_| BlsError::InvalidSecretKey)?;
        let pk = sk.sk_to_pk();
        Ok(Self { sk, pk })
    }

    /// Sign a message.
    pub fn sign(&self, msg: &[u8]) -> BlsSignature {
        let sig = self.sk.sign(msg, BLS_DST, &[]);
        BlsSignature(sig.to_bytes().to_vec())
    }

    /// Get compressed public key bytes (48 bytes).
    pub fn public_key_bytes(&self) -> BlsPublicKey {
        BlsPublicKey(self.pk.to_bytes().to_vec())
    }

    /// Get raw secret key bytes (32 bytes).
    pub fn secret_key_bytes(&self) -> BlsSecretKey {
        BlsSecretKey(self.sk.to_bytes().to_vec())
    }
}

/// Stateless BLS verification and aggregation.
pub struct BlsVerifier;

impl BlsVerifier {
    /// Verify a single BLS signature.
    pub fn verify(msg: &[u8], sig: &BlsSignature, pk: &BlsPublicKey) -> bool {
        let pk = match BlstPublicKey::from_bytes(&pk.0) {
            Ok(pk) => pk,
            Err(_) => return false,
        };
        let sig = match BlstSignature::from_bytes(&sig.0) {
            Ok(sig) => sig,
            Err(_) => return false,
        };
        sig.verify(true, msg, BLS_DST, &[], &pk, true) == blst::BLST_ERROR::BLST_SUCCESS
    }

    /// Aggregate multiple BLS signatures into one.
    /// Returns None if the input is empty or contains invalid signatures.
    pub fn aggregate_signatures(sigs: &[BlsSignature]) -> Option<BlsSignature> {
        if sigs.is_empty() {
            return None;
        }
        let parsed: Vec<BlstSignature> = sigs.iter()
            .filter_map(|s| BlstSignature::from_bytes(&s.0).ok())
            .collect();
        if parsed.len() != sigs.len() {
            return None;
        }
        let refs: Vec<&BlstSignature> = parsed.iter().collect();
        let agg = AggregateSignature::aggregate(&refs, true).ok()?;
        Some(BlsSignature(agg.to_signature().to_bytes().to_vec()))
    }

    /// Verify an aggregated signature against multiple public keys.
    /// All signers must have signed the same message.
    pub fn aggregate_verify(msg: &[u8], agg_sig: &BlsSignature, pks: &[BlsPublicKey]) -> bool {
        if pks.is_empty() {
            return false;
        }
        let sig = match BlstSignature::from_bytes(&agg_sig.0) {
            Ok(sig) => sig,
            Err(_) => return false,
        };
        let parsed_pks: Vec<BlstPublicKey> = pks.iter()
            .filter_map(|pk| BlstPublicKey::from_bytes(&pk.0).ok())
            .collect();
        if parsed_pks.len() != pks.len() {
            return false;
        }
        let pk_refs: Vec<&BlstPublicKey> = parsed_pks.iter().collect();
        sig.fast_aggregate_verify(true, msg, BLS_DST, &pk_refs) == blst::BLST_ERROR::BLST_SUCCESS
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BlsError {
    #[error("invalid BLS public key bytes")]
    InvalidPublicKey,
    #[error("invalid BLS secret key bytes")]
    InvalidSecretKey,
    #[error("invalid BLS signature bytes")]
    InvalidSignature,
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

    // ─── BLS12-381 Tests ────────────────────────────────────────────────

    #[test]
    fn test_bls_sign_verify_roundtrip() {
        let kp = BlsKeypair::generate();
        let msg = b"consensus prevote for block 42";
        let sig = kp.sign(msg);
        let pk = kp.public_key_bytes();
        assert!(BlsVerifier::verify(msg, &sig, &pk));
    }

    #[test]
    fn test_bls_wrong_message_rejects() {
        let kp = BlsKeypair::generate();
        let sig = kp.sign(b"message A");
        let pk = kp.public_key_bytes();
        assert!(!BlsVerifier::verify(b"message B", &sig, &pk));
    }

    #[test]
    fn test_bls_wrong_key_rejects() {
        let kp1 = BlsKeypair::generate();
        let kp2 = BlsKeypair::generate();
        let msg = b"hello";
        let sig = kp1.sign(msg);
        assert!(!BlsVerifier::verify(msg, &sig, &kp2.public_key_bytes()));
    }

    #[test]
    fn test_bls_aggregate_signatures() {
        let kp1 = BlsKeypair::generate();
        let kp2 = BlsKeypair::generate();
        let kp3 = BlsKeypair::generate();
        let msg = b"block hash for height 100";

        let sig1 = kp1.sign(msg);
        let sig2 = kp2.sign(msg);
        let sig3 = kp3.sign(msg);

        let agg = BlsVerifier::aggregate_signatures(&[sig1, sig2, sig3]).unwrap();
        let pks = vec![kp1.public_key_bytes(), kp2.public_key_bytes(), kp3.public_key_bytes()];
        assert!(BlsVerifier::aggregate_verify(msg, &agg, &pks));
    }

    #[test]
    fn test_bls_aggregate_wrong_message_rejects() {
        let kp1 = BlsKeypair::generate();
        let kp2 = BlsKeypair::generate();
        let msg = b"correct message";

        let sig1 = kp1.sign(msg);
        let sig2 = kp2.sign(msg);

        let agg = BlsVerifier::aggregate_signatures(&[sig1, sig2]).unwrap();
        let pks = vec![kp1.public_key_bytes(), kp2.public_key_bytes()];
        assert!(!BlsVerifier::aggregate_verify(b"wrong message", &agg, &pks));
    }

    #[test]
    fn test_bls_aggregate_missing_signer_rejects() {
        let kp1 = BlsKeypair::generate();
        let kp2 = BlsKeypair::generate();
        let kp3 = BlsKeypair::generate();
        let msg = b"block hash";

        // Only kp1 and kp2 signed, but we claim kp3 also signed
        let sig1 = kp1.sign(msg);
        let sig2 = kp2.sign(msg);
        let agg = BlsVerifier::aggregate_signatures(&[sig1, sig2]).unwrap();

        let pks = vec![kp1.public_key_bytes(), kp2.public_key_bytes(), kp3.public_key_bytes()];
        assert!(!BlsVerifier::aggregate_verify(msg, &agg, &pks));
    }

    #[test]
    fn test_bls_keypair_from_secret_roundtrip() {
        let kp = BlsKeypair::generate();
        let sk_bytes = kp.secret_key_bytes();
        let kp2 = BlsKeypair::from_secret_bytes(&sk_bytes.0).unwrap();

        let msg = b"roundtrip";
        let sig = kp2.sign(msg);
        assert!(BlsVerifier::verify(msg, &sig, &kp.public_key_bytes()));
    }

    #[test]
    fn test_bls_invalid_bytes() {
        let garbage = vec![0xFFu8; 10];
        assert!(!BlsVerifier::verify(b"msg", &BlsSignature(garbage.clone()), &BlsPublicKey(garbage)));
    }

    // ─── ML-DSA Trait Object Tests ────────────────────────────────────

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
