//! Nova IVC proving engine for EvaporChain.
//!
//! Adapts the fold-a-block prototype circuit to work with real block data
//! (state roots, transaction counts, evaporation counts, epochs).

use core::marker::PhantomData;
use std::time::Instant;

use nova_snark::{
    frontend::{num::AllocatedNum, ConstraintSystem, SynthesisError},
    nova::{CompressedSNARK, PublicParams, RecursiveSNARK},
    provider::{Bn256EngineKZG, GrumpkinEngine},
    traits::{circuit::StepCircuit, snark::RelaxedR1CSSNARKTrait, Engine, Group},
};

use crate::{CompressedProof, ProvingEngine, ProvingError};
use evaporchain_types::Block;

// ─────────────────────────── Type Aliases ─────────────────────────────────

type E1 = Bn256EngineKZG;
type E2 = GrumpkinEngine;
type EE1 = nova_snark::provider::hyperkzg::EvaluationEngine<E1>;
type EE2 = nova_snark::provider::ipa_pc::EvaluationEngine<E2>;
type S1 = nova_snark::spartan::snark::RelaxedR1CSSNARK<E1, EE1>;
type S2 = nova_snark::spartan::snark::RelaxedR1CSSNARK<E2, EE2>;

type Scalar = <E1 as Engine>::Scalar;
type G1 = <E1 as Engine>::GE;

// ─────────────────────────── Block Step Circuit ──────────────────────────

/// Witness data for one block's state transition.
#[derive(Clone, Debug)]
struct BlockStepWitness {
    /// Truncated new state root (first 8 bytes as u64).
    new_state_hash: u64,
    /// Number of transactions in this block.
    tx_count: u64,
    /// Number of objects evaporated in this block.
    evaporation_count: u64,
}

/// Nova step circuit for a single block state transition.
///
/// IVC state vector (arity = 2): `[state_hash, epoch]`
///
/// Constraints per step:
///   1. epoch_new = epoch_old + 1
///   2. state_hash binding (new_state_hash * 1 = new_state_hash)
///   3. tx_count * tx_count = tx_count^2 (binds transaction batch)
///   4. evap_count binding
#[derive(Clone, Debug)]
struct BlockStepCircuit<G: Group> {
    witness: BlockStepWitness,
    _p: PhantomData<G>,
}

impl<G: Group> BlockStepCircuit<G> {
    fn new(new_state_hash: u64, tx_count: u64, evaporation_count: u64) -> Self {
        Self {
            witness: BlockStepWitness {
                new_state_hash,
                tx_count,
                evaporation_count,
            },
            _p: PhantomData,
        }
    }

    /// Create a dummy circuit for public parameter setup.
    fn dummy() -> Self {
        Self::new(0, 0, 0)
    }
}

impl<G: Group> StepCircuit<G::Scalar> for BlockStepCircuit<G> {
    fn arity(&self) -> usize {
        2 // [state_hash, epoch]
    }

    fn synthesize<CS: ConstraintSystem<G::Scalar>>(
        &self,
        cs: &mut CS,
        z: &[AllocatedNum<G::Scalar>],
    ) -> Result<Vec<AllocatedNum<G::Scalar>>, SynthesisError> {
        let current_epoch = &z[1];

        // === 1. Epoch increment: new_epoch = current_epoch + 1 ===
        let new_epoch = AllocatedNum::alloc(cs.namespace(|| "new_epoch"), || {
            let e = current_epoch
                .get_value()
                .ok_or(SynthesisError::AssignmentMissing)?;
            Ok(e + G::Scalar::from(1u64))
        })?;
        cs.enforce(
            || "epoch_inc",
            |lc| lc + new_epoch.get_variable(),
            |lc| lc + CS::one(),
            |lc| lc + current_epoch.get_variable() + CS::one(),
        );

        // === 2. State hash binding ===
        let new_state_hash = AllocatedNum::alloc(cs.namespace(|| "state_hash"), || {
            Ok(G::Scalar::from(self.witness.new_state_hash))
        })?;
        cs.enforce(
            || "state_hash_bind",
            |lc| lc + new_state_hash.get_variable(),
            |lc| lc + CS::one(),
            |lc| lc + new_state_hash.get_variable(),
        );

        // === 3. Transaction count binding (vol * vol = vol^2) ===
        let tx_count = AllocatedNum::alloc(cs.namespace(|| "tx_count"), || {
            Ok(G::Scalar::from(self.witness.tx_count))
        })?;
        let tx_sq = AllocatedNum::alloc(cs.namespace(|| "tx_sq"), || {
            let v = tx_count
                .get_value()
                .ok_or(SynthesisError::AssignmentMissing)?;
            Ok(v * v)
        })?;
        cs.enforce(
            || "tx_bind",
            |lc| lc + tx_count.get_variable(),
            |lc| lc + tx_count.get_variable(),
            |lc| lc + tx_sq.get_variable(),
        );

        // === 4. Evaporation count binding ===
        let evap_count = AllocatedNum::alloc(cs.namespace(|| "evap_count"), || {
            Ok(G::Scalar::from(self.witness.evaporation_count))
        })?;
        cs.enforce(
            || "evap_bind",
            |lc| lc + evap_count.get_variable(),
            |lc| lc + CS::one(),
            |lc| lc + evap_count.get_variable(),
        );

        Ok(vec![new_state_hash, new_epoch])
    }
}

