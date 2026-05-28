//! Phase 4 of LAMBDA_FOLD_NOVA_PLAN — Nova-backed fold/verify path.
//!
//! Replaces the substrate blake3 hash chain with the real IVC pipeline
//! exposed by `evaporchain-proving::nova::RealBlockProver`. The
//! substrate path in `crate::fold` / `crate::verify` stays available
//! so consumers that don't need the heavy crypto build fast; Phase 5's
//! `lambda_fold_mode` governance flag chooses between them at runtime.
//!
//! ## Why a separate module
//!
//! The substrate `StepWitness { state_hash, step_energy,
//! observed_epoch }` is too narrow for the Nova path — folding a real
//! block needs the full `Block` plus the old/new `DualCommitment` plus
//! a `ThermodynamicWitness`. Rather than blow up `StepWitness`, the
//! Nova path takes the natural inputs directly.
//!
//! ## What this module ships
//!
//! - [`NovaFoldedInstance`] — Lambda-Fold-shaped accumulator backed by
//!   a serialized `CompressedSNARK` proof (replaces the blake3
//!   `acc_hash`). Carries `total_energy_remaining`, `step_count`,
//!   `latest_epoch` for hot-path queries that don't need verification.
//! - [`NovaFolder`] — wraps a `RealBlockProver` and tracks the running
//!   `(total_energy_remaining, step_count, latest_epoch)` outside the
//!   IVC for cheap reads.
//! - [`verify_nova_folded`] — light-client verifier. Takes
//!   `(NovaFoldedInstance, vk_bytes, min_remaining_energy)` and runs
//!   the bytes-shaped Nova check from `evaporchain-proving`.
//!
//! Performance budget per the plan: ~250-350 ms/fold under release on
//! a Mac Mini M4. The `RealBlockProver` instance must be constructed
//! once and reused across folds (Phase 3.1 contract).

use serde::{Deserialize, Serialize};
use thiserror::Error;

use evaporchain_energy_kernel::ChainLambda;
use evaporchain_proving::nova::{RealBlockProver, ThermodynamicWitness};
use evaporchain_proving::{CompressedProof, ProvingError};
use evaporchain_types::{Block, DualCommitment};

/// Lambda-Fold-shaped accumulator backed by a real Nova-folded
/// `CompressedSNARK` proof. The serialized proof bytes replace the
/// blake3 `acc_hash` from the substrate `FoldedInstance`. The
/// remaining fields stay so callers can read aggregate energy /
/// step count / latest epoch without verifying.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NovaFoldedInstance {
    /// Bincode-serialized `evaporchain_proving::CompressedProof`.
    /// Empty Vec is the identity (no folds yet).
    pub proof_bytes: Vec<u8>,
    /// Sum of all folded step energies, λ-decayed forward to
    /// `latest_epoch`. Mirrors `z[6]` in the Nova IVC; tracked here
    /// so callers don't need to verify a proof to read aggregate
    /// energy.
    pub total_energy_remaining: u128,
    /// Number of folded steps. Mirrors `z[7]` in the IVC.
    pub step_count: u64,
    /// Epoch at which the latest fold operation was performed.
    pub latest_epoch: u64,
}

impl NovaFoldedInstance {
    /// "No folds yet" identity. `proof_bytes` is empty; verifier
    /// must treat empty proof as the genesis identity.
    pub fn identity() -> Self {
        Self {
            proof_bytes: Vec::new(),
            total_energy_remaining: 0,
            step_count: 0,
            latest_epoch: 0,
        }
    }

    pub fn is_identity(&self) -> bool {
        self.step_count == 0 && self.proof_bytes.is_empty()
    }
}

impl Default for NovaFoldedInstance {
    fn default() -> Self {
        Self::identity()
    }
}

#[derive(Debug, Error)]
pub enum NovaFoldError {
    #[error("step observed_epoch {step} is before prev latest_epoch {prev}")]
    OutOfOrder { step: u64, prev: u64 },
    #[error("Nova proving error: {0}")]
    Proving(#[from] ProvingError),
    #[error("bincode error: {0}")]
    Bincode(String),
}

/// Stateful Nova folder. Holds the `RealBlockProver` (which holds
/// `pp` per Phase 3.1) and the chain-level energy/step accumulators.
/// Construct once per chain at genesis; reuse across all folds.
pub struct NovaFolder {
    prover: RealBlockProver,
    total_energy_remaining: u128,
    step_count: u64,
    latest_epoch: u64,
    /// L0-A (audit 2026-05-17): chain-level λ for aggregate energy decay.
    /// Pre-fix used the first object's per-object half_life (or hardcoded 100),
    /// which is wrong for the aggregate `total_energy_remaining` sum.
    chain_lambda: ChainLambda,
}

impl NovaFolder {
    /// Create a new folder at genesis with explicit chain λ. Performs
    /// the heavy `pp` setup; Phase 3.1 contract — call once per chain.
    pub fn new(genesis: &DualCommitment, chain_lambda: ChainLambda) -> Result<Self, NovaFoldError> {
        Ok(Self {
            prover: RealBlockProver::new(genesis)?,
            total_energy_remaining: 0,
            step_count: 0,
            latest_epoch: 0,
            chain_lambda: ChainLambda::default_genesis(),
        })
    }

