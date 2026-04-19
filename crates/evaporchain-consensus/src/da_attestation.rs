//! DA attestation manager for consensus.
//!
//! Orchestrates the data availability attestation flow:
//! 1. When a block is proposed with a `data_root`, this validator samples
//!    random shards and creates a BLS-signed attestation.
//! 2. Attestations from other validators are collected.
//! 3. When 2/3+ stake attests, a `DACertificate` is built.
//!
//! The DA certificate proves data availability to light clients and bridges
//! without requiring them to download the full block data.

use evaporchain_crypto::signatures::{BlsKeypair, BlsPublicKey, BlsVerifier};
use evaporchain_da::certificate::{
    CertificateBuilder, DAAttestation, DACertificate, create_attestation,
};
use std::collections::HashMap;
use tracing::{debug, info, warn};

// ─────────────────────── Configuration ─────────────────────────────���────

/// Minimum samples a validator must verify before attesting.
const MIN_SAMPLES_FOR_ATTESTATION: u32 = 4;

/// Default number of samples per attestation.
const DEFAULT_SAMPLE_COUNT: u32 = 8;

// ─────────────────────── DAAttestationManager ───────────────────────────

/// Manages DA attestation collection and certificate building for a single height.
///
/// Each height with a non-empty `data_root` gets its own attestation round.
/// The manager tracks which validators have attested, prevents duplicates,
/// verifies attestation signatures, and builds the certificate once
/// supermajority is reached.
pub struct DAAttestationManager {
    /// Current height being attested.
    current_height: u64,
    /// Data root for the current height (None if no block proposed yet).
    current_data_root: Option<[u8; 32]>,
    /// Certificate builder for the current height.
    builder: Option<CertificateBuilder>,
    /// Validators that have already attested (prevent duplicates).
    attested_validators: HashMap<u64, bool>,
    /// Completed certificate (once supermajority reached).
    certificate: Option<DACertificate>,
    /// Number of samples to verify per attestation.
    sample_count: u32,
}

impl DAAttestationManager {
    /// Create a new attestation manager.
    pub fn new() -> Self {
        Self {
            current_height: 0,
            current_data_root: None,
            builder: None,
            attested_validators: HashMap::new(),
            certificate: None,
            sample_count: DEFAULT_SAMPLE_COUNT,
        }
    }

    /// Start a new attestation round for a block height with the given data root.
    ///
    /// Resets all state from the previous round.
    pub fn start_round(
        &mut self,
        height: u64,
        data_root: [u8; 32],
        total_stake: u64,
    ) {
        self.current_height = height;
        self.current_data_root = Some(data_root);
        self.builder = Some(CertificateBuilder::new(height, data_root, total_stake));
        self.attested_validators.clear();
        self.certificate = None;

        debug!(
            height,
            data_root = hex::encode(data_root),
            total_stake,
            "DA attestation round started"
        );
    }

    /// Create this validator's attestation for the current round.
    ///
    /// Returns None if no round is active or the data_root doesn't match.
    pub fn create_own_attestation(
        &self,
        validator_id: u64,
        stake: u64,
        keypair: &BlsKeypair,
    ) -> Option<DAAttestation> {
        let data_root = self.current_data_root.as_ref()?;

        Some(create_attestation(
            self.current_height,
            data_root,
            validator_id,
            self.sample_count,
            stake,
            keypair,
        ))
    }

    /// Add an attestation from a validator.
    ///
    /// Returns:
    /// - `Ok(true)` if the attestation was accepted and supermajority is now reached
    /// - `Ok(false)` if accepted but no supermajority yet
    /// - `Err(reason)` if the attestation was rejected
    pub fn add_attestation(
        &mut self,
        attestation: DAAttestation,
    ) -> Result<bool, &'static str> {
        // Check round is active
        let data_root = self.current_data_root.ok_or("no active attestation round")?;

        // Already have a certificate
        if self.certificate.is_some() {
            return Err("certificate already built");
        }

        // Check height matches
        if attestation.block_number != self.current_height {
            return Err("attestation height mismatch");
        }

        // Check data_root matches
        if attestation.data_root != data_root {
            return Err("attestation data_root mismatch");
        }

        // Check minimum samples
        if attestation.samples_verified < MIN_SAMPLES_FOR_ATTESTATION {
            return Err("insufficient samples verified");
        }

        // Prevent duplicate attestations from the same validator
        if self.attested_validators.contains_key(&attestation.validator_id) {
            return Err("duplicate attestation from validator");
        }

