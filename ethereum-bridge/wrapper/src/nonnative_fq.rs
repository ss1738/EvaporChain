//! Non-native Pallas-Fq-in-BN254-Fr field gadget — sub-B-finish foundation.
//!
//! # Why a non-native field?
//!
//! The in-circuit Halo2 IPA verifier (sub-B-finish) needs to perform
//! Pallas Fq arithmetic. The wrapper circuit is constraint-formed over
//! BN254 Fr (the scalar field of the curve the Solidity verifier uses
//! via EIP-197). Pallas Fq ≠ BN254 Fr, so every Pallas-Fq operation
//! must be **emulated** as a sequence of BN254-Fr operations: each
//! Pallas-Fq element is encoded as a tuple of BN254-Fr limbs, and
//! addition / multiplication / equality are R1CS-checked over the
//! limbs.
//!
//! arkworks ships this emulation as `NonNativeFieldVar<TargetField,
//! BaseField>`. Each non-native multiplication is ~3,000 R1CS
//! constraints (CRT decomposition into ~5 Fr-multiplications per
//! Fq-multiplication). The Halo2 IPA verifier inside the wrapper will
//! issue ~10 IPA challenge rounds × ~3 Fq operations per round, so
//! ~150-200k constraints total — feasible inside a Powers-of-Tau
//! ceremony at 2^18 (260k constraints).
//!
//! # Why `ark_pallas::Fq`, not `pasta_curves::pallas::Base`?
//!
//! The two are the *same field* (same modulus, same arithmetic), but
//! they're two distinct Rust types from two unrelated libraries. The
//! Halo2 prover side uses `pasta_curves`; the arkworks wrapper side
//! uses `ark-pallas`. Bridging is a byte-level reinterpretation step
//! that sub-B-finish handles — the gadget itself doesn't care which
//! library produced the original witness.
//!
//! Using `ark_pallas::Fq` here means:
//!   1. arkworks's `NonNativeFieldVar` machinery sees a properly-typed
//!      `PrimeField` and can do its CRT decomposition over Fr.
//!   2. The gadget tests are fully arkworks-native — no
//!      `pasta_curves` <→ `ark-pallas` byte conversions needed yet.
//!
//! # What this scaffold ships
//!
//! - [`NonNativeFqVar`] — type alias `NonNativeFieldVar<Pallas::Fq, Bn254::Fr>`.
//! - [`alloc_nonnative_fq_witness`] — allocate a Pallas-Fq witness var.
//! - [`alloc_nonnative_fq_input`] — allocate as a *public input*
//!   (the Halo2 IPA verifier's challenge anchors will be public inputs).
//! - [`enforce_nonnative_fq_add`] — toy operation: `a + b == c` over
//!   non-native Fq. Proves the constraint system can witness, add,
//!   and compare emulated Fq values.
//! - Tests for satisfied-correct and unsatisfied-wrong constraint paths.
//!
//! # What sub-B-finish will add
//!
//! - Group operations: G1 (Pallas curve) point addition + scalar mul
//!   on top of the Fq non-native scaffold here.
//! - The Halo2 IPA verifier algorithm constraint-by-constraint:
//!   transcript reconstruction, challenge derivation, inner-product
//!   accumulator.
//! - Public-input binding so the wrapper's 4 anchors are committed in
//!   the IPA transcript hash.

use ark_bn254::Fr as Bn254Fr;
use ark_pallas::Fq as PallasFq;
use ark_r1cs_std::alloc::AllocVar;
use ark_r1cs_std::eq::EqGadget;
use ark_r1cs_std::fields::nonnative::NonNativeFieldVar;
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

/// Non-native Pallas-Fq variable allocated inside a BN254-Fr
/// constraint system. Each `NonNativeFqVar` decomposes into multiple
/// BN254-Fr limbs internally — arkworks handles the limb arithmetic
/// transparently.
pub type NonNativeFqVar = NonNativeFieldVar<PallasFq, Bn254Fr>;

/// Allocate a Pallas-Fq value as a **witness** variable in the
/// constraint system. Witness variables aren't visible to the verifier
/// — they're the private parts of the proof.
pub fn alloc_nonnative_fq_witness(
    cs: ConstraintSystemRef<Bn254Fr>,
    value: PallasFq,
) -> Result<NonNativeFqVar, SynthesisError> {
    NonNativeFqVar::new_witness(cs, || Ok(value))
}

/// Allocate a Pallas-Fq value as a **public input** in the constraint
/// system. Used for anchors that the verifier checks against
/// (challenge points, IPA transcript anchors, etc.).
///
/// Note: each non-native public input costs more limbs in the IC[]
/// table than a single BN254-Fr public input. The wrapper's 4 anchors
/// (state_root, key, value_commitment, params_fingerprint) are
/// BN254-Fr-native; only the IPA verifier's internal binding values
/// would use this allocator.
pub fn alloc_nonnative_fq_input(
    cs: ConstraintSystemRef<Bn254Fr>,
    value: PallasFq,
) -> Result<NonNativeFqVar, SynthesisError> {
    NonNativeFqVar::new_input(cs, || Ok(value))
}

