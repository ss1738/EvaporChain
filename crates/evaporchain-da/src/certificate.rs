//! DA certificate: aggregated validator attestations proving data availability.
//!
//! Validators sample random cells from the 2D erasure-coded matrix, verify proofs,
//! and sign attestations. When a supermajority of stake attests, a DA certificate
//! is produced and included in the block.

use evaporchain_crypto::signatures::{BlsKeypair, BlsPublicKey, BlsSignature, BlsVerifier};
use serde::{Deserialize, Serialize};

/// A single validator's attestation that they verified DA samples.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DAAttestation {
    /// Block number attested to.
    pub block_number: u64,
    /// Data root the attestation is for.
    pub data_root: [u8; 32],
    /// Validator ID.
    pub validator_id: u64,
    /// Number of cells sampled and verified.
    pub samples_verified: u32,
    /// Validator's stake weight.
    pub stake: u64,
    /// BLS signature over (block_number || data_root || validator_id || samples_verified).
    pub signature: Vec<u8>,
    /// BLS public key of the signer.
    pub public_key: Vec<u8>,
}

/// DA certificate: proof that a supermajority of validators verified data availability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DACertificate {
    /// Block number this certificate covers.
    pub block_number: u64,
    /// Data root attested to.
    pub data_root: [u8; 32],
    /// Individual attestations.
    pub attestations: Vec<DAAttestation>,
    /// Total stake that attested.
    pub attested_stake: u64,
    /// Total stake in the validator set.
    pub total_stake: u64,
}

impl DACertificate {
    /// Check if attested stake >= 2/3 of total stake (supermajority).
    pub fn is_supermajority(&self) -> bool {
        // 2/3 threshold: attested_stake * 3 >= total_stake * 2
        self.attested_stake * 3 >= self.total_stake * 2
    }

    /// Verify every attestation's BLS signature and check that `attested_stake`
    /// matches the sum of individual attestation stakes.
    ///
    /// This MUST be called on any `DACertificate` received from the network
    /// (deserialized) to prevent forged-signature attacks.
    pub fn verify_signatures(&self) -> bool {
        if self.attestations.is_empty() {
            return false;
        }

        let mut recomputed_stake: u64 = 0;

        for att in &self.attestations {
            // Each attestation must match the certificate's block/data_root
            if att.block_number != self.block_number || att.data_root != self.data_root {
                return false;
            }

            // Reconstruct the signed message — must match create_attestation exactly.
            let mut msg = Vec::with_capacity(DA_ATTESTATION_DST.len() + 8 + 32 + 8 + 4);
            msg.extend_from_slice(DA_ATTESTATION_DST);
            msg.extend_from_slice(&att.block_number.to_le_bytes());
            msg.extend_from_slice(&att.data_root);
            msg.extend_from_slice(&att.validator_id.to_le_bytes());
            msg.extend_from_slice(&att.samples_verified.to_le_bytes());

            let pk = BlsPublicKey(att.public_key.clone());
            let sig = BlsSignature(att.signature.clone());
            if !BlsVerifier::verify(&msg, &sig, &pk) {
                return false;
            }

            recomputed_stake = recomputed_stake.saturating_add(att.stake);
        }

        // Guard against inflated attested_stake: the certificate's claimed
        // attested_stake must not exceed the sum of individual attestation stakes.
        if self.attested_stake > recomputed_stake {
            return false;
        }

        true
    }

    /// Full validation: verify BLS signatures AND supermajority threshold.
    /// Use this as the single entry point for validating received certificates.
    pub fn verify_all(&self) -> bool {
        self.verify_signatures() && self.is_supermajority()
    }
}

/// Domain-separation tag for DA attestation signatures.
///
/// Prepended to every signed message so a DA attestation BLS signature
/// cannot be replayed as a consensus vote or oracle report.
pub const DA_ATTESTATION_DST: &[u8] = b"evaporchain:da-attestation:v1:";

/// Create a BLS-signed attestation for DA verification.
pub fn create_attestation(
    block_number: u64,
    data_root: &[u8; 32],
    validator_id: u64,
    samples_verified: u32,
    stake: u64,
    keypair: &BlsKeypair,
) -> DAAttestation {
    // Build the message to sign — DST prefix ensures cross-context separation.
    let mut msg = Vec::with_capacity(DA_ATTESTATION_DST.len() + 8 + 32 + 8 + 4);
    msg.extend_from_slice(DA_ATTESTATION_DST);
    msg.extend_from_slice(&block_number.to_le_bytes());
    msg.extend_from_slice(data_root);
    msg.extend_from_slice(&validator_id.to_le_bytes());
    msg.extend_from_slice(&samples_verified.to_le_bytes());

    let sig = keypair.sign(&msg);
    let pk = keypair.public_key_bytes();

    DAAttestation {
        block_number,
        data_root: *data_root,
        validator_id,
        samples_verified,
        stake,
        signature: sig.0,
        public_key: pk.0,
    }
}