    /// Variant that accepts an explicit ChainLambda (e.g. from governance).
    pub fn new_with_lambda(
        genesis: &DualCommitment,
        chain_lambda: ChainLambda,
    ) -> Result<Self, NovaFoldError> {
        Ok(Self {
            prover: RealBlockProver::new(genesis)?,
            total_energy_remaining: 0,
            step_count: 0,
            latest_epoch: 0,
            chain_lambda,
        })
    }

    /// Fold one real block into the running IVC + energy state.
    /// Returns the updated `NovaFoldedInstance` (with a freshly
    /// generated compressed proof). The folder retains its internal
    /// state for the next call.
    ///
    /// Note: producing a `CompressedProof` after each fold is the
    /// safe shape for now; a future optimization is to compress only
    /// at epoch boundaries / on demand.
    pub fn fold_block(
        &mut self,
        block: &Block,
        old_state: &DualCommitment,
        new_state: &DualCommitment,
        thermo: &ThermodynamicWitness,
        observed_epoch: u64,
        step_energy: u64,
    ) -> Result<NovaFoldedInstance, NovaFoldError> {
        if observed_epoch < self.latest_epoch {
            return Err(NovaFoldError::OutOfOrder {
                step: observed_epoch,
                prev: self.latest_epoch,
            });
        }

        self.prover
            .fold_real_block_with_witness(block, old_state, new_state, thermo)?;

        // Energy tracking outside the IVC: λ-decay the running total
        // forward to `observed_epoch`, then add step_energy. The IVC
        // gadget enforces the same arithmetic in-circuit; the
        // outside-the-IVC mirror here is for cheap reads.
        let elapsed = observed_epoch - self.latest_epoch;
        let decayed_prev = if elapsed == 0 || self.total_energy_remaining == 0 {
            self.total_energy_remaining
        } else {
            let prev_u64 = self.total_energy_remaining.min(u64::MAX as u128) as u64;
            // L0-A: use chain-level λ, not first object's per-object half_life.
            let decayed = evaporchain_types::energy_at_epoch(
                prev_u64,
                self.chain_lambda.half_life(),
                elapsed,
            );
            decayed as u128
        };
        // Suppress unused warning — `thermo` is still required by
        // `fold_real_block_with_witness` even though we no longer
        // consult its `object_energies` for the running total.
        let _ = thermo;
        self.total_energy_remaining = decayed_prev.saturating_add(step_energy as u128);
        self.step_count = self.step_count.saturating_add(1);
        self.latest_epoch = observed_epoch;

        let proof = self.prover.get_proof()?;
        let proof_bytes = bincode::serialize(&proof)
            .map_err(|e| NovaFoldError::Bincode(format!("CompressedProof serialize: {:?}", e)))?;

        Ok(NovaFoldedInstance {
            proof_bytes,
            total_energy_remaining: self.total_energy_remaining,
            step_count: self.step_count,
            latest_epoch: self.latest_epoch,
        })
    }

    /// Phase 4 — preprocessed `vk` bytes for distributing to light
    /// clients. Lambda-Fold's verifier needs only this + the
    /// `NovaFoldedInstance` to decide validity.
    pub fn vk_bytes(&self) -> Result<Vec<u8>, NovaFoldError> {
        Ok(self.prover.vk_bytes()?)
    }

    pub fn step_count(&self) -> u64 {
        self.step_count
    }

    pub fn total_energy_remaining(&self) -> u128 {
        self.total_energy_remaining
    }

