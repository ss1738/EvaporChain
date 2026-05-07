//! Top-level [`LightClient`] struct — the primary SDK entry point.
//!
//! Composes the three verifier layers (BFT commit-certificate,
//! Nova-IVC sublinear, Verkle state-query) into a single object
//! consumers hold across calls.

use evaporchain_consensus::light_client::{
    LightBlockHeader, LightClientVerifier, VerificationResult,
};

use crate::error::LightClientError;

/// Top-level light-client struct. Holds the BFT verifier (always
/// active), optional Nova verification key bytes (active when the
/// `nova` feature is enabled and `vk_bytes` is supplied at
/// construction), and the most-recently-trusted block header.
///
/// Construct via [`LightClient::new`]; verify each new block via
/// [`LightClient::ingest_block`]. State queries are verified via
/// [`LightClient::verify_state`] (in `state_query.rs`).
pub struct LightClient {
    /// BFT commit-certificate verifier (the tracking-trust-period
    /// + validator-set engine).
    bft: LightClientVerifier,

    /// `vk_bytes` for Nova-IVC sublinear verification. `None`
    /// means the SDK runs in BFT-only mode (still useful — energy-
    /// floor checks at the BFT level work fine).
    #[cfg(feature = "nova")]
    vk_bytes: Option<Vec<u8>>,

    /// Most-recently-trusted block header. Updated on each
    /// successful `ingest_block`. Used for state-root binding on
    /// state queries.
    trusted_tip: LightBlockHeader,

    /// Trust period in seconds (mirrored from the BFT verifier
    /// for surface-level operator visibility).
    trust_period_secs: u64,
}

impl LightClient {
    /// Construct a new light client anchored at `genesis_header`,
    /// with the given `current_time` (Unix seconds) used for
    /// trust-period tracking. Optionally supply `vk_bytes` for
    /// Nova-IVC sublinear verification (feature-gated).
    pub fn new(
        genesis_header: LightBlockHeader,
        current_time: u64,
        #[cfg(feature = "nova")] vk_bytes: Option<Vec<u8>>,
    ) -> Self {
        let bft = LightClientVerifier::new(genesis_header.clone(), current_time);
        Self {
            bft,
            #[cfg(feature = "nova")]
            vk_bytes,
            trusted_tip: genesis_header,
            // Default 2-week trust period from
            // evaporchain-consensus::light_client::TRUST_PERIOD_SECS.
            trust_period_secs: 14 * 24 * 3600,
        }
    }

    /// Construct with a custom trust period (seconds). Useful for
    /// testing or operator policies that want a tighter / looser
    /// trust window.
    pub fn with_trust_period(
        genesis_header: LightBlockHeader,
        current_time: u64,
        trust_period_secs: u64,
        #[cfg(feature = "nova")] vk_bytes: Option<Vec<u8>>,
    ) -> Self {
        let bft = evaporchain_consensus::light_client::LightClientVerifier::with_trust_period(
            genesis_header.clone(),
            current_time,
            trust_period_secs,
        );
        Self {
            bft,
            #[cfg(feature = "nova")]
            vk_bytes,
            trusted_tip: genesis_header,
            trust_period_secs,
        }
    }

    /// Current trust-period (seconds).
    pub fn trust_period_secs(&self) -> u64 {
        self.trust_period_secs
    }

    /// Most-recently-trusted block header.
    pub fn trusted_tip(&self) -> &LightBlockHeader {
        &self.trusted_tip
    }

    /// Most-recently-trusted block height.
    pub fn current_height(&self) -> u64 {
        self.trusted_tip.height
    }

    /// Most-recently-trusted state root (32 bytes). Use this as
    /// the binding for state-query Merkle proofs.
    pub fn current_state_root(&self) -> [u8; 32] {
        self.trusted_tip.state_root
    }

