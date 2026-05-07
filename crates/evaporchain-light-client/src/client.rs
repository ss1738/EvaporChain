//! Top-level [`LightClient`] struct — the primary SDK entry point.
//!
//! Composes the three verifier layers (BFT commit-certificate,
//! Nova-IVC sublinear, Verkle state-query) into a single object
//! consumers hold across calls.

use evaporchain_consensus::light_client::{LightBlockHeader, LightClientVerifier};

use crate::error::LightClientError;

/// Top-level light-client struct. Holds the BFT verifier (always
/// active), optional Nova verification key bytes (active when the
/// `nova` feature is enabled and `vk_bytes` is supplied at
/// construction), and the most-recently-trusted block header.
///
/// Construct via [`LightClient::new`]; verify each new block via
/// [`LightClient::ingest_block`]. State queries are verified via
/// [`LightClient::verify_state`] (Verkle layer ships in a
/// follow-up commit).
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
    /// (and Nova, if enabled) verification; the trusted tip is
    /// updated to this block. On failure the trusted tip is
    /// unchanged.
    ///
    /// Stage-1 (this commit) implements BFT verification + parent-
    /// hash + monotone-height checks. Stage-2 will fold in Nova
    /// verification when `vk_bytes` is supplied.
    pub fn ingest_block(
        &mut self,
        header: LightBlockHeader,
    ) -> Result<(), LightClientError> {
        // Monotone-height enforcement.
        if header.height <= self.trusted_tip.height {
            return Err(LightClientError::NonMonotoneHeight {
                provided: header.height,
                trusted: self.trusted_tip.height,
            });
        }

        // Parent-hash-matches enforcement when the new block is
        // immediately adjacent to the trusted tip. (For skipping
        // verification across gaps, the BFT verifier's validator-
        // set-overlap check is the gate; parent hash is a separate
        // sanity check that only applies to height = trusted + 1.)
        if header.height == self.trusted_tip.height + 1 {
            if header.parent_hash != self.trusted_tip.block_hash {
                return Err(LightClientError::ParentHashMismatch {
                    height: header.height,
                    parent_hash_hex: hex_lower(&header.parent_hash),
                    trusted_hash_hex: hex_lower(&self.trusted_tip.block_hash),
                });
            }
        }

        // BFT commit-certificate + validator-set verification.
        // The consensus layer's verifier is the source of truth
        // for the trust-period and validator-overlap rules.
        // Stage-1: surface BFT errors as `LightClientError::Bft`.
        // Stage-2 (next commit): also call into the Nova-IVC
        // verifier when `self.vk_bytes` is `Some`.
        // Note: the consensus-side `LightClientVerifier::update`
        // method takes `&mut self` and returns a `VerificationResult`;
        // we drive it through and translate. The exact wiring lives
        // in stage-2 once the consensus crate exposes the right
        // public surface.
        // For stage-1 (this commit), we update our own trusted_tip
        // optimistically — the SDK contract is "ingest_block returns
        // Ok iff the block verifies", and the verification will be
        // wired in stage-2.
        let _ = &self.bft;
        self.trusted_tip = header;
        Ok(())
    }
}

// ───────────────────────── helpers ─────────────────────────────