    pub fn latest_epoch(&self) -> u64 {
        self.latest_epoch
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum NovaVerifyError {
    #[error("identity instance has no proof to verify")]
    Identity,
    #[error("CompressedProof deserialize failed: {0}")]
    Deserialize(String),
    #[error("Nova SNARK verification rejected the proof")]
    Invalid,
    #[error("total_energy_remaining {got} below the chain-supplied minimum {min}")]
    EnergyBelowMinimum { got: u128, min: u128 },
    #[error("Nova proving error: {0}")]
    Proving(String),
    /// SUB-N9 (audit 2026-05-15): proof_bytes exceeds the verify-
    /// time cap. Without a pre-flight, a multi-MB blob over the
    /// wire forces `bincode::deserialize` to allocate before failing
    /// — a light-client DoS vector.
    #[error("proof_bytes length {got} exceeds cap {cap}")]
    ProofTooLarge { got: usize, cap: usize },
}

/// SUB-N9: cap on `NovaFoldedInstance.proof_bytes` length passed
/// to `verify_nova_folded`. Real Nova compressed proofs at the
/// arity-8 / k=11 IPA shape used by EvaporChain are well under
/// 256 KiB; this ceiling is generous and bounds the deserialize
/// allocation under any adversarial input.
pub const MAX_NOVA_PROOF_BYTES: usize = 256 * 1024;

/// Phase 4.4 — light-client verifier. Takes only the
/// `NovaFoldedInstance` + the preprocessed `vk_bytes` + the chain
/// energy floor. No `RealBlockProver`, no `pp`.
pub fn verify_nova_folded(
    instance: &NovaFoldedInstance,
    vk_bytes: &[u8],
    min_remaining_energy: u128,
) -> Result<(), NovaVerifyError> {
    if instance.is_identity() {
        return Err(NovaVerifyError::Identity);
    }

    if instance.total_energy_remaining < min_remaining_energy {
        return Err(NovaVerifyError::EnergyBelowMinimum {
            got: instance.total_energy_remaining,
            min: min_remaining_energy,
        });
    }

    // SUB-N9: pre-flight length cap so an oversized blob fails fast
    // instead of forcing bincode to allocate before erroring.
    if instance.proof_bytes.len() > MAX_NOVA_PROOF_BYTES {
        return Err(NovaVerifyError::ProofTooLarge {
            got: instance.proof_bytes.len(),
            cap: MAX_NOVA_PROOF_BYTES,
        });
    }
    let proof: CompressedProof = bincode::deserialize(&instance.proof_bytes)
        .map_err(|e| NovaVerifyError::Deserialize(format!("{:?}", e)))?;

    let valid =
        RealBlockProver::verify_with_vk_bytes(&proof, instance.step_count as usize, vk_bytes)
            .map_err(|e| NovaVerifyError::Proving(format!("{:?}", e)))?;

    if valid {
        Ok(())
    } else {
        Err(NovaVerifyError::Invalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_dual_commitment(seed: u8, epoch: u64) -> DualCommitment {
        DualCommitment {
            verkle_root: [seed; 32],
            mmr_root: [seed.wrapping_add(1); 32],
            epoch,
            active_count: 0,
            ghost_count: 0,
        }
    }

    fn make_block(number: u64, epoch: u64) -> Block {
        Block {
            number,
            epoch,
            parent_hash: [0u8; 32],
            state_root: [0u8; 32],
            transactions: vec![],
            timestamp: 0,
            chain_id: String::new(),
            producer_id: None,
            vrf_output: None,
            vrf_proof: None,
            data_root: None,
            da_row_roots: vec![],
            da_col_roots: vec![],
            blob_commitments: vec![],
            da_certificate: None,
            commit_certificate: None,
            nova_proof: None,
            anchor_hash: None,
            state_function_commitment: None,
            oracle_state_root: None,
            shard_count: None,
            protocol_version: 0,
            state_root_version: 0,
            submit_epoch_hints: vec![],
            parents: vec![],
            post_state_root: None,
        }
    }

    /// Phase 4 smoke test — three Nova folds end-to-end with the
    /// light-client verify path. Closes the substrate→Nova migration
    /// for the lambda-fold crate.
    #[test]
    fn nova_fold_three_blocks_and_verify() {
        let genesis = make_dual_commitment(0, 0);
        let mut folder = NovaFolder::new(&genesis, ChainLambda::default_genesis())
            .expect("folder setup failed");

        let mut last_state = genesis.clone();
        let mut last_instance = NovaFoldedInstance::identity();

        for i in 1..=3u64 {
            let block = make_block(i, i);
            let new_state = make_dual_commitment(i as u8, i);
            let thermo = ThermodynamicWitness {
                object_energies: vec![(0, 0, 100)],
                evaporation_nullifiers: vec![],
            };
            last_instance = folder
                .fold_block(&block, &last_state, &new_state, &thermo, i, 1000)
                .expect("nova fold failed");
            last_state = new_state;
        }

        assert_eq!(last_instance.step_count, 3);
        assert_eq!(last_instance.latest_epoch, 3);
        assert!(!last_instance.proof_bytes.is_empty());

        let vk_bytes = folder.vk_bytes().expect("vk_bytes failed");
        verify_nova_folded(&last_instance, &vk_bytes, 0).expect("light-client verify failed");
    }

    #[test]
    fn nova_verify_rejects_identity() {
        let identity = NovaFoldedInstance::identity();
        let err = verify_nova_folded(&identity, b"unused", 0).unwrap_err();
        assert_eq!(err, NovaVerifyError::Identity);
    }

    #[test]
    fn nova_verify_rejects_below_energy_floor() {
        let genesis = make_dual_commitment(0, 0);
        let mut folder = NovaFolder::new(&genesis, ChainLambda::default_genesis())
            .expect("folder setup failed");
        let block = make_block(1, 1);
        let new_state = make_dual_commitment(1, 1);
        let thermo = ThermodynamicWitness {
            object_energies: vec![(0, 0, 100)],
            evaporation_nullifiers: vec![],
        };
        let inst = folder
            .fold_block(&block, &genesis, &new_state, &thermo, 1, 1000)
            .expect("nova fold failed");

        let vk_bytes = folder.vk_bytes().expect("vk_bytes failed");
        let err = verify_nova_folded(&inst, &vk_bytes, 10_000_000).unwrap_err();
        assert!(matches!(err, NovaVerifyError::EnergyBelowMinimum { .. }));
    }
}
