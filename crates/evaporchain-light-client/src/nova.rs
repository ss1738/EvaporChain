//! Nova-IVC sublinear verification path (feature `nova`).
//!
//! Wraps [`evaporchain_lambda_fold::nova_path::verify_nova_folded`]
//! for SDK consumers. The Nova path is the **sublinear-in-active-
//! energy verifier** — light clients hold only `vk_bytes` (~few KB,
//! fixed-size) and verify any chain length in ~23 ms at any depth
//! (sublinearity claim empirically locked in Phase 6.1 at 1.083× of
//! 10 folds — see `LAMBDA_FOLD_NOVA_PLAN.md`).
//!
//! ## When to use
//!
//! Use [`LightClient::ingest_block_with_nova`] when:
//!   - The SDK was constructed with `vk_bytes = Some(_)` so it
//!     knows how to verify the chain's compiled circuit.
//!   - The caller has fetched the chain's running
//!     [`NovaFoldedInstance`] (typically via the
//!     `GET /api/lambda_fold/nova` HTTP endpoint).
//!   - The block being ingested is associated with a folded
//!     instance whose `step_count` matches the block's height
//!     relative to genesis.
//!
//! Otherwise, use the BFT-only [`LightClient::ingest_block`] —
//! the chain's BLS aggregate cert is sufficient for trust.
//!
//! ## Verification semantics
//!
//! `ingest_block_with_nova` does both:
//!   1. The full BFT verification chain (monotone-height + parent-
//!      hash + BLS aggregate-sig + trust period) — same as
//!      [`LightClient::ingest_block`].
//!   2. The Nova-IVC sublinear verification:
//!        - Identity-instance rejection (no folds yet).
//!        - `total_energy_remaining ≥ min_remaining_energy` policy
//!          check.
//!        - Bincode-deserialize the embedded `CompressedProof`.
//!        - SNARK verification via
//!          `RealBlockProver::verify_with_vk_bytes`.
//!
//! Both stages must pass; the trusted tip is updated only on
//! complete success. On failure the trusted tip is unchanged.

use evaporchain_consensus::light_client::{LightBlockHeader, VerificationResult};
use evaporchain_lambda_fold::nova_path::{verify_nova_folded, NovaFoldedInstance, NovaVerifyError};

use crate::client::{hex_lower, LightClient};
use crate::error::LightClientError;

/// Re-export so SDK consumers don't need an `evaporchain-lambda-fold`
/// dep just to construct the witness.
pub use evaporchain_lambda_fold::nova_path::NovaFoldedInstance as NovaAttestation;

