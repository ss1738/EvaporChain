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
    /// **DEPRECATED. AUDIT_2026_05_13 H5.**
    ///
    /// Compares two attacker-supplied fields (`self.attested_stake` and
    /// `self.total_stake`). A single Byzantine validator with stake `s`
    /// can build a cert with `attested_stake = s, total_stake = s,
    /// attestations = [self_attestation]` — trivially passes
    /// `s * 3 >= s * 2` AND the BLS-signature check in
    /// `verify_signatures` (the cert's own self-attestation is a
    /// genuine signature). Acceptance as DA supermajority follows.
    ///
    /// Use [`Self::verify_with_real_total_stake`] instead — it
    /// reconciles `total_stake` against the node's live validator-set
    /// view rather than the cert's self-reported value.
    #[deprecated(
        since = "0.2.0",
        note = "AUDIT_2026_05_13 H5: trusts attacker-supplied total_stake. \
                Use verify_with_real_total_stake which reconciles against \
                the local validator-set view."
    )]
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

    /// **DEPRECATED. AUDIT_2026_05_13 H5.**
    ///
    /// Combines `verify_signatures` with the broken
    /// `is_supermajority` — still trusts the cert's self-reported
    /// `total_stake`. Use [`Self::verify_with_real_total_stake`].
    #[deprecated(
        since = "0.2.0",
        note = "AUDIT_2026_05_13 H5: chains is_supermajority which trusts \
                attacker-supplied total_stake. Use verify_with_real_total_stake."
    )]
    pub fn verify_all(&self) -> bool {
        #[allow(deprecated)]
        let res = self.verify_signatures() && self.is_supermajority();
        res
    }

    /// **AUDIT_2026_05_13 H5 closure.** Full DA-cert validation that
    /// reconciles `total_stake` against the local validator-set view —
    /// closes the "single Byzantine validator forges supermajority" hole.
    ///
    /// For every attestation:
    /// 1. The signer must be active per `active_stake_of(vid)` (returns
    ///    `Some(real_stake)` for active, `None` for jailed / unknown).
    /// 2. The attestation's claimed `att.stake` is IGNORED in favour
    ///    of the real on-chain stake — closes the inflated-self-stake
    ///    forgery path within `verify_signatures`.
    /// 3. The BLS signature must verify against the attestation's
    ///    payload (block_number || data_root || validator_id ||
    ///    samples_verified) under DA_ATTESTATION_DST.
    ///
    /// Final gate: `Σ real_stakes_of_active_attesters * 3 ≥
    /// real_total_stake * 2`. The cert's self-reported
    /// `attested_stake` and `total_stake` fields are NEVER consulted.
    ///
    /// This is the only DA-cert verifier that should run anywhere a
    /// cert reaches consensus or finality gating. The legacy
    /// `is_supermajority` / `verify_all` / `verify_signatures` are
    /// preserved for back-compat against pre-fix tooling but flagged
    /// `#[deprecated]`.
    pub fn verify_with_real_total_stake(
        &self,
        real_total_stake: u64,
        active_stake_of: &dyn Fn(u64) -> Option<u64>,
    ) -> bool {
        if self.attestations.is_empty() {
            return false;
        }
        let mut real_attested_stake: u64 = 0;
        for att in &self.attestations {
            if att.block_number != self.block_number || att.data_root != self.data_root {
                return false;
            }
            let Some(real_stake) = active_stake_of(att.validator_id) else {
                // Jailed / unknown signer — skip per the same doctrine
                // as verify_signatures_with_active.
                continue;
            };
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
            real_attested_stake = real_attested_stake.saturating_add(real_stake);
        }
        // Supermajority against the REAL total — attacker's self-reported
        // total_stake field plays no part.
        real_attested_stake.saturating_mul(3) >= real_total_stake.saturating_mul(2)
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
        // The active-only stake must still hit supermajority.
        recomputed_stake.saturating_mul(3) >= self.total_stake.saturating_mul(2)
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
#[allow(deprecated)] // legacy tests still call deprecated is_supermajority / verify_all
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

    // ─── AUDIT_2026_05_13 H5 regression suite ─────────────────────────

    #[test]
    fn audit_h5_single_validator_supermajority_forgery_rejected() {
        // The audit's exact exploit: a single Byzantine validator V
        // with stake s builds a cert with self-attestation, attested
        // = total = s. Pre-fix this passes is_supermajority + verify_all
        // (the BLS signature IS genuine — V signed their own attestation).
        // The fix: real_total_stake is read from the validator set, not
        // the cert; the forgery's self-reported 100% supermajority is
        // measured against the REAL ~5000 total and rejected.
        let data_root = [0xAAu8; 32];
        let kp = BlsKeypair::generate();
        let v_stake = 1000u64;
        let att = create_attestation(1, &data_root, 1, 8, v_stake, &kp);
        let forgery = DACertificate {
            block_number: 1,
            data_root,
            attestations: vec![att],
            // Attacker self-reports their own stake as the total.
            attested_stake: v_stake,
            total_stake: v_stake,
        };
        // Pre-fix: is_supermajority returns true (1000 * 3 >= 1000 * 2).
        assert!(forgery.is_supermajority());
        assert!(forgery.verify_signatures(), "BLS sig itself is genuine");
        assert!(
            forgery.verify_all(),
            "pre-fix verify_all returns true — the audit's exact forgery"
        );

        // Post-fix: against the real validator set (V is one of 5 with
        // 1000 each), verify_with_real_total_stake correctly rejects.
        let real_total_stake = 5 * v_stake; // 5 validators in the real set
        let active_stake_of = |vid: u64| {
            if (1..=5).contains(&vid) {
                Some(v_stake)
            } else {
                None
            }
        };
        assert!(
            !forgery.verify_with_real_total_stake(real_total_stake, &active_stake_of),
            "single attester out of 5 must NOT pass supermajority against real total"
        );
    }

    #[test]
    fn audit_h5_legitimate_supermajority_against_real_set_verifies() {
        // Soundness lower bound: a real cert with 4 of 5 active
        // validators attesting (80% > 66.7%) must verify under
        // verify_with_real_total_stake.
        let data_root = [0xBBu8; 32];
        let v_stake = 1000u64;
        let kps: Vec<BlsKeypair> = (0..5).map(|_| BlsKeypair::generate()).collect();
        let attestations: Vec<DAAttestation> = kps
            .iter()
            .enumerate()
            .take(4) // only 4 of 5 attest
            .map(|(i, kp)| create_attestation(1, &data_root, (i + 1) as u64, 8, v_stake, kp))
            .collect();
        let cert = DACertificate {
            block_number: 1,
            data_root,
            attestations,
            // These fields could be anything attacker-supplied — verify_with_real_total_stake
            // ignores them. Set them to legitimate values for clarity.
            attested_stake: 4 * v_stake,
            total_stake: 5 * v_stake,
        };
        let real_total_stake = 5 * v_stake;
        let active_stake_of = |vid: u64| {
            if (1..=5).contains(&vid) {
                Some(v_stake)
            } else {
                None
            }
        };
        assert!(cert.verify_with_real_total_stake(real_total_stake, &active_stake_of));
    }

    #[test]
    fn audit_h5_inflated_self_stake_attestation_overridden_by_real() {
        // A signer who is a real validator but claims a wildly inflated
        // self-stake in `att.stake` must be measured by their REAL
        // on-chain stake, not the inflated one. Pre-fix verify_signatures
        // accumulated att.stake (capped by attested_stake but that itself
        // is attacker-controlled). Post-fix uses the active_stake_of
        // callback only.
        let data_root = [0xCCu8; 32];
        let kp = BlsKeypair::generate();
        let inflated_claimed = 1_000_000_000u64; // 1B (claim)
        let real = 1000u64; // truth
        let att = create_attestation(1, &data_root, 1, 8, inflated_claimed, &kp);
        let cert = DACertificate {
            block_number: 1,
            data_root,
            attestations: vec![att],
            attested_stake: inflated_claimed,
            total_stake: inflated_claimed, // attacker says it's the whole network
        };
        let real_total_stake = 10 * real; // 10 validators of 1000 each
        let active_stake_of = |vid: u64| if vid == 1 { Some(real) } else { Some(real) };
        // verify_with_real_total_stake uses `real` (1000) not `inflated_claimed`
        // (1B), so attested = 1000 against total 10000 → 10% < 66.7% → reject.
        assert!(
            !cert.verify_with_real_total_stake(real_total_stake, &active_stake_of),
            "inflated self-stake claim must be overridden by real on-chain stake"
        );
    }

    #[test]
    fn audit_h5_jailed_signer_drops_from_attested_pool() {
        // 5 attestations but signer 5 is jailed at verification time.
        // Real total is still 5000 (the validator-set view at proposal
        // time, before jailing). 4 active attesters × 1000 = 4000 ≥ 3334,
        // so the cert still passes — mirrors verify_signatures_with_active
        // doctrine but with real total instead of self-reported.
        let data_root = [0xDDu8; 32];
        let v_stake = 1000u64;
        let kps: Vec<BlsKeypair> = (0..5).map(|_| BlsKeypair::generate()).collect();
        let attestations: Vec<DAAttestation> = kps
            .iter()
            .enumerate()
            .map(|(i, kp)| create_attestation(1, &data_root, (i + 1) as u64, 8, v_stake, kp))
            .collect();
        let cert = DACertificate {
            block_number: 1,
            data_root,
            attestations,
            attested_stake: 5 * v_stake,
            total_stake: 5 * v_stake,
        };
        let real_total_stake = 5 * v_stake;
        let active_stake_of = |vid: u64| {
            if (1..=4).contains(&vid) {
                Some(v_stake)
            } else {
                None // signer 5 jailed
            }
        };
        assert!(cert.verify_with_real_total_stake(real_total_stake, &active_stake_of));
    }

    #[test]
    fn audit_h5_empty_attestations_rejected() {
        let cert = DACertificate {
            block_number: 1,
            data_root: [0xAA; 32],
            attestations: vec![],
            attested_stake: 0,
            total_stake: 5000,
        };
        let active_stake_of = |_: u64| Some(1000u64);
        assert!(!cert.verify_with_real_total_stake(5000, &active_stake_of));
    }
}