    /// Ingest a new block. Returns `Ok(())` on successful BFT
    /// verification (and Nova, if enabled — see [Self::ingest_block_with_nova]);
    /// the trusted tip is updated to this block. On failure the
    /// trusted tip is unchanged.
    ///
    /// Verification stages in order:
    ///   1. Monotone-height check (provided > trusted).
    ///   2. Parent-hash adjacency check (when height = trusted+1).
    ///   3. BFT BLS aggregate-signature verification via the
    ///      consensus-side `LightClientVerifier::verify`. Validates
    ///      signer set membership, ≥2/3 stake quorum, BLS aggregate
    ///      signature, and trust-period freshness. For skip-mode
    ///      (height gap > 1), also checks validator-set overlap.
    pub fn ingest_block(
        &mut self,
        header: LightBlockHeader,
        current_time: u64,
    ) -> Result<(), LightClientError> {
        // Stage 1: Monotone-height enforcement.
        if header.height <= self.trusted_tip.height {
            return Err(LightClientError::NonMonotoneHeight {
                provided: header.height,
                trusted: self.trusted_tip.height,
            });
        }

        // Stage 2: Parent-hash adjacency check (only for height = trusted+1).
        if header.height == self.trusted_tip.height + 1 {
            if header.parent_hash != self.trusted_tip.block_hash {
                return Err(LightClientError::ParentHashMismatch {
                    height: header.height,
                    parent_hash_hex: hex_lower(&header.parent_hash),
                    trusted_hash_hex: hex_lower(&self.trusted_tip.block_hash),
                });
            }
        }

        // Stage 3: BFT BLS aggregate-sig verification via the
        // consensus-side verifier. Translates VerificationResult
        // into our error type.
        match self.bft.verify(&header, current_time) {
            VerificationResult::Valid => {
                self.trusted_tip = header;
                Ok(())
            }
            VerificationResult::Invalid(msg) => Err(LightClientError::Bft(msg)),
            VerificationResult::NeedBisection {
                trusted_height,
                target_height,
            } => Err(LightClientError::Bft(format!(
                "skip verification needs bisection: trusted_height={trusted_height}, target_height={target_height}"
            ))),
        }
    }
}

// ───────────────────────── helpers ─────────────────────────────

pub(crate) fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(nibble_hex(b >> 4));
        s.push(nibble_hex(b & 0x0f));
    }
    s
}

fn nibble_hex(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'a' + (n - 10)) as char,
        _ => '?',
    }
}

// ───────────────────────── tests ───────────────────────────────

#[cfg(test)]
pub(crate) mod test_fixtures {
    //! Test fixture helpers. Build real BLS-signed validator sets
    //! and commit certificates for SDK tests. Mirrors the helpers
    //! used in `evaporchain-consensus::light_client::tests` so the
    //! SDK exercises the same verification path the chain uses.

    use evaporchain_consensus::light_client::LightBlockHeader;
    use evaporchain_consensus::validator_set::{ValidatorInfo, ValidatorSet};
    use evaporchain_crypto::signatures::{BlsKeypair, BlsSignature, BlsVerifier};
    use evaporchain_types::CommitCertificate;

    /// Canonical BLS vote message format — matches
    /// `evaporchain-consensus::light_client::bls_vote_message`
    /// (private fn there; inline copy here so SDK tests can build
    /// matching certs without exposing a new public surface in
    /// consensus).
    pub fn bls_vote_message(height: u64, round: u32, block_hash: &[u8; 32]) -> Vec<u8> {
        let mut msg = Vec::with_capacity(48);
        msg.extend_from_slice(b"precommit");
        msg.extend_from_slice(&height.to_le_bytes());
        msg.extend_from_slice(&round.to_le_bytes());
        msg.extend_from_slice(block_hash);
        msg
    }

