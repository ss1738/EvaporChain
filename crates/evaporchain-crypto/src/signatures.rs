use k256::ecdsa::{
    signature::Signer as K256Signer,
    signature::Verifier as K256Verifier,
    Signature as K256Signature,
    SigningKey, VerifyingKey,
};
use pqc_dilithium::Keypair;
use zeroize::Zeroize;

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
/// Uses NIST security level 3 (Dilithium3) via `pqc_dilithium` mode3.
/// Public key: 1952 bytes, Secret key: 4000 bytes, Signature: 3293 bytes.
///
/// This is the same pure-Rust implementation used in the WASM bridge,
/// ensuring byte-level compatibility between node and browser extension.
pub struct MlDsaKeypair {
    inner: Keypair,
}

impl Drop for MlDsaKeypair {
    fn drop(&mut self) {
        // Zeroize the entire keypair (both PK and SK) to prevent secret key leakage.
        // Zeroing the full struct avoids assuming internal field layout order.
        unsafe {
            let ptr = &mut self.inner as *mut Keypair as *mut u8;
            std::ptr::write_bytes(ptr, 0, std::mem::size_of::<Keypair>());
        }
    }
}

impl MlDsaKeypair {
    /// Generate a new random keypair.
    pub fn generate() -> Self {
        Self { inner: Keypair::generate() }
    }

    /// Reconstruct a keypair from raw bytes.
    ///
    /// # Safety rationale
    ///
    /// `pqc_dilithium::Keypair` has a **private** `secret` field and provides no
    /// public constructor from (pk, sk) bytes. The only way to inject key material
    /// is via raw pointer writes. We mitigate the layout assumption with:
    ///
    /// 1. A compile-time assertion that `size_of::<Keypair>() == PK + SK`, which
    ///    will fail if the crate ever adds padding, vtables, or extra fields.
    /// 2. A runtime check that the public field and `expose_secret()` return
    ///    exactly the bytes we wrote, catching any field-order swap.
    /// 3. A sign-then-verify roundtrip proving the reconstructed keypair is
    ///    functionally correct, not just byte-correct.
    pub fn from_bytes(pk_bytes: &[u8], sk_bytes: &[u8]) -> Result<Self, MlDsaError> {
        if pk_bytes.len() != pqc_dilithium::PUBLICKEYBYTES {
            return Err(MlDsaError::InvalidPublicKey);
        }
        if sk_bytes.len() != pqc_dilithium::SECRETKEYBYTES {
            return Err(MlDsaError::InvalidSecretKey);
        }

        // Compile-time layout guard: if pqc_dilithium ever changes Keypair's
        // internal representation (adds fields, padding, etc.) this will fail
        // at compile time, preventing silent misuse of the unsafe block below.
        const _: () = {
            let expected = pqc_dilithium::PUBLICKEYBYTES + pqc_dilithium::SECRETKEYBYTES;
            assert!(
                std::mem::size_of::<Keypair>() == expected,
                "pqc_dilithium::Keypair layout changed — unsafe from_bytes is invalid"
            );
        };

        // Reconstruct Keypair by generating a dummy then overwriting fields.
        let mut kp = Keypair::generate();

        // SAFETY: Keypair layout is { public: [u8; PK], secret: [u8; SK] } with
        // no padding (verified by the const assertion above). We write pk_bytes
        // at offset 0 and sk_bytes at offset PUBLICKEYBYTES. The runtime checks
        // and sign/verify roundtrip below catch any remaining layout mismatch.
        unsafe {
            let ptr = &mut kp as *mut Keypair as *mut u8;
            std::ptr::copy_nonoverlapping(pk_bytes.as_ptr(), ptr, pqc_dilithium::PUBLICKEYBYTES);
            std::ptr::copy_nonoverlapping(
                sk_bytes.as_ptr(),
                ptr.add(pqc_dilithium::PUBLICKEYBYTES),
                pqc_dilithium::SECRETKEYBYTES,
            );
        }

        // Runtime verification: confirm the bytes ended up in the right fields.
        if kp.public.as_slice() != pk_bytes {
            return Err(MlDsaError::InvalidPublicKey);
        }
        if kp.expose_secret() != sk_bytes {
            return Err(MlDsaError::InvalidSecretKey);
        }

        // Functional roundtrip: sign a canary message and verify with the
        // extracted public key. This catches subtle corruption where bytes
        // landed in the right fields but produce an invalid signing state.
        let canary_msg = b"evaporchain-keypair-integrity-check";
        let canary_sig = kp.sign(canary_msg);
        if pqc_dilithium::verify(&canary_sig, canary_msg, &kp.public).is_err() {
            return Err(MlDsaError::InvalidSecretKey);
        }

        Ok(Self { inner: kp })
    }