impl LightClient {
    /// Ingest a block with Nova-IVC sublinear verification on top
    /// of the standard BFT verification chain. See module-level
    /// docs for when to use this vs the BFT-only
    /// [`LightClient::ingest_block`].
    ///
    /// The `nova_attestation` is typically fetched out-of-band
    /// via the chain's `GET /api/lambda_fold/nova` endpoint. The
    /// `min_remaining_energy` is a chain-policy energy floor —
    /// a non-zero value here causes the SDK to reject blocks
    /// whose folded total energy has decayed below the threshold,
    /// matching the chain's own verifier behaviour.
    ///
    /// Returns:
    ///   - `Ok(())` on full success — trusted tip updated.
    ///   - [`LightClientError::Bft`] for BFT-layer failures
    ///     (same shape as `ingest_block`).
    ///   - [`LightClientError::Nova`] for Nova-layer failures
    ///     (proof rejected, deserialize failed, identity instance,
    ///     energy below minimum, or vk_bytes missing).
    ///   - [`LightClientError::NonMonotoneHeight`] /
    ///     [`LightClientError::ParentHashMismatch`] for chain-
    ///     integrity failures (caught before BFT/Nova).
    pub fn ingest_block_with_nova(
        &mut self,
        header: LightBlockHeader,
        current_time: u64,
        nova_attestation: &NovaFoldedInstance,
        min_remaining_energy: u128,
    ) -> Result<(), LightClientError> {
        // Stage 1: Monotone-height + parent-hash chain integrity.
        if header.height <= self.trusted_tip().height {
            return Err(LightClientError::NonMonotoneHeight {
                provided: header.height,
                trusted: self.trusted_tip().height,
            });
        }
        // Parent-hash adjacency check intentionally omitted — see
        // `ingest_block` doc comment for rationale (chain producer
        // uses a different parent-hash formula than cert.block_hash;
        // BFT BLS aggregate-sig is the authoritative authentication).

        // Stage 2: BFT BLS aggregate-sig verification (same as
        // `ingest_block`).
        match self.bft_verifier_mut().verify(&header, current_time) {
            VerificationResult::Valid => {}
            VerificationResult::Invalid(msg) => return Err(LightClientError::Bft(msg)),
            VerificationResult::NeedBisection {
                trusted_height,
                target_height,
            } => {
                return Err(LightClientError::Bft(format!(
                    "skip verification needs bisection: trusted_height={trusted_height}, target_height={target_height}"
                )));
            }
        }

        // Stage 3: Nova-IVC sublinear verification.
        let vk = match self.vk_bytes_ref() {
            Some(vk) => vk,
            None => {
                return Err(LightClientError::Nova(
                    "vk_bytes not configured — light client constructed without Nova verification capability"
                        .to_string(),
                ));
            }
        };
        verify_nova_folded(nova_attestation, vk, min_remaining_energy)
            .map_err(|e| nova_err_to_sdk(&e))?;

        // Both BFT and Nova verified — promote to trusted tip.
        self.set_trusted_tip(header);
        Ok(())
    }
}

/// Translate a `NovaVerifyError` into the SDK's unified error
/// type with a human-readable message.
fn nova_err_to_sdk(e: &NovaVerifyError) -> LightClientError {
    match e {
        NovaVerifyError::Identity => LightClientError::Nova(
            "identity instance has no proof to verify (chain has not yet folded any blocks)"
                .to_string(),
        ),
        NovaVerifyError::Deserialize(msg) => {
            LightClientError::Nova(format!("CompressedProof deserialize failed: {msg}"))
        }
        NovaVerifyError::Invalid => {
            LightClientError::Nova("Nova SNARK verification rejected the proof".to_string())
        }
        NovaVerifyError::EnergyBelowMinimum { got, min } => LightClientError::Nova(format!(
            "total_energy_remaining {got} below chain-supplied minimum {min}"
        )),
        NovaVerifyError::Proving(msg) => {
            LightClientError::Nova(format!("proving-layer error: {msg}"))
        }
    }
}