// ─────────────────────────── Helpers ─────────────────────────────────────

/// Truncate a 32-byte state root to u64 for circuit use.
fn state_root_to_u64(root: &[u8; 32]) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&root[..8]);
    u64::from_le_bytes(buf)
}

// ─────────────────────────── NovaProver ──────────────────────────────────

/// Nova IVC proving engine that folds each block's state transition.
pub struct NovaProver {
    pp: PublicParams<E1, E2, BlockStepCircuit<G1>>,
    recursive_snark: Option<RecursiveSNARK<E1, E2, BlockStepCircuit<G1>>>,
    z0: Vec<Scalar>,
    num_folded: usize,
    last_fold_time_us: u64,
}

impl NovaProver {
    /// Create a new NovaProver. This performs the (expensive) public parameter setup.
    pub fn new(genesis_state_root: [u8; 32]) -> Result<Self, ProvingError> {
        let dummy = BlockStepCircuit::<G1>::dummy();

        let pp = PublicParams::<E1, E2, BlockStepCircuit<G1>>::setup(
            &dummy,
            &*S1::ck_floor(),
            &*S2::ck_floor(),
        )
        .map_err(|e| ProvingError::FoldingFailed(format!("PP setup failed: {:?}", e)))?;

        let z0 = vec![
            Scalar::from(state_root_to_u64(&genesis_state_root)),
            Scalar::from(0u64), // epoch starts at 0
        ];

        Ok(Self {
            pp,
            recursive_snark: None,
            z0,
            num_folded: 0,
            last_fold_time_us: 0,
        })
    }

    /// Number of R1CS constraints in the primary circuit.
    pub fn num_constraints(&self) -> (usize, usize) {
        self.pp.num_constraints()
    }
}

impl ProvingEngine for NovaProver {
    fn fold_block(
        &mut self,
        block: &Block,
        _old_state_root: [u8; 32],
        new_state_root: [u8; 32],
    ) -> Result<(), ProvingError> {
        let circuit = BlockStepCircuit::<G1>::new(
            state_root_to_u64(&new_state_root),
            block.transactions.len() as u64,
            0, // evaporation count not tracked on Block; caller can extend
        );

        let start = Instant::now();

        if let Some(snark) = &mut self.recursive_snark {
            snark.prove_step(&self.pp, &circuit).map_err(|e| {
                ProvingError::FoldingFailed(format!("prove_step: {:?}", e))
            })?;
        } else {
            // First fold: create the RecursiveSNARK
            let mut snark =
                RecursiveSNARK::<E1, E2, BlockStepCircuit<G1>>::new(&self.pp, &circuit, &self.z0)
                    .map_err(|e| {
                        ProvingError::FoldingFailed(format!("RecursiveSNARK::new: {:?}", e))
                    })?;
            snark.prove_step(&self.pp, &circuit).map_err(|e| {
                ProvingError::FoldingFailed(format!("prove_step (first): {:?}", e))
            })?;
            self.recursive_snark = Some(snark);
        }

        self.last_fold_time_us = start.elapsed().as_micros() as u64;
        self.num_folded += 1;
        Ok(())
    }

    fn get_proof(&self) -> Result<CompressedProof, ProvingError> {
        let snark = self
            .recursive_snark
            .as_ref()
            .ok_or(ProvingError::NoBlocksFolded)?;

        // Verify the recursive SNARK before compressing
        snark
            .verify(&self.pp, self.num_folded, &self.z0)
            .map_err(|e| {
                ProvingError::CompressionFailed(format!("recursive verify failed: {:?}", e))
            })?;

        // Compress to succinct SNARK
        let (pk, _vk) = CompressedSNARK::<_, _, _, S1, S2>::setup(&self.pp)
            .map_err(|e| ProvingError::CompressionFailed(format!("CS setup: {:?}", e)))?;

        let compressed = CompressedSNARK::<_, _, _, S1, S2>::prove(&self.pp, &pk, snark)
            .map_err(|e| ProvingError::CompressionFailed(format!("CS prove: {:?}", e)))?;

        let proof_bytes = bincode::serialize(&compressed)
            .map_err(|e| ProvingError::CompressionFailed(format!("serialize: {:?}", e)))?;

        let z0_bytes = bincode::serialize(&self.z0)
            .map_err(|e| ProvingError::CompressionFailed(format!("z0 serialize: {:?}", e)))?;

        Ok(CompressedProof {
            proof_bytes,
            num_steps: self.num_folded,
            z0_bytes,
        })
    }