    /// Build a validator set with `n` validators, each with the
    /// given uniform stake and a real BLS keypair. Returns the set
    /// + keypairs (caller signs with these to build commit certs).
    pub fn make_validator_set_with_bls(n: u64, stake: u64) -> (ValidatorSet, Vec<BlsKeypair>) {
        let mut vs = ValidatorSet::new();
        let mut keypairs = Vec::new();
        for i in 0..n {
            let kp = BlsKeypair::generate();
            let mut info = ValidatorInfo::new(i, stake, [i as u8; 32]);
            info.bls_public_key = Some(kp.public_key_bytes().0);
            info.pop_verified = true;
            vs.add_validator(info);
            keypairs.push(kp);
        }
        (vs, keypairs)
    }

    /// Build a valid commit certificate signed by the listed
    /// `signer_ids` from `keypairs`.
    pub fn make_commit_certificate(
        height: u64,
        round: u32,
        block_hash: [u8; 32],
        keypairs: &[BlsKeypair],
        signer_ids: &[u64],
    ) -> CommitCertificate {
        let msg = bls_vote_message(height, round, &block_hash);
        let sigs: Vec<BlsSignature> = signer_ids
            .iter()
            .map(|&id| keypairs[id as usize].sign(&msg))
            .collect();
        let agg_sig = BlsVerifier::aggregate_signatures(&sigs).unwrap();
        CommitCertificate {
            height,
            round,
            block_hash,
            aggregate_signature: agg_sig.0,
            signer_ids: signer_ids.to_vec(),
        }
    }

    /// Assemble a valid signed light block header.
    pub fn make_signed_header(
        height: u64,
        parent_hash: [u8; 32],
        block_hash: [u8; 32],
        vs: ValidatorSet,
        keypairs: &[BlsKeypair],
        signer_ids: &[u64],
    ) -> LightBlockHeader {
        let cert = make_commit_certificate(height, 0, block_hash, keypairs, signer_ids);
        LightBlockHeader {
            height,
            epoch: 0,
            block_hash,
            parent_hash,
            state_root: [height as u8; 32],
            timestamp: height * 10,
            validator_set: vs,
            commit_certificate: cert,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::test_fixtures::*;

    #[test]
    fn new_starts_at_genesis_height() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis = make_signed_header(1, [0u8; 32], [0xaa; 32], vs, &kps, &[0, 1, 2]);
        let lc = LightClient::new(
            genesis,
            100,
            #[cfg(feature = "nova")]
            None,
        );
        assert_eq!(lc.current_height(), 1);
        assert_eq!(lc.current_state_root(), [1u8; 32]);
        assert_eq!(lc.trusted_tip().block_hash, [0xaa; 32]);
    }

    #[test]
    fn ingest_signed_sequential_block_succeeds() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis = make_signed_header(1, [0u8; 32], [0xaa; 32], vs.clone(), &kps, &[0, 1, 2]);
        let mut lc = LightClient::new(
            genesis,
            100,
            #[cfg(feature = "nova")]
            None,
        );

        let next = make_signed_header(2, [0xaa; 32], [0xbb; 32], vs, &kps, &[0, 1, 2]);
        lc.ingest_block(next, 110).expect("signed sequential block must verify");
        assert_eq!(lc.current_height(), 2);
        assert_eq!(lc.current_state_root(), [2u8; 32]);
    }