/// Builder for constructing a DA certificate from individual attestations.
pub struct CertificateBuilder {
    block_number: u64,
    data_root: [u8; 32],
    total_stake: u64,
    attestations: Vec<DAAttestation>,
    attested_stake: u64,
}

impl CertificateBuilder {
    /// Create a new builder for the given block.
    pub fn new(block_number: u64, data_root: [u8; 32], total_stake: u64) -> Self {
        Self {
            block_number,
            data_root,
            total_stake,
            attestations: Vec::new(),
            attested_stake: 0,
        }
    }

    /// Add a validator attestation. Returns false and rejects the attestation if
    /// the BLS signature is invalid or the attestation is for a different block/data_root.
    pub fn add_attestation(&mut self, att: DAAttestation) -> bool {
        if att.block_number != self.block_number || att.data_root != self.data_root {
            return false;
        }

        // Reconstruct the signed message and verify the BLS signature.
        // MUST mirror create_attestation byte-for-byte, including the
        // DA_ATTESTATION_DST prefix — without it, every attestation
        // produced by create_attestation fails verification.
        let mut msg = Vec::with_capacity(DA_ATTESTATION_DST.len() + 8 + 32 + 8 + 4);
        msg.extend_from_slice(DA_ATTESTATION_DST);
        msg.extend_from_slice(&att.block_number.to_le_bytes());
        msg.extend_from_slice(&att.data_root);
        msg.extend_from_slice(&att.validator_id.to_le_bytes());
        msg.extend_from_slice(&att.samples_verified.to_le_bytes());

        let pk = BlsPublicKey(att.public_key.clone());
        let sig = BlsSignature(att.signature.clone());
        if !BlsVerifier::verify(&msg, &sig, &pk) {
            return false;
        }

        self.attested_stake += att.stake;
        self.attestations.push(att);
        true
    }

    /// Try to build the certificate. Returns Some if supermajority is reached.
    pub fn try_build(self) -> Option<DACertificate> {
        let cert = DACertificate {
            block_number: self.block_number,
            data_root: self.data_root,
            attestations: self.attestations,
            attested_stake: self.attested_stake,
            total_stake: self.total_stake,
        };
        if cert.is_supermajority() {
            Some(cert)
        } else {
            None
        }
    }

    /// Current attested stake.
    pub fn attested_stake(&self) -> u64 {
        self.attested_stake
    }

