//! Engine type aliases for the Verkle IVC circuit.
//!
//! We use Bn256EngineKZG / GrumpkinEngine (BN254 + Grumpkin) because:
//!   - CompressedSNARK with HyperKZG produces a proof verifiable in Solidity
//!     via a standard Groth16/Spartan verifier library.
//!   - The existing EvaporChain Nova proving engine uses the same pair, so
//!     public-parameter ceremonies and test infrastructure are shared.
//!
//! The 32-byte Verkle commitment hashes (Pallas x-coordinates) are interpreted
//! as BN254 scalar field elements after masking the top 2 bits — the same
//! mask used by `verkle::bytes_to_scalar` for Pallas Fq.  The field orders
//! are both ~2^254, so the mask is safe.

use nova_snark::{
    nova::{PublicParams, RecursiveSNARK},
    provider::{Bn256EngineKZG, GrumpkinEngine},
    provider::{hyperkzg::EvaluationEngine as HyperKZG, ipa_pc::EvaluationEngine as IPA},
    spartan::snark::RelaxedR1CSSNARK,
    traits::Engine,
};

use crate::circuit::VerkleStepCircuit;

pub type EngineE1 = Bn256EngineKZG;
pub type EngineE2 = GrumpkinEngine;

type EE1 = HyperKZG<EngineE1>;
type EE2 = IPA<EngineE2>;

pub type S1 = RelaxedR1CSSNARK<EngineE1, EE1>;
pub type S2 = RelaxedR1CSSNARK<EngineE2, EE2>;

pub type Scalar = <EngineE1 as Engine>::Scalar;
pub type G1 = <EngineE1 as Engine>::GE;

pub type VerklePublicParams = PublicParams<EngineE1, EngineE2, VerkleStepCircuit<G1>>;
pub type VerkleRecursiveSNARK = RecursiveSNARK<EngineE1, EngineE2, VerkleStepCircuit<G1>>;
