//! Top-level [`LightClient`] struct — the primary SDK entry point.
//!
//! Composes the three verifier layers (BFT commit-certificate,
//! Nova-IVC sublinear, Verkle state-query) into a single object
//! consumers hold across calls.

use evaporchain_consensus_types::{LightBlockHeader, LightClientVerifier, VerificationResult};

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
    /// floor checks at the BFT level work fine). Always present
    /// in the struct regardless of the `nova` feature flag — the
    /// flag only gates whether the actual SNARK verification path
    /// is compiled in. Keeping the field unconditional keeps
    /// constructor signatures uniform across feature flavours.
    #[cfg_attr(not(feature = "nova"), allow(dead_code))]
    vk_bytes: Option<Vec<u8>>,

    /// Most-recently-trusted block header. Updated on each
    /// successful `ingest_block`. Used for state-root binding on
    /// state queries.
    trusted_tip: LightBlockHeader,

    /// Trust period in seconds (mirrored from the BFT verifier
    /// for surface-level operator visibility).
    trust_period_secs: u64,

    /// Chain ID bound into BLS vote messages.
    chain_id: String,
}

impl LightClient {
    /// Construct a new light client anchored at `genesis_header`,
    /// with the given `current_time` (Unix seconds) used for
    /// trust-period tracking. Pass `vk_bytes = Some(...)` to
    /// enable Nova-IVC sublinear verification via
    /// [`LightClient::ingest_block_with_nova`] (requires `nova`
    /// feature); pass `None` for BFT-only operation.
    pub fn new(
        genesis_header: LightBlockHeader,
        current_time: u64,
        chain_id: &str,
        vk_bytes: Option<Vec<u8>>,
    ) -> Self {
        let bft = LightClientVerifier::new(genesis_header.clone(), current_time, chain_id);
        Self {
            bft,
            vk_bytes,
            trusted_tip: genesis_header,
            // Default 2-week trust period from
            // evaporchain-consensus::light_client::TRUST_PERIOD_SECS.
            trust_period_secs: 14 * 24 * 3600,
            chain_id: chain_id.to_string(),
        }
    }

    /// Construct with a custom trust period (seconds). Useful for
    /// testing or operator policies that want a tighter / looser
    /// trust window.
    pub fn with_trust_period(
        genesis_header: LightBlockHeader,
        current_time: u64,
        trust_period_secs: u64,
        chain_id: &str,
        vk_bytes: Option<Vec<u8>>,
    ) -> Self {
        let bft = evaporchain_consensus_types::LightClientVerifier::with_trust_period(
            genesis_header.clone(),
            current_time,
            trust_period_secs,
            chain_id,
        );
        Self {
            bft,
            vk_bytes,
            trusted_tip: genesis_header,
            trust_period_secs,
            chain_id: chain_id.to_string(),
        }
    }

    // ── Crate-internal accessors used by the `nova` module ────

    /// Mutable handle to the BFT verifier. Crate-private so the
    /// `nova` module can drive the same BFT verification path.
    #[cfg(feature = "nova")]
    pub(crate) fn bft_verifier_mut(&mut self) -> &mut LightClientVerifier {
        &mut self.bft
    }

    /// Read-only access to vk_bytes for the Nova path.
    #[cfg(feature = "nova")]
    pub(crate) fn vk_bytes_ref(&self) -> Option<&[u8]> {
        self.vk_bytes.as_deref()
    }