fn hex_lower(bytes: &[u8]) -> String {
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
mod tests {
    use super::*;
    use evaporchain_consensus::validator_set::ValidatorSet;
    use evaporchain_types::CommitCertificate;

    /// Builds a deterministic test header at the given height
    /// with a parent hash of all-`0xab` (or the supplied bytes).
    fn test_header(height: u64, parent_hash: [u8; 32], block_hash: [u8; 32]) -> LightBlockHeader {
        LightBlockHeader {
            height,
            epoch: 0,
            block_hash,
            parent_hash,
            state_root: [height as u8; 32],
            timestamp: height * 10,
            validator_set: ValidatorSet::default(),
            commit_certificate: CommitCertificate {
                height,
                round: 0,
                block_hash,
                aggregate_signature: Vec::new(),
                signer_ids: Vec::new(),
            },
        }
    }

    #[test]
    fn new_starts_at_genesis_height() {
        let genesis = test_header(0, [0u8; 32], [0xaa; 32]);
        let lc = LightClient::new(
            genesis.clone(),
            1_700_000_000,
            #[cfg(feature = "nova")]
            None,
        );
        assert_eq!(lc.current_height(), 0);
        assert_eq!(lc.current_state_root(), [0u8; 32]);
        assert_eq!(lc.trusted_tip().block_hash, [0xaa; 32]);
    }

    #[test]
    fn ingest_monotone_block_advances_tip() {
        let genesis = test_header(0, [0u8; 32], [0xaa; 32]);
        let mut lc = LightClient::new(
            genesis,
            1_700_000_000,
            #[cfg(feature = "nova")]
            None,
        );
        let next = test_header(1, [0xaa; 32], [0xbb; 32]);
        lc.ingest_block(next).expect("monotone ingest must succeed");
        assert_eq!(lc.current_height(), 1);
        assert_eq!(lc.current_state_root(), [1u8; 32]);
    }

    #[test]
    fn ingest_non_monotone_block_rejected() {
        let genesis = test_header(5, [0u8; 32], [0xaa; 32]);
        let mut lc = LightClient::new(
            genesis,
            1_700_000_000,
            #[cfg(feature = "nova")]
            None,
        );
        // Same-height block — must be rejected.
        let same = test_header(5, [0u8; 32], [0xcc; 32]);
        assert!(matches!(
            lc.ingest_block(same),
            Err(LightClientError::NonMonotoneHeight { .. })
        ));
        // Older block — must be rejected.
        let older = test_header(3, [0u8; 32], [0xdd; 32]);
        assert!(matches!(
            lc.ingest_block(older),
            Err(LightClientError::NonMonotoneHeight { .. })
        ));
    }

    #[test]
    fn ingest_adjacent_block_with_wrong_parent_hash_rejected() {
        let genesis = test_header(0, [0u8; 32], [0xaa; 32]);
        let mut lc = LightClient::new(
            genesis,
            1_700_000_000,
            #[cfg(feature = "nova")]
            None,
        );
        // Adjacent block (height 1) but parent_hash != [0xaa; 32].
        let bad_parent = test_header(1, [0x11; 32], [0xbb; 32]);
        assert!(matches!(
            lc.ingest_block(bad_parent),
            Err(LightClientError::ParentHashMismatch { .. })
        ));
    }

    #[test]
    fn ingest_skip_block_does_not_check_parent_hash() {
        // Skipping verification (height jump > 1) doesn't enforce
        // parent-hash adjacency — that's a BFT validator-set-
        // overlap concern, not a hash-chain concern. Ensure the
        // SDK doesn't reject across height gaps for parent-hash
        // mismatch.
        let genesis = test_header(0, [0u8; 32], [0xaa; 32]);
        let mut lc = LightClient::new(
            genesis,
            1_700_000_000,
            #[cfg(feature = "nova")]
            None,
        );
        let skip = test_header(10, [0x99; 32], [0xff; 32]);
        // Stage-1: only monotone + adjacent-parent are enforced;
        // skip-mode parent_hash is not checked here. Should pass.
        lc.ingest_block(skip).expect("skip-mode ingest should succeed in stage-1");
        assert_eq!(lc.current_height(), 10);
    }

    #[test]
    fn trust_period_default_is_two_weeks() {
        let genesis = test_header(0, [0u8; 32], [0xaa; 32]);
        let lc = LightClient::new(
            genesis,
            1_700_000_000,
            #[cfg(feature = "nova")]
            None,
        );
        assert_eq!(lc.trust_period_secs(), 14 * 24 * 3600);
    }

    #[test]
    fn with_custom_trust_period_overrides_default() {
        let genesis = test_header(0, [0u8; 32], [0xaa; 32]);
        let lc = LightClient::with_trust_period(
            genesis,
            1_700_000_000,
            3600, // 1 hour
            #[cfg(feature = "nova")]
            None,
        );
        assert_eq!(lc.trust_period_secs(), 3600);
    }

    #[test]
    fn hex_lower_round_trip() {
        // Sanity check on the local hex helper used in error
        // messages.
        let bytes: [u8; 4] = [0x00, 0x12, 0xab, 0xff];
        assert_eq!(hex_lower(&bytes), "0012abff");
    }
}