    #[test]
    fn ingest_non_monotone_block_rejected() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis = make_signed_header(5, [0u8; 32], [0xaa; 32], vs.clone(), &kps, &[0, 1, 2]);
        let mut lc = LightClient::new(
            genesis,
            100,
            #[cfg(feature = "nova")]
            None,
        );
        // Same-height block — must be rejected before any BFT check.
        let same = make_signed_header(5, [0u8; 32], [0xcc; 32], vs.clone(), &kps, &[0, 1, 2]);
        assert!(matches!(
            lc.ingest_block(same, 110),
            Err(LightClientError::NonMonotoneHeight { .. })
        ));
        // Older block — must be rejected.
        let older = make_signed_header(3, [0u8; 32], [0xdd; 32], vs, &kps, &[0, 1, 2]);
        assert!(matches!(
            lc.ingest_block(older, 110),
            Err(LightClientError::NonMonotoneHeight { .. })
        ));
    }

    #[test]
    fn ingest_adjacent_block_with_wrong_parent_hash_rejected() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis = make_signed_header(1, [0u8; 32], [0xaa; 32], vs.clone(), &kps, &[0, 1, 2]);
        let mut lc = LightClient::new(
            genesis,
            100,
            #[cfg(feature = "nova")]
            None,
        );
        // Adjacent block (height 2) but parent_hash != [0xaa; 32].
        let bad_parent = make_signed_header(2, [0x11; 32], [0xbb; 32], vs, &kps, &[0, 1, 2]);
        assert!(matches!(
            lc.ingest_block(bad_parent, 110),
            Err(LightClientError::ParentHashMismatch { .. })
        ));
    }

    #[test]
    fn ingest_block_with_insufficient_signers_rejected() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis = make_signed_header(1, [0u8; 32], [0xaa; 32], vs.clone(), &kps, &[0, 1, 2]);
        let mut lc = LightClient::new(
            genesis,
            100,
            #[cfg(feature = "nova")]
            None,
        );
        // Only 1 of 4 signers — below quorum (need ≥3).
        let weak = make_signed_header(2, [0xaa; 32], [0xbb; 32], vs, &kps, &[0]);
        let err = lc
            .ingest_block(weak, 110)
            .expect_err("insufficient signers must be rejected");
        assert!(matches!(err, LightClientError::Bft(_)));
    }

    #[test]
    fn ingest_block_with_corrupted_signature_rejected() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis = make_signed_header(1, [0u8; 32], [0xaa; 32], vs.clone(), &kps, &[0, 1, 2]);
        let mut lc = LightClient::new(
            genesis,
            100,
            #[cfg(feature = "nova")]
            None,
        );
        // Build a valid header, then corrupt the aggregate sig.
        let mut bad = make_signed_header(2, [0xaa; 32], [0xbb; 32], vs, &kps, &[0, 1, 2]);
        if !bad.commit_certificate.aggregate_signature.is_empty() {
            bad.commit_certificate.aggregate_signature[0] ^= 0xff;
        }
        let err = lc
            .ingest_block(bad, 110)
            .expect_err("corrupted aggregate sig must be rejected");
        assert!(matches!(err, LightClientError::Bft(_)));
    }

    #[test]
    fn ingest_block_after_trust_period_expiry_rejected() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis = make_signed_header(1, [0u8; 32], [0xaa; 32], vs.clone(), &kps, &[0, 1, 2]);
        // 1-second trust period for fast expiry test.
        let mut lc = LightClient::with_trust_period(
            genesis,
            100,
            1,
            #[cfg(feature = "nova")]
            None,
        );
        // Wait past the trust period.
        let next = make_signed_header(2, [0xaa; 32], [0xbb; 32], vs, &kps, &[0, 1, 2]);
        let err = lc
            .ingest_block(next, 200)
            .expect_err("expired trust period must reject");
        assert!(matches!(err, LightClientError::Bft(_)));
    }

    #[test]
    fn trust_period_default_is_two_weeks() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis = make_signed_header(1, [0u8; 32], [0xaa; 32], vs, &kps, &[0, 1, 2]);
        let lc = LightClient::new(
            genesis,
            100,
            #[cfg(feature = "nova")]
            None,
        );
        assert_eq!(lc.trust_period_secs(), 14 * 24 * 3600);
    }

    #[test]
    fn with_custom_trust_period_overrides_default() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis = make_signed_header(1, [0u8; 32], [0xaa; 32], vs, &kps, &[0, 1, 2]);
        let lc = LightClient::with_trust_period(
            genesis,
            100,
            3600,
            #[cfg(feature = "nova")]
            None,
        );
        assert_eq!(lc.trust_period_secs(), 3600);
    }

    #[test]
    fn hex_lower_round_trip() {
        let bytes: [u8; 4] = [0x00, 0x12, 0xab, 0xff];
        assert_eq!(hex_lower(&bytes), "0012abff");
    }
}