    /// Check if we already have supermajority.
    pub fn has_supermajority(&self) -> bool {
        self.attested_stake * 3 >= self.total_stake * 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_certificate_supermajority() {
        let data_root = [0xABu8; 32];
        let mut builder = CertificateBuilder::new(1, data_root, 4000);

        for vid in 1..=3 {
            let kp = BlsKeypair::generate();
            let att = create_attestation(1, &data_root, vid, 8, 1000, &kp);
            assert!(builder.add_attestation(att));
        }

        // 3000/4000 = 75% >= 66.7%
        assert!(builder.has_supermajority());
        let cert = builder.try_build().unwrap();
        assert!(cert.is_supermajority());
        assert_eq!(cert.attestations.len(), 3);
    }

    #[test]
    fn test_certificate_no_supermajority() {
        let data_root = [0xCDu8; 32];
        let mut builder = CertificateBuilder::new(1, data_root, 4000);

        let kp = BlsKeypair::generate();
        let att = create_attestation(1, &data_root, 1, 8, 1000, &kp);
        assert!(builder.add_attestation(att));

        // 1000/4000 = 25% < 66.7%
        assert!(!builder.has_supermajority());
        assert!(builder.try_build().is_none());
    }

    #[test]
    fn test_certificate_rejects_forged_signature() {
        let data_root = [0xEEu8; 32];
        let mut builder = CertificateBuilder::new(1, data_root, 4000);

        let kp = BlsKeypair::generate();
        let mut att = create_attestation(1, &data_root, 1, 8, 1000, &kp);
        att.signature = vec![0xFF; 96]; // forged signature
        assert!(!builder.add_attestation(att));
        assert_eq!(builder.attested_stake(), 0);
    }

    #[test]
    fn test_certificate_rejects_wrong_block() {
        let data_root = [0xAAu8; 32];
        let mut builder = CertificateBuilder::new(1, data_root, 4000);

        let kp = BlsKeypair::generate();
        let att = create_attestation(99, &data_root, 1, 8, 1000, &kp); // wrong block_number
        assert!(!builder.add_attestation(att));
        assert_eq!(builder.attested_stake(), 0);
    }

    // ─── C-09: verify_signatures / verify_all tests ───────────────────

    /// Helper: build a valid certificate with `n` validators.
    fn build_valid_cert(n: usize) -> DACertificate {
        let data_root = [0xBBu8; 32];
        let stake_per = 1000u64;
        let total_stake = (n as u64) * stake_per;
        let mut builder = CertificateBuilder::new(1, data_root, total_stake);

        for vid in 1..=(n as u64) {
            let kp = BlsKeypair::generate();
            let att = create_attestation(1, &data_root, vid, 8, stake_per, &kp);
            assert!(builder.add_attestation(att));
        }
        builder.try_build().unwrap()
    }

    #[test]
    fn test_verify_signatures_valid_cert() {
        let cert = build_valid_cert(3);
        assert!(cert.verify_signatures());
        assert!(cert.verify_all());
    }

    #[test]
    fn test_verify_signatures_rejects_forged_sig() {
        let mut cert = build_valid_cert(3);
        // Corrupt one attestation's signature
        cert.attestations[1].signature = vec![0xFF; 96];
        assert!(!cert.verify_signatures());
        assert!(!cert.verify_all());
    }

    #[test]
    fn test_verify_signatures_rejects_wrong_pubkey() {
        let mut cert = build_valid_cert(3);
        // Replace public key with a different validator's key
        let other_kp = BlsKeypair::generate();
        cert.attestations[0].public_key = other_kp.public_key_bytes().0;
        assert!(!cert.verify_signatures());
    }

    #[test]
    fn test_verify_signatures_rejects_empty_attestations() {
        let cert = DACertificate {
            block_number: 1,
            data_root: [0xBB; 32],
            attestations: vec![],
            attested_stake: 3000,
            total_stake: 3000,
        };
        assert!(!cert.verify_signatures());
    }

    #[test]
    fn test_verify_signatures_rejects_inflated_attested_stake() {
        let mut cert = build_valid_cert(3);
        // Inflate the claimed attested_stake beyond what attestations sum to
        cert.attested_stake += 999_999;
        assert!(!cert.verify_signatures());
    }

    #[test]
    fn test_verify_all_rejects_valid_sigs_but_no_supermajority() {
        let data_root = [0xDDu8; 32];
        let kp = BlsKeypair::generate();
        let att = create_attestation(1, &data_root, 1, 8, 1000, &kp);

        // Manually construct a certificate without supermajority
        let cert = DACertificate {
            block_number: 1,
            data_root,
            attestations: vec![att],
            attested_stake: 1000,
            total_stake: 10_000, // 10% < 66.7%
        };
        assert!(cert.verify_signatures()); // sigs valid
        assert!(!cert.is_supermajority()); // but not enough stake
        assert!(!cert.verify_all()); // full validation fails
    }

    #[test]
    fn test_verify_signatures_rejects_attestation_wrong_block() {
        let mut cert = build_valid_cert(3);
        // Tamper: change an attestation's block_number so it mismatches the cert
        cert.attestations[2].block_number = 999;
        assert!(!cert.verify_signatures());
    }

    #[test]
    fn test_verify_signatures_rejects_attestation_wrong_data_root() {
        let mut cert = build_valid_cert(3);
        // Tamper: change an attestation's data_root so it mismatches the cert
        cert.attestations[0].data_root = [0xFF; 32];
        assert!(!cert.verify_signatures());
    }

    #[test]
    fn test_forged_certificate_from_scratch_is_rejected() {
        // Simulate an attacker crafting a certificate entirely from scratch
        // with fabricated signatures and inflated stake
        let forged_cert = DACertificate {
            block_number: 100,
            data_root: [0xAA; 32],
            attestations: vec![
                DAAttestation {
                    block_number: 100,
                    data_root: [0xAA; 32],
                    validator_id: 1,
                    samples_verified: 16,
                    stake: 50_000,
                    signature: vec![0x42; 96],  // garbage
                    public_key: vec![0x13; 48], // garbage
                },
                DAAttestation {
                    block_number: 100,
                    data_root: [0xAA; 32],
                    validator_id: 2,
                    samples_verified: 16,
                    stake: 50_000,
                    signature: vec![0x43; 96],  // garbage
                    public_key: vec![0x14; 48], // garbage
                },
            ],
            attested_stake: 100_000,
            total_stake: 100_000,
        };
        // Without verify_signatures this would pass is_supermajority()
        assert!(forged_cert.is_supermajority());
        // But full validation catches the forged signatures
        assert!(!forged_cert.verify_signatures());
        assert!(!forged_cert.verify_all());
    }
}
