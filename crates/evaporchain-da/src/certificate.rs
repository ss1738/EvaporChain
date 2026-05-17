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
    /// BLS signature over (block_number || data_root || validator_id || samples_verified || stake).
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
    /// Check if attested stake is STRICTLY more than 2/3 of total stake.
    ///
    /// Q4 (audit 2026-05-17): Tendermint safety requires strict `> 2T/3`.
    /// Pre-fix used `>= ceil(2T/3)` which collapses to exactly `2T/3` when
    /// total_stake is divisible by 3. A Byzantine actor holding stake
    /// exactly T/3 could combine with two distinct honest T/3 sets to
    /// produce two disjoint quorums for different blocks at the same
    /// height (Agreement violation). Matches `finality.rs::has_supermajority`.
    pub fn is_supermajority(&self) -> bool {
        // Audit C2: cast to u128 before multiplication to prevent wrap when stake > u64::MAX/3.
        (self.attested_stake as u128) * 3 > (self.total_stake as u128) * 2
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

        // Q2 (audit 2026-05-17): dedup by validator_id — a cert carrying N
        // copies of the same attestation would otherwise count each copy's
        // stake separately, allowing a single-key forgery to amplify to 100×
        // stake by replaying one valid attestation 100 times.
        let mut seen_validators = std::collections::HashSet::<u64>::new();
        let mut recomputed_stake: u64 = 0;

        for att in &self.attestations {
            // Each attestation must match the certificate's block/data_root
            if att.block_number != self.block_number || att.data_root != self.data_root {
                return false;
            }

            // Q2: reject duplicate signer.
            if !seen_validators.insert(att.validator_id) {
                return false;
            }

            // Reconstruct the signed message — must match create_attestation exactly.
            // Q3 (audit 2026-05-17): stake is now included in the signed
            // message so an adversary cannot inflate att.stake post-signing.
            let mut msg = Vec::with_capacity(DA_ATTESTATION_DST.len() + 8 + 32 + 8 + 4 + 8);
            msg.extend_from_slice(DA_ATTESTATION_DST);
            msg.extend_from_slice(&att.block_number.to_le_bytes());
            msg.extend_from_slice(&att.data_root);
            msg.extend_from_slice(&att.validator_id.to_le_bytes());
            msg.extend_from_slice(&att.samples_verified.to_le_bytes());
            msg.extend_from_slice(&att.stake.to_le_bytes());

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

    /// M4 (audit 2026-05-02): like `verify_signatures`, but only counts
    /// attestations whose signer is currently active per
    /// `is_active(validator_id)`. A stale cert with signers who have
    /// since exited or been jailed will fail this check even if the
    /// raw BLS signatures themselves verify. Use this method (not
    /// `verify_signatures`) anywhere a cert reaches consensus or
    /// finality gating.
    pub fn verify_signatures_with_active(&self, is_active: &dyn Fn(u64) -> bool) -> bool {
        if self.attestations.is_empty() {
            return false;
        }
        let mut recomputed_stake: u64 = 0;
        for att in &self.attestations {
            if att.block_number != self.block_number || att.data_root != self.data_root {
                return false;
            }
            if !is_active(att.validator_id) {
                // Inactive / jailed signer — skip this attestation.
                // We do NOT short-circuit return false; jailing a
                // single signer post-hoc shouldn't invalidate the
                // whole cert if the remaining signers still meet
                // supermajority on their own.
                continue;
            }
            // Q3: include stake in signed message (matches create_attestation + verify_signatures).
            let mut msg = Vec::with_capacity(DA_ATTESTATION_DST.len() + 8 + 32 + 8 + 4 + 8);
            msg.extend_from_slice(DA_ATTESTATION_DST);
            msg.extend_from_slice(&att.block_number.to_le_bytes());
            msg.extend_from_slice(&att.data_root);
            msg.extend_from_slice(&att.validator_id.to_le_bytes());
            msg.extend_from_slice(&att.samples_verified.to_le_bytes());
            msg.extend_from_slice(&att.stake.to_le_bytes());
            let pk = BlsPublicKey(att.public_key.clone());
            let sig = BlsSignature(att.signature.clone());
            if !BlsVerifier::verify(&msg, &sig, &pk) {
                return false;
            }
            recomputed_stake = recomputed_stake.saturating_add(att.stake);
        }
        // The active-only stake must still hit strict supermajority.
        // Q4: strict > prevents two disjoint quorums at the boundary.
        (recomputed_stake as u128) * 3 > (self.total_stake as u128) * 2
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
    // Q3 (audit 2026-05-17): stake is included so the field cannot be
    // inflated post-signing without breaking the BLS verification.
    let mut msg = Vec::with_capacity(DA_ATTESTATION_DST.len() + 8 + 32 + 8 + 4 + 8);
    msg.extend_from_slice(DA_ATTESTATION_DST);
    msg.extend_from_slice(&block_number.to_le_bytes());
    msg.extend_from_slice(data_root);
    msg.extend_from_slice(&validator_id.to_le_bytes());
    msg.extend_from_slice(&samples_verified.to_le_bytes());
    msg.extend_from_slice(&stake.to_le_bytes());

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
        // DA_ATTESTATION_DST prefix and stake (Q3 fix).
        let mut msg = Vec::with_capacity(DA_ATTESTATION_DST.len() + 8 + 32 + 8 + 4 + 8);
        msg.extend_from_slice(DA_ATTESTATION_DST);
        msg.extend_from_slice(&att.block_number.to_le_bytes());
        msg.extend_from_slice(&att.data_root);
        msg.extend_from_slice(&att.validator_id.to_le_bytes());
        msg.extend_from_slice(&att.samples_verified.to_le_bytes());
        msg.extend_from_slice(&att.stake.to_le_bytes());

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

    /// Check if we already have strict supermajority (> 2/3).
    pub fn has_supermajority(&self) -> bool {
        // Audit C2: u128 to prevent wrap when stake > u64::MAX/3.
        // Q4: strict > prevents disjoint-quorum Byzantine forgery.
        (self.attested_stake as u128) * 3 > (self.total_stake as u128) * 2
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

    // ── T1.20: verify_signatures_with_active (M4 audit post-jail filter) ──

    /// T1.20 — M4 audit feature: `verify_signatures_with_active` with
    /// an `is_active` predicate that returns true for every signer
    /// behaves identically to `verify_signatures` (active-stake hits
    /// supermajority). Pinning the all-active baseline before the
    /// adversarial cases.
    #[test]
    fn t1_20_verify_with_active_all_active_passes() {
        let cert = build_valid_cert(3);
        // is_active accepts everyone.
        assert!(cert.verify_signatures_with_active(&|_| true));
    }

    /// T1.20 — M4 audit: one signer marked inactive but the remaining
    /// active signers still hit supermajority. Doctrine: a stale cert
    /// whose minority signer was jailed post-hoc must still be
    /// accepted (lines 117-125 — `continue` skips that attestation but
    /// the loop keeps adding the remaining active stakes).
    #[test]
    fn t1_20_verify_with_active_one_inactive_still_meets_quorum() {
        // 5 signers @ 1000 each = 5000 total. 2/3 threshold = 3333.
        // Drop validator 5 → remaining 4 × 1000 = 4000 ≥ 3334.
        let cert = build_valid_cert(5);
        let is_active = |vid: u64| vid != 5;
        assert!(
            cert.verify_signatures_with_active(&is_active),
            "remaining 4 of 5 signers must still constitute supermajority"
        );
    }

    /// T1.20 — M4 audit: enough signers marked inactive that the
    /// remaining active stake DROPS below supermajority. Doctrine:
    /// post-jail check must refuse the cert. (3 jailed of 5 →
    /// remaining 2 × 1000 = 2000 < 3334.)
    #[test]
    fn t1_20_verify_with_active_jailed_majority_fails() {
        let cert = build_valid_cert(5);
        // Validators 3, 4, 5 inactive — only 1+2 left.
        let is_active = |vid: u64| vid <= 2;
        assert!(
            !cert.verify_signatures_with_active(&is_active),
            "2 of 5 active signers is below 2/3 — cert must fail"
        );
    }

    /// T1.20 — M4 audit boundary: ALL signers marked inactive. The
    /// `recomputed_stake` stays at 0, and `0 * 3 >= total_stake * 2`
    /// holds only when `total_stake == 0`. With non-zero stake this
    /// must fail. Pinning the all-jailed edge case explicitly.
    #[test]
    fn t1_20_verify_with_active_all_inactive_fails() {
        let cert = build_valid_cert(3);
        assert!(
            !cert.verify_signatures_with_active(&|_| false),
            "every signer jailed → no recomputed stake → must fail"
        );
    }

    // ── Q1/Q2/Q3 adversarial tests (audit 2026-05-17) ────────────────────────

    /// Q2: duplicate validator_id in one certificate must be rejected.
    /// Attack: replaying a single valid attestation 100× inflates recomputed_stake
    /// 100-fold, bypassing the supermajority check with a single BLS keypair.
    #[test]
    fn q2_duplicate_validator_id_rejected() {
        let data_root = [0xD2u8; 32];
        let kp = BlsKeypair::generate();
        let att = create_attestation(1, &data_root, 42, 8, 1_000, &kp);

        // Two copies of the same attestation (validator_id=42 appears twice).
        let cert = DACertificate {
            block_number: 1,
            data_root,
            attestations: vec![att.clone(), att],
            attested_stake: 2_000,
            total_stake: 3_000, // 2000/3000 >= 2/3 — would pass supermajority
        };
        assert!(!cert.verify_signatures(), "duplicate validator_id must be rejected");
    }

    /// Q3: tampering the `stake` field post-signing must break BLS verification
    /// because `stake` is now included in the signed message.
    #[test]
    fn q3_tampered_stake_rejected() {
        let data_root = [0xD3u8; 32];
        let kp = BlsKeypair::generate();
        let mut att = create_attestation(1, &data_root, 1, 8, 1_000, &kp);

        // Inflate stake after signing.
        att.stake = 999_999_999;

        let cert = DACertificate {
            block_number: 1,
            data_root,
            attestations: vec![att],
            attested_stake: 999_999_999,
            total_stake: 999_999_999,
        };
        assert!(!cert.verify_signatures(), "inflated stake must fail BLS verification");
    }

    /// Q3: zero-stake attestation with matching sig still round-trips cleanly
    /// (stake=0 is a valid signed value; the forgery requires tampering it).
    #[test]
    fn q3_zero_stake_attestation_verifies() {
        let data_root = [0xD3u8; 32];
        let kp = BlsKeypair::generate();
        let att = create_attestation(1, &data_root, 1, 4, 0, &kp);
        let mut builder = CertificateBuilder::new(1, data_root, 0);
        assert!(builder.add_attestation(att));
    }

    // Audit C2: is_supermajority must not overflow when stakes are near u64::MAX.
    // Pre-fix: `attested * 3` and `total * 2` both wrap in u64, producing false
    // positives (undersized quorum passes) or false negatives (real quorum fails).
    #[test]
    fn audit_c2_is_supermajority_no_overflow_near_u64_max() {
        // total near u64::MAX (divisible by 3 for clean 2/3 boundary).
        let total: u64 = u64::MAX / 3 * 3;
        let attested: u64 = total / 3 * 2; // exactly 2/3
        let mut cert = DACertificate {
            block_number: 1,
            data_root: [0u8; 32],
            attestations: vec![],
            attested_stake: attested,
            total_stake: total,
        };
        assert!(cert.is_supermajority(), "2/3 of near-max stake must be supermajority");
        cert.attested_stake = attested - 1; // one below 2/3
        assert!(!cert.is_supermajority(), "2/3 - 1 of near-max stake must not be supermajority");
    }
}