/// Toy gadget: enforce `a + b == c` over non-native Pallas Fq.
///
/// This is the unit-test analogue of what the in-circuit Halo2 IPA
/// verifier will do thousands of times: combine emulated Fq values
/// via R1CS constraints. If this gadget compiles and passes its
/// constraint-satisfaction tests, the non-native toolchain is wired
/// correctly — sub-B-finish builds on top.
pub fn enforce_nonnative_fq_add(
    a: &NonNativeFqVar,
    b: &NonNativeFqVar,
    c: &NonNativeFqVar,
) -> Result<(), SynthesisError> {
    let sum = a + b;
    sum.enforce_equal(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ff::UniformRand;
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::rand::SeedableRng;

    fn seeded_rng() -> ark_std::rand::rngs::StdRng {
        ark_std::rand::rngs::StdRng::seed_from_u64(0xC0FFEE_u64)
    }

    /// Smoke test — allocate one witness, no operations. Pins that
    /// `NonNativeFieldVar<PallasFq, Bn254Fr>` actually constructs
    /// (catches version-skew / type-trait-bound regressions in arkworks).
    #[test]
    fn nonnative_fq_witness_allocates() {
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let val = PallasFq::from(42u64);
        let var = alloc_nonnative_fq_witness(cs.clone(), val).expect("alloc");
        // Just touching the var to silence unused-var lint.
        let _ = &var;
        assert!(
            cs.num_witness_variables() > 0,
            "allocating a non-native Fq must produce > 0 BN254-Fr limb witnesses"
        );
    }

    /// `a + b == c` is satisfied when c = a + b.
    #[test]
    fn nonnative_fq_add_satisfied_when_correct() {
        let mut rng = seeded_rng();
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();

        let a_val = PallasFq::rand(&mut rng);
        let b_val = PallasFq::rand(&mut rng);
        let c_val = a_val + b_val;

        let a = alloc_nonnative_fq_witness(cs.clone(), a_val).expect("alloc a");
        let b = alloc_nonnative_fq_witness(cs.clone(), b_val).expect("alloc b");
        let c = alloc_nonnative_fq_witness(cs.clone(), c_val).expect("alloc c");

        enforce_nonnative_fq_add(&a, &b, &c).expect("enforce");
        assert!(
            cs.is_satisfied().expect("is_satisfied"),
            "a + b == c must be satisfied for c = a + b"
        );
    }

    /// Mult sanity check — `a * b == c` for non-native Fq. Provides
    /// baseline confidence that arkworks's `NonNativeFieldVar` mult
    /// works for `PallasFq`-in-`Bn254Fr` (the next gadget layer up,
    /// pallas_g1, found cases where chained mults+adds break this).
    #[test]
    fn nonnative_fq_mul_satisfied_when_correct() {
        let mut rng = seeded_rng();
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();

        let a_val = PallasFq::rand(&mut rng);
        let b_val = PallasFq::rand(&mut rng);
        let c_val = a_val * b_val;

        let a = alloc_nonnative_fq_witness(cs.clone(), a_val).expect("alloc a");
        let b = alloc_nonnative_fq_witness(cs.clone(), b_val).expect("alloc b");
        let c = alloc_nonnative_fq_witness(cs.clone(), c_val).expect("alloc c");

        let product = &a * &b;
        product.enforce_equal(&c).expect("enforce");

        assert!(
            cs.is_satisfied().expect("is_satisfied"),
            "a * b == c must be satisfied for c = a * b"
        );
    }

    /// `a + b == c` is unsatisfied when c ≠ a + b. Critical — without
    /// this gate, the gadget would accept arbitrary triples.
    #[test]
    fn nonnative_fq_add_unsatisfied_when_wrong() {
        let mut rng = seeded_rng();
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();

        let a_val = PallasFq::rand(&mut rng);
        let b_val = PallasFq::rand(&mut rng);
        let wrong_c = a_val + b_val + PallasFq::from(1u64); // off by 1

        let a = alloc_nonnative_fq_witness(cs.clone(), a_val).expect("alloc a");
        let b = alloc_nonnative_fq_witness(cs.clone(), b_val).expect("alloc b");
        let c = alloc_nonnative_fq_witness(cs.clone(), wrong_c).expect("alloc c");

        enforce_nonnative_fq_add(&a, &b, &c).expect("enforce");
        assert!(
            !cs.is_satisfied().expect("is_satisfied"),
            "a + b == c must be UNSATISFIED for c = a + b + 1"
        );
    }

    /// Public-input allocator works — the IPA verifier's challenge
    /// anchors will go through this path. Each non-native input adds
    /// multiple BN254-Fr instance variables to the IC[] table.
    #[test]
    fn nonnative_fq_input_allocates_as_instance_var() {
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let n_inputs_before = cs.num_instance_variables();
        let val = PallasFq::from(0xCAFEu64);
        let _ = alloc_nonnative_fq_input(cs.clone(), val).expect("alloc input");
        let n_inputs_after = cs.num_instance_variables();
        assert!(
            n_inputs_after > n_inputs_before,
            "non-native Fq input must add ≥1 BN254-Fr instance var"
        );
    }

    /// Constraint count for a single Fq-add is non-zero and finite.
    /// Sub-B-finish capacity-plans on the ~thousands-per-Fq-mult cost,
    /// so this pins the baseline measurement.
    #[test]
    fn nonnative_fq_add_constraint_count_is_finite() {
        let mut rng = seeded_rng();
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();

        let a_val = PallasFq::rand(&mut rng);
        let b_val = PallasFq::rand(&mut rng);
        let c_val = a_val + b_val;

        let a = alloc_nonnative_fq_witness(cs.clone(), a_val).expect("alloc a");
        let b = alloc_nonnative_fq_witness(cs.clone(), b_val).expect("alloc b");
        let c = alloc_nonnative_fq_witness(cs.clone(), c_val).expect("alloc c");
        enforce_nonnative_fq_add(&a, &b, &c).expect("enforce");

        let n = cs.num_constraints();
        assert!(n > 0, "non-native Fq add must produce ≥1 constraint");
        // Sanity upper bound — a single Fq add should be hundreds, not
        // millions, of constraints. If this trips, arkworks has either
        // changed its decomposition strategy or our limb config is off.
        assert!(
            n < 10_000,
            "single Fq add somehow needs >= 10k constraints — investigate"
        );
    }
}