    fn verify_proof(
        &self,
        proof: &CompressedProof,
        num_blocks: usize,
        genesis_state: [u8; 32],
    ) -> Result<bool, ProvingError> {
        let compressed: CompressedSNARK<E1, E2, BlockStepCircuit<G1>, S1, S2> =
            bincode::deserialize(&proof.proof_bytes)
                .map_err(|e| ProvingError::VerificationFailed(format!("deserialize: {:?}", e)))?;

        let z0: Vec<Scalar> = bincode::deserialize(&proof.z0_bytes)
            .map_err(|e| ProvingError::VerificationFailed(format!("z0 deserialize: {:?}", e)))?;

        // Verify z0 matches expected genesis state
        let expected_z0_hash = Scalar::from(state_root_to_u64(&genesis_state));
        if z0.first() != Some(&expected_z0_hash) {
            return Ok(false);
        }

        let (_pk, vk) = CompressedSNARK::<_, _, _, S1, S2>::setup(&self.pp)
            .map_err(|e| ProvingError::VerificationFailed(format!("CS setup: {:?}", e)))?;

        match compressed.verify(&vk, num_blocks, &z0) {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    fn accumulator_size(&self) -> usize {
        // Approximate size of the running RecursiveSNARK
        match &self.recursive_snark {
            Some(snark) => {
                // Rough estimate: serialize to get actual size
                bincode::serialize(snark).map(|b| b.len()).unwrap_or(0)
            }
            None => 0,
        }
    }

    fn num_blocks_folded(&self) -> usize {
        self.num_folded
    }

    fn last_fold_time_us(&self) -> u64 {
        self.last_fold_time_us
    }
}

// ─────────────────────────── Tests ───────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_types::Block;

    fn dummy_block(num: u64, epoch: u64) -> Block {
        Block {
            number: num,
            epoch,
            parent_hash: [0u8; 32],
            state_root: [0u8; 32],
            transactions: vec![],
            timestamp: 0,
        }
    }

    fn make_state_root(seed: u8) -> [u8; 32] {
        let mut root = [0u8; 32];
        root[0] = seed;
        root[1] = seed.wrapping_mul(7);
        root
    }

    #[test]
    fn test_nova_prover_folds_blocks() {
        let genesis = [0u8; 32];
        let mut prover = NovaProver::new(genesis).expect("setup failed");

        // Fold 3 blocks (keep it small for CI speed)
        for i in 1..=3u64 {
            let block = dummy_block(i, i);
            let old_root = make_state_root((i - 1) as u8);
            let new_root = make_state_root(i as u8);
            prover
                .fold_block(&block, old_root, new_root)
                .expect("fold failed");
        }

        assert_eq!(prover.num_blocks_folded(), 3);
        assert!(prover.last_fold_time_us() > 0);
        assert!(prover.accumulator_size() > 0);
    }

    #[test]
    fn test_nova_prover_proof_roundtrip() {
        let genesis = [0u8; 32];
        let mut prover = NovaProver::new(genesis).expect("setup failed");

        for i in 1..=3u64 {
            let block = dummy_block(i, i);
            prover
                .fold_block(&block, make_state_root((i - 1) as u8), make_state_root(i as u8))
                .expect("fold failed");
        }

        let proof = prover.get_proof().expect("get_proof failed");
        assert_eq!(proof.num_steps, 3);
        assert!(!proof.proof_bytes.is_empty());

        // Verify with correct genesis
        let valid = prover
            .verify_proof(&proof, 3, genesis)
            .expect("verify failed");
        assert!(valid);
    }

    #[test]
    fn test_nova_prover_rejects_wrong_genesis() {
        let genesis = [0u8; 32];
        let mut prover = NovaProver::new(genesis).expect("setup failed");

        for i in 1..=2u64 {
            let block = dummy_block(i, i);
            prover
                .fold_block(&block, make_state_root((i - 1) as u8), make_state_root(i as u8))
                .expect("fold failed");
        }

        let proof = prover.get_proof().expect("get_proof failed");

        // Verify with wrong genesis should fail
        let wrong_genesis = [0xFFu8; 32];
        let result = prover
            .verify_proof(&proof, 2, wrong_genesis)
            .expect("verify call failed");
        assert!(!result);
    }
}