    /// Promote a header to the trusted tip after both BFT and
    /// Nova verification have succeeded.
    #[cfg(feature = "nova")]
    pub(crate) fn set_trusted_tip(&mut self, header: LightBlockHeader) {
        self.trusted_tip = header;
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
    ///   2. BFT BLS aggregate-signature verification via the
    ///      consensus-side `LightClientVerifier::verify`. Validates
    ///      signer set membership, ≥2/3 stake quorum, BLS aggregate
    ///      signature, and trust-period freshness. For skip-mode
    ///      (height gap > 1), also checks validator-set overlap.
    ///
    /// **Note on parent-hash adjacency**: an earlier draft of the
    /// SDK enforced `header.parent_hash == trusted_tip.block_hash`
    /// when `height == trusted+1`. That check was removed
    /// 2026-05-08 after live-chain verification revealed
    /// EvaporChain producer-side `block.parent_hash` uses a
    /// custom recursive formula (see `tendermint.rs:5263-5269`,
    /// `blake3(number || epoch || state_root || prev_parent_hash)`)
    /// distinct from `cert.block_hash` used in commit certificates.
    /// The two never coincide, so the adjacency check rejected
    /// every otherwise-valid sequential block. BFT BLS aggregate-
    /// sig verification is the actual authoritative chain
    /// authentication; parent-hash adjacency was speculative
    /// defence-in-depth that turned out to be incorrect for this
    /// chain. The cert verification ensures ≥2/3 stake attested
    /// to (height, block_hash) — sufficient.
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

        // Stage 2: BFT BLS aggregate-sig verification via the
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

    use evaporchain_consensus_types::LightBlockHeader;
    use evaporchain_consensus_types::{ValidatorInfo, ValidatorSet};
    use evaporchain_crypto::signatures::{BlsKeypair, BlsSignature, BlsVerifier};
    use evaporchain_types::CommitCertificate;

    /// Canonical BLS vote message format — matches tendermint.rs format exactly.
    /// Format: u8(len(chain_id)) || chain_id || "precommit" || height_le8 || round_le4 || block_hash
    pub fn bls_vote_message(
        chain_id: &str,
        height: u64,
        round: u32,
        block_hash: &[u8; 32],
    ) -> Vec<u8> {
        let chain_id_bytes = chain_id.as_bytes();
        let mut msg = Vec::with_capacity(1 + chain_id_bytes.len() + 9 + 8 + 4 + 32);
        msg.push(chain_id_bytes.len() as u8);
        msg.extend_from_slice(chain_id_bytes);
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
        let msg = bls_vote_message("", height, round, &block_hash);
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
    use super::test_fixtures::*;
    use super::*;

    #[test]
    fn new_starts_at_genesis_height() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis = make_signed_header(1, [0u8; 32], [0xaa; 32], vs, &kps, &[0, 1, 2]);
        let lc = LightClient::new(genesis, 100, "", None);
        assert_eq!(lc.current_height(), 1);
        assert_eq!(lc.current_state_root(), [1u8; 32]);
        assert_eq!(lc.trusted_tip().block_hash, [0xaa; 32]);
    }

    #[test]
    fn ingest_signed_sequential_block_succeeds() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis = make_signed_header(1, [0u8; 32], [0xaa; 32], vs.clone(), &kps, &[0, 1, 2]);
        let mut lc = LightClient::new(genesis, 100, "", None);

        let next = make_signed_header(2, [0xaa; 32], [0xbb; 32], vs, &kps, &[0, 1, 2]);
        lc.ingest_block(next, 110)
            .expect("signed sequential block must verify");
        assert_eq!(lc.current_height(), 2);
        assert_eq!(lc.current_state_root(), [2u8; 32]);
    }

    #[test]
    fn ingest_non_monotone_block_rejected() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis = make_signed_header(5, [0u8; 32], [0xaa; 32], vs.clone(), &kps, &[0, 1, 2]);
        let mut lc = LightClient::new(genesis, 100, "", None);
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
    fn ingest_adjacent_block_with_arbitrary_parent_hash_accepted() {
        // Test renamed + inverted 2026-05-08 after live-chain
        // verification revealed EvaporChain's producer-side
        // `block.parent_hash` uses a different formula than
        // `cert.block_hash` (see `tendermint.rs:5263-5269`).
        // The SDK no longer enforces parent-hash adjacency —
        // BFT BLS aggregate-sig is the authoritative chain
        // authentication. So a block with a parent_hash that
        // doesn't match the trusted tip's block_hash should
        // STILL verify, as long as the BLS sig is valid.
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis = make_signed_header(1, [0u8; 32], [0xaa; 32], vs.clone(), &kps, &[0, 1, 2]);
        let mut lc = LightClient::new(genesis, 100, "", None);
        // Adjacent block (height 2) with an unrelated parent_hash —
        // must STILL accept since BFT BLS sig is valid.
        let arbitrary_parent = make_signed_header(2, [0x11; 32], [0xbb; 32], vs, &kps, &[0, 1, 2]);
        lc.ingest_block(arbitrary_parent, 110)
            .expect("BFT-valid block must verify regardless of parent_hash");
        assert_eq!(lc.current_height(), 2);
    }

    #[test]
    fn ingest_block_with_insufficient_signers_rejected() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis = make_signed_header(1, [0u8; 32], [0xaa; 32], vs.clone(), &kps, &[0, 1, 2]);
        let mut lc = LightClient::new(genesis, 100, "", None);
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
        let mut lc = LightClient::new(genesis, 100, "", None);
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
        let mut lc = LightClient::with_trust_period(genesis, 100, 1, "", None);
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
        let lc = LightClient::new(genesis, 100, "", None);
        assert_eq!(lc.trust_period_secs(), 14 * 24 * 3600);
    }

    #[test]
    fn with_custom_trust_period_overrides_default() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis = make_signed_header(1, [0u8; 32], [0xaa; 32], vs, &kps, &[0, 1, 2]);
        let lc = LightClient::with_trust_period(genesis, 100, 3600, "", None);
        assert_eq!(lc.trust_period_secs(), 3600);
    }

    #[test]
    fn hex_lower_round_trip() {
        let bytes: [u8; 4] = [0x00, 0x12, 0xab, 0xff];
        assert_eq!(hex_lower(&bytes), "0012abff");
    }
}
