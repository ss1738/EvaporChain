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
}

/// Create a BLS-signed attestation for DA verification.
pub fn create_attestation(
    block_number: u64,
    data_root: &[u8; 32],
    validator_id: u64,
    samples_verified: u32,
    stake: u64,
    keypair: &BlsKeypair,
) -> DAAttestation {
    // Build the message to sign
    let mut msg = Vec::with_capacity(8 + 32 + 8 + 4);
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

        // Reconstruct the signed message and verify the BLS signature
        let mut msg = Vec::with_capacity(8 + 32 + 8 + 4);
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
}