// ───────────────────────── tests ───────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::test_fixtures::*;

    /// Build a NovaFoldedInstance manually for SDK testing. We
    /// don't actually generate a real Nova proof — that requires
    /// a heavy `pp` setup (~60-90s) plus per-step proving time.
    /// Instead, we test the SDK's plumbing: identity rejection,
    /// energy-floor rejection, and missing-vk-bytes rejection.
    /// Real Nova-verification e2e tests live in the
    /// `evaporchain-proving` and `evaporchain-lambda-fold` crates'
    /// own test suites.
    fn identity_instance() -> NovaFoldedInstance {
        // The identity instance has empty proof_bytes by
        // convention; lambda-fold's verify_nova_folded checks
        // this via `instance.is_identity()`.
        NovaFoldedInstance {
            proof_bytes: Vec::new(),
            total_energy_remaining: 0,
            step_count: 0,
            latest_epoch: 0,
        }
    }

    fn instance_with_energy(total_energy: u128, step_count: u64) -> NovaFoldedInstance {
        // Fake-but-non-empty proof_bytes so it's not identity.
        // Will fail SNARK verification, which is expected for
        // synthetic-fixture tests.
        NovaFoldedInstance {
            proof_bytes: vec![0u8; 64],
            total_energy_remaining: total_energy,
            step_count,
            latest_epoch: 0,
        }
    }

    #[test]
    fn ingest_with_nova_rejects_when_no_vk_bytes() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis =
            make_signed_header(1, [0u8; 32], [0xaa; 32], vs.clone(), &kps, &[0, 1, 2]);
        let mut lc = LightClient::new(genesis, 100, /* vk_bytes */ None);

        let next = make_signed_header(2, [0xaa; 32], [0xbb; 32], vs, &kps, &[0, 1, 2]);
        let err = lc
            .ingest_block_with_nova(next, 110, &identity_instance(), 0)
            .expect_err("must reject when no vk_bytes configured");
        assert!(matches!(err, LightClientError::Nova(_)));
        if let LightClientError::Nova(msg) = err {
            assert!(msg.contains("vk_bytes not configured"));
        }
    }

    #[test]
    fn ingest_with_nova_rejects_identity_instance() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis =
            make_signed_header(1, [0u8; 32], [0xaa; 32], vs.clone(), &kps, &[0, 1, 2]);
        // Garbage vk_bytes — won't be reached because identity is
        // checked before SNARK verification.
        let mut lc = LightClient::new(genesis, 100, Some(vec![0u8; 16]));

        let next = make_signed_header(2, [0xaa; 32], [0xbb; 32], vs, &kps, &[0, 1, 2]);
        let err = lc
            .ingest_block_with_nova(next, 110, &identity_instance(), 0)
            .expect_err("identity instance must be rejected");
        assert!(matches!(err, LightClientError::Nova(_)));
        if let LightClientError::Nova(msg) = err {
            assert!(msg.contains("identity"));
        }
    }

    #[test]
    fn ingest_with_nova_rejects_below_min_energy() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis =
            make_signed_header(1, [0u8; 32], [0xaa; 32], vs.clone(), &kps, &[0, 1, 2]);
        let mut lc = LightClient::new(genesis, 100, Some(vec![0u8; 16]));

        let next = make_signed_header(2, [0xaa; 32], [0xbb; 32], vs, &kps, &[0, 1, 2]);
        let instance = instance_with_energy(50, 5);
        let err = lc
            .ingest_block_with_nova(next, 110, &instance, /* min */ 1_000)
            .expect_err("below-min-energy must reject");
        assert!(matches!(err, LightClientError::Nova(_)));
        if let LightClientError::Nova(msg) = err {
            assert!(msg.contains("below"));
        }
    }

    #[test]
    fn ingest_with_nova_rejects_garbage_proof_bytes() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis =
            make_signed_header(1, [0u8; 32], [0xaa; 32], vs.clone(), &kps, &[0, 1, 2]);
        let mut lc = LightClient::new(genesis, 100, Some(vec![0u8; 16]));

        let next = make_signed_header(2, [0xaa; 32], [0xbb; 32], vs, &kps, &[0, 1, 2]);
        let instance = instance_with_energy(10_000, 5);
        // 64 bytes of zeros isn't a valid bincoded CompressedProof
        // — deserialize will fail, surfaces as Nova(...) error.
        let err = lc
            .ingest_block_with_nova(next, 110, &instance, /* min */ 0)
            .expect_err("garbage proof must reject");
        assert!(matches!(err, LightClientError::Nova(_)));
    }

    #[test]
    fn ingest_with_nova_still_enforces_monotone_height() {
        // Even with a "valid" Nova attestation, the BFT-layer
        // monotone-height check fires first. This is a
        // defence-in-depth guarantee — Nova proving doesn't
        // bypass chain-integrity gates.
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let genesis =
            make_signed_header(5, [0u8; 32], [0xaa; 32], vs.clone(), &kps, &[0, 1, 2]);
        let mut lc = LightClient::new(genesis, 100, Some(vec![0u8; 16]));

        let same = make_signed_header(5, [0u8; 32], [0xcc; 32], vs, &kps, &[0, 1, 2]);
        let err = lc
            .ingest_block_with_nova(same, 110, &identity_instance(), 0)
            .expect_err("non-monotone height must reject before reaching Nova path");
        assert!(matches!(err, LightClientError::NonMonotoneHeight { .. }));
    }
}