    /// Raw public key bytes (1952 bytes).
    pub fn public_key(&self) -> &[u8] {
        &self.inner.public
    }

    /// Raw secret key bytes (4000 bytes).
    pub fn secret_key(&self) -> &[u8] {
        self.inner.expose_secret()
    }
}

impl Signer for MlDsaKeypair {
    fn sign(&self, msg: &[u8]) -> Vec<u8> {
        self.inner.sign(msg).to_vec()
    }

    fn public_key_bytes(&self) -> Vec<u8> {
        self.inner.public.to_vec()
    }
}

/// Stateless ML-DSA signature verifier.
pub struct MlDsaVerifier;

impl Verifier for MlDsaVerifier {
    fn verify(msg: &[u8], signature: &[u8], public_key: &[u8]) -> bool {
        if public_key.len() != pqc_dilithium::PUBLICKEYBYTES
            || signature.len() != pqc_dilithium::SIGNBYTES
        {
            return false;
        }
        pqc_dilithium::verify(signature, msg, public_key).is_ok()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MlDsaError {
    #[error("invalid public key bytes")]
    InvalidPublicKey,
    #[error("invalid secret key bytes")]
    InvalidSecretKey,
}

// ──────────────── ECDSA secp256k1 (Classical Signatures) ────────────────

const ECDSA_COMPRESSED_PK_LEN: usize = 33;
const ECDSA_SIGNATURE_LEN: usize = 64;

pub struct EcdsaKeypair {
    signing_key: SigningKey,
}

impl Drop for EcdsaKeypair {
    fn drop(&mut self) {
        let ptr = &mut self.signing_key as *mut SigningKey as *mut u8;
        unsafe { std::ptr::write_bytes(ptr, 0, std::mem::size_of::<SigningKey>()) };
    }
}

impl EcdsaKeypair {
    pub fn generate() -> Self {
        let signing_key = SigningKey::random(&mut rand::thread_rng());
        Self { signing_key }
    }

    pub fn from_bytes(sk_bytes: &[u8]) -> Result<Self, EcdsaError> {
        let signing_key = SigningKey::from_bytes(sk_bytes.into())
            .map_err(|_| EcdsaError::InvalidSecretKey)?;
        Ok(Self { signing_key })
    }

    pub fn public_key_compressed(&self) -> Vec<u8> {
        let vk = VerifyingKey::from(&self.signing_key);
        vk.to_encoded_point(true).as_bytes().to_vec()
    }

    pub fn secret_key_bytes(&self) -> Vec<u8> {
        self.signing_key.to_bytes().to_vec()
    }

    pub fn sign(&self, msg: &[u8]) -> Vec<u8> {
        let sig: K256Signature = self.signing_key.sign(msg);
        sig.to_bytes().to_vec()
    }
}

pub struct EcdsaVerifier;

impl EcdsaVerifier {
    pub fn verify(msg: &[u8], signature: &[u8], public_key: &[u8]) -> bool {
        if signature.len() != ECDSA_SIGNATURE_LEN || public_key.len() != ECDSA_COMPRESSED_PK_LEN {
            return false;
        }
        let vk = match VerifyingKey::from_sec1_bytes(public_key) {
            Ok(vk) => vk,
            Err(_) => return false,
        };
        let sig = match K256Signature::from_bytes(signature.into()) {
            Ok(sig) => sig,
            Err(_) => return false,
        };
        vk.verify(msg, &sig).is_ok()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EcdsaError {
    #[error("invalid ECDSA secret key bytes")]
    InvalidSecretKey,
    #[error("invalid ECDSA public key bytes")]
    InvalidPublicKey,
}

// ─────────────── Hybrid Post-Quantum (ECDSA + ML-DSA) ──────────────────
//
// Wire format (backward compatible):
//   Public key:
//     Legacy ML-DSA:  1952 raw bytes (no tag)
//     Hybrid:         0x02 || 33-byte ECDSA PK || 1952-byte ML-DSA PK  = 1986 bytes
//   Signature:
//     Legacy ML-DSA:  3293 raw bytes (no tag)
//     Hybrid:         0x02 || 64-byte ECDSA sig || 3293-byte ML-DSA sig = 3358 bytes
//
// Verification: BOTH signatures must independently verify.

const HYBRID_TAG: u8 = 0x02;
pub const HYBRID_PK_LEN: usize = 1 + ECDSA_COMPRESSED_PK_LEN + pqc_dilithium::PUBLICKEYBYTES;
pub const HYBRID_SIG_LEN: usize = 1 + ECDSA_SIGNATURE_LEN + pqc_dilithium::SIGNBYTES;

pub struct HybridKeypair {
    pub ecdsa: EcdsaKeypair,
    pub mldsa: MlDsaKeypair,
}

impl HybridKeypair {
    pub fn generate() -> Self {
        Self {
            ecdsa: EcdsaKeypair::generate(),
            mldsa: MlDsaKeypair::generate(),
        }
    }

    pub fn from_parts(ecdsa: EcdsaKeypair, mldsa: MlDsaKeypair) -> Self {
        Self { ecdsa, mldsa }
    }
}

impl Signer for HybridKeypair {
    fn sign(&self, msg: &[u8]) -> Vec<u8> {
        let ecdsa_sig = self.ecdsa.sign(msg);
        let mldsa_sig = self.mldsa.sign(msg);
        let mut out = Vec::with_capacity(HYBRID_SIG_LEN);
        out.push(HYBRID_TAG);
        out.extend_from_slice(&ecdsa_sig);
        out.extend_from_slice(&mldsa_sig);
        out
    }

    fn public_key_bytes(&self) -> Vec<u8> {
        let ecdsa_pk = self.ecdsa.public_key_compressed();
        let mldsa_pk = self.mldsa.public_key_bytes();
        let mut out = Vec::with_capacity(HYBRID_PK_LEN);
        out.push(HYBRID_TAG);
        out.extend_from_slice(&ecdsa_pk);
        out.extend_from_slice(&mldsa_pk);
        out
    }
}

pub struct HybridVerifier;

impl HybridVerifier {
    pub fn is_hybrid_pk(public_key: &[u8]) -> bool {
        public_key.len() == HYBRID_PK_LEN && public_key[0] == HYBRID_TAG
    }

    pub fn is_hybrid_sig(signature: &[u8]) -> bool {
        signature.len() == HYBRID_SIG_LEN && signature[0] == HYBRID_TAG
    }

    fn split_pk(public_key: &[u8]) -> Option<(&[u8], &[u8])> {
        if !Self::is_hybrid_pk(public_key) {
            return None;
        }
        let ecdsa_pk = &public_key[1..1 + ECDSA_COMPRESSED_PK_LEN];
        let mldsa_pk = &public_key[1 + ECDSA_COMPRESSED_PK_LEN..];
        Some((ecdsa_pk, mldsa_pk))
    }

    fn split_sig(signature: &[u8]) -> Option<(&[u8], &[u8])> {
        if !Self::is_hybrid_sig(signature) {
            return None;
        }
        let ecdsa_sig = &signature[1..1 + ECDSA_SIGNATURE_LEN];
        let mldsa_sig = &signature[1 + ECDSA_SIGNATURE_LEN..];
        Some((ecdsa_sig, mldsa_sig))
    }

    pub fn verify_hybrid(msg: &[u8], signature: &[u8], public_key: &[u8]) -> bool {
        let (ecdsa_pk, mldsa_pk) = match Self::split_pk(public_key) {
            Some(pks) => pks,
            None => return false,
        };
        let (ecdsa_sig, mldsa_sig) = match Self::split_sig(signature) {
            Some(sigs) => sigs,
            None => return false,
        };
        EcdsaVerifier::verify(msg, ecdsa_sig, ecdsa_pk)
            && MlDsaVerifier::verify(msg, mldsa_sig, mldsa_pk)
    }
}

impl Verifier for HybridVerifier {
    fn verify(msg: &[u8], signature: &[u8], public_key: &[u8]) -> bool {
        if Self::is_hybrid_pk(public_key) || Self::is_hybrid_sig(signature) {
            Self::verify_hybrid(msg, signature, public_key)
        } else {
            MlDsaVerifier::verify(msg, signature, public_key)
        }
    }
}

// ──────────────────── BLS12-381 (Consensus Attestations) ────────────────

use blst::min_pk::{AggregateSignature, PublicKey as BlstPublicKey, SecretKey as BlstSecretKey, Signature as BlstSignature};

const BLS_DST: &[u8] = b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_NUL_";
/// Domain separation tag for proof-of-possession (prevents rogue-key attacks).
/// Different from BLS_DST so PoP signatures cannot be replayed as message signatures.
const BLS_POP_DST: &[u8] = b"BLS_POP_BLS12381G2_XMD:SHA-256_SSWU_RO_POP_";

/// BLS12-381 public key for consensus attestation aggregation.
/// Public key: 48 bytes (compressed G1 point).
#[derive(Debug, Clone, PartialEq)]
pub struct BlsPublicKey(pub Vec<u8>);

/// BLS12-381 secret key (32 bytes scalar). Zeroized on drop.
#[derive(Clone)]
pub struct BlsSecretKey(pub Vec<u8>);

impl BlsSecretKey {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for BlsSecretKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for BlsSecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BlsSecretKey([REDACTED])")
    }
}

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
        ikm.zeroize();
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

    /// Generate a proof-of-possession: sign the public key with a distinct DST.
    /// This proves ownership of the secret key and prevents the rogue-key attack
    /// on BLS aggregate signatures. Each validator must submit their PoP when
    /// registering their BLS public key.
    pub fn proof_of_possession(&self) -> BlsSignature {
        let pk_bytes = self.pk.to_bytes();
        let sig = self.sk.sign(&pk_bytes, BLS_POP_DST, &[]);
        BlsSignature(sig.to_bytes().to_vec())
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

    /// Verify a proof-of-possession for a BLS public key.
    /// Returns true only if the PoP was produced by the holder of the
    /// secret key corresponding to `pk`. Uses BLS_POP_DST to prevent
    /// cross-domain replay.
    pub fn verify_proof_of_possession(pk: &BlsPublicKey, pop: &BlsSignature) -> bool {
        let pk_parsed = match BlstPublicKey::from_bytes(&pk.0) {
            Ok(pk) => pk,
            Err(_) => return false,
        };
        let sig = match BlstSignature::from_bytes(&pop.0) {
            Ok(sig) => sig,
            Err(_) => return false,
        };
        sig.verify(true, &pk.0, BLS_POP_DST, &[], &pk_parsed, true) == blst::BLST_ERROR::BLST_SUCCESS
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

    #[test]
    fn test_mldsa_keypair_layout_size_matches() {
        // Verify that the Keypair struct is exactly PK + SK bytes, no padding.
        // This is the assumption underlying from_bytes()'s unsafe block.
        assert_eq!(
            std::mem::size_of::<Keypair>(),
            pqc_dilithium::PUBLICKEYBYTES + pqc_dilithium::SECRETKEYBYTES,
            "Keypair size must equal PUBLICKEYBYTES + SECRETKEYBYTES — layout assumption violated"
        );
    }

    #[test]
    fn test_mldsa_from_bytes_rejects_wrong_pk_length() {
        let kp = MlDsaKeypair::generate();
        let sk = kp.secret_key().to_vec();
        let short_pk = vec![0u8; 100]; // too short
        assert!(MlDsaKeypair::from_bytes(&short_pk, &sk).is_err());
    }

    #[test]
    fn test_mldsa_from_bytes_rejects_wrong_sk_length() {
        let kp = MlDsaKeypair::generate();
        let pk = kp.public_key().to_vec();
        let short_sk = vec![0u8; 100]; // too short
        assert!(MlDsaKeypair::from_bytes(&pk, &short_sk).is_err());
    }

    #[test]
    fn test_mldsa_from_bytes_cross_verify_with_original() {
        // Generate keypair, extract bytes, reconstruct, and verify that
        // a signature from the reconstructed keypair verifies with the
        // original public key — proving functional equivalence.
        let kp1 = MlDsaKeypair::generate();
        let pk = kp1.public_key().to_vec();
        let sk = kp1.secret_key().to_vec();

        let kp2 = MlDsaKeypair::from_bytes(&pk, &sk).unwrap();

        // Sign with kp2, verify with kp1's public key
        let msg = b"cross-verify after reconstruction";
        let sig = kp2.sign(msg);
        assert!(MlDsaVerifier::verify(msg, &sig, &pk));

        // Sign with kp1, verify (kp2 should have same public key)
        let sig2 = kp1.sign(msg);
        assert!(MlDsaVerifier::verify(msg, &sig2, kp2.public_key()));
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

    // ─── Cross-validation: simulates WASM → Node flow ────────────────

    #[test]
    fn test_cross_validation_wasm_to_node() {
        // Simulate what the WASM bridge does: generate keypair, sign tx
        let wasm_kp = pqc_dilithium::Keypair::generate();
        let tx_bytes = b"transfer:from=0xabc,to=0xdef,amount=1000";
        let wasm_sig = wasm_kp.sign(tx_bytes);
        let wasm_pk = wasm_kp.public.to_vec();

        // Simulate what the node does: verify with MlDsaVerifier
        assert!(
            MlDsaVerifier::verify(tx_bytes, &wasm_sig, &wasm_pk),
            "Node must accept WASM-signed transactions"
        );
    }

    #[test]
    fn test_cross_validation_node_to_wasm() {
        // Simulate what the node does: generate keypair, sign
        let node_kp = MlDsaKeypair::generate();
        let msg = b"block proposal at height 42";
        let node_sig = node_kp.sign(msg);
        let node_pk = node_kp.public_key().to_vec();

        // Simulate what the WASM bridge does: verify with pqc_dilithium
        assert!(
            pqc_dilithium::verify(&node_sig, msg, &node_pk).is_ok(),
            "WASM must accept node-signed data"
        );
    }

    #[test]
    fn test_cross_validation_keypair_serialization() {
        // Generate in "WASM" (raw pqc_dilithium), reconstruct in "node" (MlDsaKeypair)
        let wasm_kp = pqc_dilithium::Keypair::generate();
        let pk = wasm_kp.public.to_vec();
        let sk = wasm_kp.expose_secret().to_vec();

        // Node reconstructs keypair from bytes sent by WASM
        let node_kp = MlDsaKeypair::from_bytes(&pk, &sk).unwrap();

        // Sign with reconstructed keypair, verify with raw pqc_dilithium
        let msg = b"cross-crate roundtrip";
        let sig = node_kp.sign(msg);
        assert!(pqc_dilithium::verify(&sig, msg, &pk).is_ok());
    }

    #[test]
    fn test_key_sizes_match_wasm_expectations() {
        let kp = MlDsaKeypair::generate();
        assert_eq!(kp.public_key().len(), 1952, "PK must be 1952 bytes");
        assert_eq!(kp.secret_key().len(), pqc_dilithium::SECRETKEYBYTES, "SK size must match");
        let sig = kp.sign(b"test");
        assert_eq!(sig.len(), 3293, "Signature must be 3293 bytes");
    }

    // ── BLS Proof-of-Possession Tests ──

    #[test]
    fn test_bls_pop_valid() {
        let kp = BlsKeypair::generate();
        let pop = kp.proof_of_possession();
        let pk = kp.public_key_bytes();
        assert!(BlsVerifier::verify_proof_of_possession(&pk, &pop));
    }

    #[test]
    fn test_bls_pop_wrong_key_rejects() {
        let kp1 = BlsKeypair::generate();
        let kp2 = BlsKeypair::generate();
        let pop = kp1.proof_of_possession();
        let pk2 = kp2.public_key_bytes();
        // PoP from kp1 must not verify against kp2's public key
        assert!(!BlsVerifier::verify_proof_of_possession(&pk2, &pop));
    }

    #[test]
    fn test_bls_pop_cannot_be_replayed_as_message_sig() {
        // PoP uses BLS_POP_DST, message signatures use BLS_DST.
        // A PoP must not verify as a message signature.
        let kp = BlsKeypair::generate();
        let pop = kp.proof_of_possession();
        let pk = kp.public_key_bytes();
        // Try to verify the PoP as if it were a signature over pk bytes
        assert!(!BlsVerifier::verify(&pk.0, &pop, &pk));
    }

    #[test]
    fn test_bls_pop_deterministic() {
        let sk_bytes = [42u8; 32];
        let kp1 = BlsKeypair::from_secret_bytes(&sk_bytes).unwrap();
        let kp2 = BlsKeypair::from_secret_bytes(&sk_bytes).unwrap();
        assert_eq!(kp1.proof_of_possession().0, kp2.proof_of_possession().0);
    }

    #[test]
    fn test_bls_pop_forged_bytes_rejected() {
        let kp = BlsKeypair::generate();
        let pk = kp.public_key_bytes();
        // Random 96 bytes should never pass as a valid PoP
        let fake_pop = BlsSignature(vec![0xDE; 96]);
        assert!(!BlsVerifier::verify_proof_of_possession(&pk, &fake_pop));
    }

    // ─── ECDSA secp256k1 Tests ─────────────────────────────────────────

    #[test]
    fn test_ecdsa_sign_verify_roundtrip() {
        let kp = EcdsaKeypair::generate();
        let msg = b"ecdsa signing test";
        let sig = kp.sign(msg);
        let pk = kp.public_key_compressed();
        assert!(EcdsaVerifier::verify(msg, &sig, &pk));
    }

    #[test]
    fn test_ecdsa_wrong_message_rejects() {
        let kp = EcdsaKeypair::generate();
        let sig = kp.sign(b"correct");
        let pk = kp.public_key_compressed();
        assert!(!EcdsaVerifier::verify(b"wrong", &sig, &pk));
    }

    #[test]
    fn test_ecdsa_wrong_key_rejects() {
        let kp1 = EcdsaKeypair::generate();
        let kp2 = EcdsaKeypair::generate();
        let sig = kp1.sign(b"hello");
        assert!(!EcdsaVerifier::verify(b"hello", &sig, &kp2.public_key_compressed()));
    }

    #[test]
    fn test_ecdsa_from_bytes_roundtrip() {
        let kp = EcdsaKeypair::generate();
        let sk = kp.secret_key_bytes();
        let kp2 = EcdsaKeypair::from_bytes(&sk).unwrap();
        let msg = b"roundtrip";
        let sig = kp2.sign(msg);
        assert!(EcdsaVerifier::verify(msg, &sig, &kp.public_key_compressed()));
    }

    #[test]
    fn test_ecdsa_key_sizes() {
        let kp = EcdsaKeypair::generate();
        assert_eq!(kp.public_key_compressed().len(), 33);
        assert_eq!(kp.secret_key_bytes().len(), 32);
        assert_eq!(kp.sign(b"test").len(), 64);
    }

    #[test]
    fn test_ecdsa_invalid_sig_length() {
        let kp = EcdsaKeypair::generate();
        let pk = kp.public_key_compressed();
        assert!(!EcdsaVerifier::verify(b"test", &[0u8; 10], &pk));
    }

    #[test]
    fn test_ecdsa_invalid_pk_length() {
        let kp = EcdsaKeypair::generate();
        let sig = kp.sign(b"test");
        assert!(!EcdsaVerifier::verify(b"test", &sig, &[0u8; 10]));
    }

    // ─── Hybrid (ECDSA + ML-DSA) Tests ─────────────────────────────────

    #[test]
    fn test_hybrid_sign_verify_roundtrip() {
        let kp = HybridKeypair::generate();
        let msg = b"hybrid post-quantum signature";
        let sig = kp.sign(msg);
        let pk = kp.public_key_bytes();
        assert!(HybridVerifier::verify(msg, &sig, &pk));
    }

    #[test]
    fn test_hybrid_wire_format_sizes() {
        let kp = HybridKeypair::generate();
        let pk = kp.public_key_bytes();
        let sig = kp.sign(b"size check");
        assert_eq!(pk.len(), HYBRID_PK_LEN);
        assert_eq!(sig.len(), HYBRID_SIG_LEN);
        assert_eq!(pk[0], HYBRID_TAG);
        assert_eq!(sig[0], HYBRID_TAG);
    }

    #[test]
    fn test_hybrid_wrong_message_rejects() {
        let kp = HybridKeypair::generate();
        let sig = kp.sign(b"correct");
        let pk = kp.public_key_bytes();
        assert!(!HybridVerifier::verify(b"wrong", &sig, &pk));
    }

    #[test]
    fn test_hybrid_wrong_key_rejects() {
        let kp1 = HybridKeypair::generate();
        let kp2 = HybridKeypair::generate();
        let sig = kp1.sign(b"hello");
        assert!(!HybridVerifier::verify(b"hello", &sig, &kp2.public_key_bytes()));
    }

    #[test]
    fn test_hybrid_tampered_ecdsa_sig_rejects() {
        let kp = HybridKeypair::generate();
        let msg = b"tamper test";
        let mut sig = kp.sign(msg);
        sig[1] ^= 0xFF; // flip byte in ECDSA portion
        assert!(!HybridVerifier::verify(msg, &sig, &kp.public_key_bytes()));
    }

    #[test]
    fn test_hybrid_tampered_mldsa_sig_rejects() {
        let kp = HybridKeypair::generate();
        let msg = b"tamper test";
        let mut sig = kp.sign(msg);
        sig[1 + ECDSA_SIGNATURE_LEN + 10] ^= 0xFF; // flip byte in ML-DSA portion
        assert!(!HybridVerifier::verify(msg, &sig, &kp.public_key_bytes()));
    }

    #[test]
    fn test_hybrid_verifier_accepts_legacy_mldsa() {
        let kp = MlDsaKeypair::generate();
        let msg = b"legacy transaction";
        let sig = kp.sign(msg);
        let pk = kp.public_key_bytes();
        assert!(HybridVerifier::verify(msg, &sig, &pk));
    }

    #[test]
    fn test_hybrid_verifier_rejects_mixed_scheme() {
        let hybrid_kp = HybridKeypair::generate();
        let mldsa_kp = MlDsaKeypair::generate();
        let msg = b"mixed";
        let hybrid_sig = hybrid_kp.sign(msg);
        let mldsa_pk = mldsa_kp.public_key_bytes();
        assert!(!HybridVerifier::verify(msg, &hybrid_sig, &mldsa_pk));
    }

    #[test]
    fn test_hybrid_split_pk_valid() {
        let kp = HybridKeypair::generate();
        let pk = kp.public_key_bytes();
        let (ecdsa_pk, mldsa_pk) = HybridVerifier::split_pk(&pk).unwrap();
        assert_eq!(ecdsa_pk.len(), ECDSA_COMPRESSED_PK_LEN);
        assert_eq!(mldsa_pk.len(), pqc_dilithium::PUBLICKEYBYTES);
    }

    #[test]
    fn test_hybrid_split_sig_valid() {
        let kp = HybridKeypair::generate();
        let sig = kp.sign(b"split");
        let (ecdsa_sig, mldsa_sig) = HybridVerifier::split_sig(&sig).unwrap();
        assert_eq!(ecdsa_sig.len(), ECDSA_SIGNATURE_LEN);
        assert_eq!(mldsa_sig.len(), pqc_dilithium::SIGNBYTES);
    }

    #[test]
    fn test_hybrid_each_component_verifies_independently() {
        let kp = HybridKeypair::generate();
        let msg = b"independent verification";
        let sig = kp.sign(msg);
        let pk = kp.public_key_bytes();
        let (ecdsa_pk, mldsa_pk) = HybridVerifier::split_pk(&pk).unwrap();
        let (ecdsa_sig, mldsa_sig) = HybridVerifier::split_sig(&sig).unwrap();
        assert!(EcdsaVerifier::verify(msg, ecdsa_sig, ecdsa_pk));
        assert!(MlDsaVerifier::verify(msg, mldsa_sig, mldsa_pk));
    }

    #[test]
    fn test_hybrid_from_parts() {
        let ecdsa = EcdsaKeypair::generate();
        let mldsa = MlDsaKeypair::generate();
        let kp = HybridKeypair::from_parts(ecdsa, mldsa);
        let msg = b"from parts";
        let sig = kp.sign(msg);
        assert!(HybridVerifier::verify(msg, &sig, &kp.public_key_bytes()));
    }
}