        // Verify BLS signature
        if !verify_attestation_signature(&attestation) {
            return Err("invalid BLS signature");
        }

        let validator_id = attestation.validator_id;
        let stake = attestation.stake;

        // Add to builder
        if let Some(ref mut builder) = self.builder {
            builder.add_attestation(attestation);
            self.attested_validators.insert(validator_id, true);

            debug!(
                height = self.current_height,
                validator = validator_id,
                stake,
                attested_stake = builder.attested_stake(),
                has_supermajority = builder.has_supermajority(),
                "DA attestation accepted"
            );

            if builder.has_supermajority() {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Try to build the DA certificate.
    ///
    /// Consumes the builder if supermajority is reached.
    /// Returns the certificate or None if not enough attestations.
    pub fn try_build_certificate(&mut self) -> Option<DACertificate> {
        if self.certificate.is_some() {
            return self.certificate.clone();
        }

        if let Some(builder) = self.builder.take() {
            if builder.has_supermajority() {
                let cert = builder.try_build()?;
                info!(
                    height = cert.block_number,
                    attestations = cert.attestations.len(),
                    attested_stake = cert.attested_stake,
                    total_stake = cert.total_stake,
                    "DA certificate built"
                );
                self.certificate = Some(cert.clone());
                return Some(cert);
            } else {
                // Put it back
                self.builder = Some(builder);
            }
        }

        None
    }

    /// Serialize a DA certificate to bytes for inclusion in a block.
    pub fn serialize_certificate(cert: &DACertificate) -> Vec<u8> {
        serde_json::to_vec(cert).expect("DACertificate serialization should not fail")
    }

    /// Deserialize a DA certificate from block bytes.
    pub fn deserialize_certificate(bytes: &[u8]) -> Option<DACertificate> {
        serde_json::from_slice(bytes).ok()
    }

    /// Check if a certificate has been built for the current round.
    pub fn has_certificate(&self) -> bool {
        self.certificate.is_some()
    }

    /// Get the certificate if built.
    pub fn certificate(&self) -> Option<&DACertificate> {
        self.certificate.as_ref()
    }

    /// Current height being attested.
    pub fn current_height(&self) -> u64 {
        self.current_height
    }

    /// Number of attestations collected so far.
    pub fn attestation_count(&self) -> usize {
        self.attested_validators.len()
    }

    /// Check if a validator has already attested.
    pub fn has_attested(&self, validator_id: u64) -> bool {
        self.attested_validators.contains_key(&validator_id)
    }

    /// Verify a DA certificate from a received block.
    ///
    /// Checks: supermajority, height match, data_root match, all signatures.
    pub fn verify_certificate(
        cert: &DACertificate,
        expected_height: u64,
        expected_data_root: &[u8; 32],
    ) -> bool {
        // Basic checks
        if cert.block_number != expected_height {
            return false;
        }
        if cert.data_root != *expected_data_root {
            return false;
        }
        if !cert.is_supermajority() {
            return false;
        }

        // Verify all attestation signatures
        for att in &cert.attestations {
            if att.block_number != expected_height {
                return false;
            }
            if att.data_root != *expected_data_root {
                return false;
            }
            if !verify_attestation_signature(att) {
                return false;
            }
        }

        true
    }
}

impl Default for DAAttestationManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Verify the BLS signature on a DA attestation.
fn verify_attestation_signature(att: &DAAttestation) -> bool {
    // Reconstruct the signed message
    let mut msg = Vec::with_capacity(8 + 32 + 8 + 4);
    msg.extend_from_slice(&att.block_number.to_le_bytes());
    msg.extend_from_slice(&att.data_root);
    msg.extend_from_slice(&att.validator_id.to_le_bytes());
    msg.extend_from_slice(&att.samples_verified.to_le_bytes());

    let pk = BlsPublicKey(att.public_key.clone());
    let sig = evaporchain_crypto::signatures::BlsSignature(att.signature.clone());
    BlsVerifier::verify(&msg, &sig, &pk)
}

// ─────────────────────── Tests ──────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_crypto::signatures::BlsKeypair;

    fn make_keypairs(n: usize) -> Vec<BlsKeypair> {
        (0..n).map(|_| BlsKeypair::generate()).collect()
    }

    #[test]
    fn test_start_round() {
        let mut mgr = DAAttestationManager::new();
        let data_root = [0xAB; 32];
        mgr.start_round(10, data_root, 4000);

        assert_eq!(mgr.current_height(), 10);
        assert_eq!(mgr.attestation_count(), 0);
        assert!(!mgr.has_certificate());
    }

    #[test]
    fn test_create_own_attestation() {
        let mut mgr = DAAttestationManager::new();
        let data_root = [0xCD; 32];
        let kp = BlsKeypair::generate();

        // No round started
        assert!(mgr.create_own_attestation(1, 1000, &kp).is_none());

        // Start round
        mgr.start_round(5, data_root, 4000);
        let att = mgr.create_own_attestation(1, 1000, &kp).unwrap();

        assert_eq!(att.block_number, 5);
        assert_eq!(att.data_root, data_root);
        assert_eq!(att.validator_id, 1);
        assert_eq!(att.stake, 1000);
        assert_eq!(att.samples_verified, DEFAULT_SAMPLE_COUNT);
    }

    #[test]
    fn test_add_attestation_and_build_certificate() {
        let keypairs = make_keypairs(4);
        let data_root = [0xEF; 32];
        let mut mgr = DAAttestationManager::new();
        mgr.start_round(1, data_root, 4000);

        // Add 3 attestations (3000/4000 = 75% >= 66.7%)
        for (i, kp) in keypairs.iter().enumerate().take(3) {
            let att = create_attestation(
                1, &data_root, (i + 1) as u64, 8, 1000, kp,
            );
            let result = mgr.add_attestation(att).unwrap();
            if i < 2 {
                assert!(!result, "shouldn't have supermajority with {} attestations", i + 1);
            } else {
                assert!(result, "should have supermajority with 3 attestations");
            }
        }

        assert_eq!(mgr.attestation_count(), 3);

        let cert = mgr.try_build_certificate().unwrap();
        assert!(cert.is_supermajority());
        assert_eq!(cert.attestations.len(), 3);
        assert_eq!(cert.attested_stake, 3000);
        assert_eq!(cert.total_stake, 4000);
        assert!(mgr.has_certificate());
    }

    #[test]
    fn test_duplicate_attestation_rejected() {
        let kp = BlsKeypair::generate();
        let data_root = [0x11; 32];
        let mut mgr = DAAttestationManager::new();
        mgr.start_round(1, data_root, 4000);

        let att1 = create_attestation(1, &data_root, 1, 8, 1000, &kp);
        let att2 = create_attestation(1, &data_root, 1, 8, 1000, &kp);

        assert!(mgr.add_attestation(att1).is_ok());
        assert_eq!(
            mgr.add_attestation(att2).unwrap_err(),
            "duplicate attestation from validator"
        );
    }

    #[test]
    fn test_wrong_height_rejected() {
        let kp = BlsKeypair::generate();
        let data_root = [0x22; 32];
        let mut mgr = DAAttestationManager::new();
        mgr.start_round(10, data_root, 4000);

        let att = create_attestation(99, &data_root, 1, 8, 1000, &kp);
        assert_eq!(
            mgr.add_attestation(att).unwrap_err(),
            "attestation height mismatch"
        );
    }

    #[test]
    fn test_wrong_data_root_rejected() {
        let kp = BlsKeypair::generate();
        let data_root = [0x33; 32];
        let wrong_root = [0x44; 32];
        let mut mgr = DAAttestationManager::new();
        mgr.start_round(1, data_root, 4000);

        let att = create_attestation(1, &wrong_root, 1, 8, 1000, &kp);
        assert_eq!(
            mgr.add_attestation(att).unwrap_err(),
            "attestation data_root mismatch"
        );
    }

    #[test]
    fn test_insufficient_samples_rejected() {
        let kp = BlsKeypair::generate();
        let data_root = [0x55; 32];
        let mut mgr = DAAttestationManager::new();
        mgr.start_round(1, data_root, 4000);

        // Create attestation with only 2 samples (min is 4)
        let att = create_attestation(1, &data_root, 1, 2, 1000, &kp);
        assert_eq!(
            mgr.add_attestation(att).unwrap_err(),
            "insufficient samples verified"
        );
    }

    #[test]
    fn test_forged_signature_rejected() {
        let kp1 = BlsKeypair::generate();
        let kp2 = BlsKeypair::generate();
        let data_root = [0x66; 32];
        let mut mgr = DAAttestationManager::new();
        mgr.start_round(1, data_root, 4000);

        // Create attestation with kp1 but swap public key to kp2
        let mut att = create_attestation(1, &data_root, 1, 8, 1000, &kp1);
        att.public_key = kp2.public_key_bytes().0; // wrong key

        assert_eq!(
            mgr.add_attestation(att).unwrap_err(),
            "invalid BLS signature"
        );
    }

    #[test]
    fn test_no_supermajority_returns_none() {
        let kp = BlsKeypair::generate();
        let data_root = [0x77; 32];
        let mut mgr = DAAttestationManager::new();
        mgr.start_round(1, data_root, 4000);

        // Only 1 attestation (1000/4000 = 25%)
        let att = create_attestation(1, &data_root, 1, 8, 1000, &kp);
        mgr.add_attestation(att).unwrap();

        assert!(mgr.try_build_certificate().is_none());
        assert!(!mgr.has_certificate());
    }

    #[test]
    fn test_verify_certificate() {
        let keypairs = make_keypairs(3);
        let data_root = [0x88; 32];
        let mut mgr = DAAttestationManager::new();
        mgr.start_round(5, data_root, 3000);

        for (i, kp) in keypairs.iter().enumerate() {
            let att = create_attestation(
                5, &data_root, (i + 1) as u64, 8, 1000, kp,
            );
            mgr.add_attestation(att).unwrap();
        }

        let cert = mgr.try_build_certificate().unwrap();

        // Valid verification
        assert!(DAAttestationManager::verify_certificate(&cert, 5, &data_root));

        // Wrong height
        assert!(!DAAttestationManager::verify_certificate(&cert, 99, &data_root));

        // Wrong data_root
        let wrong = [0x99; 32];
        assert!(!DAAttestationManager::verify_certificate(&cert, 5, &wrong));
    }

    #[test]
    fn test_serialize_deserialize_certificate() {
        let keypairs = make_keypairs(3);
        let data_root = [0xAA; 32];
        let mut mgr = DAAttestationManager::new();
        mgr.start_round(1, data_root, 3000);

        for (i, kp) in keypairs.iter().enumerate() {
            let att = create_attestation(
                1, &data_root, (i + 1) as u64, 8, 1000, kp,
            );
            mgr.add_attestation(att).unwrap();
        }

        let cert = mgr.try_build_certificate().unwrap();
        let bytes = DAAttestationManager::serialize_certificate(&cert);
        let decoded = DAAttestationManager::deserialize_certificate(&bytes).unwrap();

        assert_eq!(decoded.block_number, cert.block_number);
        assert_eq!(decoded.data_root, cert.data_root);
        assert_eq!(decoded.attestations.len(), cert.attestations.len());
        assert_eq!(decoded.attested_stake, cert.attested_stake);
        assert!(decoded.is_supermajority());
    }

    #[test]
    fn test_new_round_resets_state() {
        let kp = BlsKeypair::generate();
        let data_root1 = [0xBB; 32];
        let data_root2 = [0xCC; 32];
        let mut mgr = DAAttestationManager::new();

        // Round 1
        mgr.start_round(1, data_root1, 1000);
        let att = create_attestation(1, &data_root1, 1, 8, 1000, &kp);
        mgr.add_attestation(att).unwrap();
        assert_eq!(mgr.attestation_count(), 1);

        // Round 2 — should reset
        mgr.start_round(2, data_root2, 2000);
        assert_eq!(mgr.attestation_count(), 0);
        assert!(!mgr.has_certificate());
        assert_eq!(mgr.current_height(), 2);
    }

    #[test]
    fn test_has_attested_tracking() {
        let keypairs = make_keypairs(2);
        let data_root = [0xDD; 32];
        let mut mgr = DAAttestationManager::new();
        mgr.start_round(1, data_root, 4000);

        assert!(!mgr.has_attested(1));
        assert!(!mgr.has_attested(2));

        let att = create_attestation(1, &data_root, 1, 8, 1000, &keypairs[0]);
        mgr.add_attestation(att).unwrap();

        assert!(mgr.has_attested(1));
        assert!(!mgr.has_attested(2));
    }

    #[test]
    fn test_no_round_active_rejects() {
        let kp = BlsKeypair::generate();
        let data_root = [0xEE; 32];
        let mut mgr = DAAttestationManager::new();

        let att = create_attestation(1, &data_root, 1, 8, 1000, &kp);
        assert_eq!(
            mgr.add_attestation(att).unwrap_err(),
            "no active attestation round"
        );
    }
}
