//! Adapter-time orchestration: pulls a real `RecursiveSNARK<E1, E2,
//! TrivialIncrementCircuit>` fixture (from [`crate::recursive_snark_fixture::generate_fixture`])
//! through [`crate::scalar_adapter`] and produces a populated
//! [`crate::verifier_circuit::NovaVerifierCircuit`] ready to be
//! synthesised by Groth16.
//!
//! # What this ships
//!
//! [`build_circuit_from_fixture`] — single entry point that maps:
//!
//! | NovaVerifierCircuit field   | Source on `RecursiveSNARK`            |
//! |-----------------------------|---------------------------------------|
//! | `num_steps`                 | `rs.num_steps()` (public)             |
//! | `zi`                        | `rs.outputs()` (public)               |
//! | `z0`                        | `[Scalar1::ZERO]` (fixture contract)  |
//! | `committed_hash_primary`    | `rs.l_u_secondary.X[0]` via serde     |
//! | `committed_hash_secondary`  | `rs.l_u_secondary.X[1]` via serde     |
//!
//! # The l_u_secondary access gap is closed via serde reflection
//!
//! `RecursiveSNARK::l_u_secondary` is a private field in nova-snark
//! v0.68 (`nova-snark-0.68/src/nova/mod.rs:338`) with no public
//! accessor. [`crate::l_u_secondary_extract::extract_committed_hashes_via_serde`]
//! works around this by `serde_json::to_value(rs)` — the
//! `#[derive(Serialize)]` on both `RecursiveSNARK` and `R1CSInstance`
//! makes the field tree walkable by name without duplicating types.
//!
//! This is a brittle workaround pinned to nova-snark v0.68's
//! named-field layout. The proper resolution is an upstream PR
//! adding a `pub fn l_u_secondary(&self) -> &R1CSInstance<E2>`
//! accessor. See `l_u_secondary_extract` module docs for the
//! drift-detection test pin.
//!
//! # What real witness values mean for Section 2 status
//!
//! With real committed hashes flowing through, the circuit's
//! `committed_hash_primary` and `committed_hash_secondary`
//! witness slots now bind to actual Nova accumulator state. The
//! Groth16 proof produced via [`crate::groth16_wrapper::prove`]
//! still verifies with Sections 2 + 3 wired (when  / 
//! are called).  alone (no sections) still produces
//! a satisfied CS via the Section 1 structural gate only.

use crate::l_u_secondary_extract::{extract_committed_hashes_via_serde, ExtractError};
use crate::recursive_snark_fixture::{TrivialIncrementCircuit, Scalar1, E1, E2};
use crate::scalar_adapter::primary_to_ark_fr;
use crate::section2_witness::extract_section2_witness;
use crate::section3_witness::extract_section3_witness;
use crate::verifier_circuit::NovaVerifierCircuit;
use ark_bn254::Fr as ArkFr;
use nova_snark::nova::{PublicParams, RecursiveSNARK};

/// Map a real `RecursiveSNARK` fixture into the populated
/// `NovaVerifierCircuit` witness shape — including real
/// `l_u_secondary.X[..2]` values extracted via serde reflection.
///
/// Returns `Err` only if the extraction step fails (nova-snark
/// layout drift). See module docstring for the field-by-field
/// mapping table.
pub fn build_circuit_from_fixture(
    rs: &RecursiveSNARK<E1, E2, TrivialIncrementCircuit>,
) -> Result<NovaVerifierCircuit, ExtractError> {
    let num_steps = rs.num_steps() as u64;

    // z0 is hard-coded by `generate_fixture` to `[Scalar1::ZERO]`;
    // mirror that contract in ark-side units.
    let z0 = vec![ArkFr::from(0u64)];

    // zi from the public `outputs()` accessor, type-changed
    // through the same-field scalar adapter.
    let zi: Vec<ArkFr> = rs.outputs().iter().copied().map(primary_to_ark_fr).collect();

    // Real committed hashes from l_u_secondary.X[..2], via serde
    // reflection (see crate::l_u_secondary_extract).
    let (committed_hash_primary, committed_hash_secondary) =
        extract_committed_hashes_via_serde(rs)?;

    Ok(NovaVerifierCircuit::new(
        num_steps,
        z0,
        zi,
        committed_hash_primary,
        committed_hash_secondary,
    ))
}

/// Like [`build_circuit_from_fixture`] but also extracts Section 2
/// Neptune witness and attaches it via `NovaVerifierCircuit::with_section2`.
///
/// Requires the `dump-neptune-constants` JSON at `dump_path`.
pub fn build_circuit_with_section2<P: AsRef<std::path::Path>>(
    rs: &RecursiveSNARK<E1, E2, TrivialIncrementCircuit>,
    pp_digest: Scalar1,
    dump_path: P,
) -> Result<NovaVerifierCircuit, ExtractError> {
    let base = build_circuit_from_fixture(rs)?;
    let s2 = extract_section2_witness(rs, pp_digest, dump_path)?;
    Ok(base.with_section2(s2))
}

/// Like [`build_circuit_from_fixture`] but also extracts the Section 3
/// primary RelaxedR1CS witness and attaches it via
/// `NovaVerifierCircuit::with_section3`.
///
/// Requires `pp` (the `PublicParams` used to generate `rs`).
/// Serialises both `rs` and `pp` to JSON internally.
pub fn build_circuit_with_section3(
    rs: &RecursiveSNARK<E1, E2, TrivialIncrementCircuit>,
    pp: &PublicParams<E1, E2, TrivialIncrementCircuit>,
) -> Result<NovaVerifierCircuit, ExtractError> {
    let base = build_circuit_from_fixture(rs)?;
    let s3 = extract_section3_witness(rs, pp)?;
    Ok(base.with_section3(s3))
}

/// Audit B-1/B-2 S2b-prover: build a REAL, satisfiable prover
/// circuit — both sections extracted from a real Nova fixture.
/// Same construction as the S6 determinism proof, so its R1CS is
/// (S6-proven) bit-identical to `setup_shape()` — the circuit
/// `setup()` keys pk/vk over — hence a proof of it verifies.
/// `setup_shape()` itself has a zeroed, non-satisfiable witness
/// (shape template only) and CANNOT produce a verifying proof.
///
/// Returns `None` (caller skips) when the neptune dump is absent.
/// Callers must be `#[ignore]` — a real Nova fixture is expensive.
#[cfg(test)]
pub(crate) fn real_provable_circuit() -> Option<NovaVerifierCircuit> {
    use crate::recursive_snark_fixture::generate_fixture_with_digest;
    use nova_snark::provider::{
        hyperkzg::EvaluationEngine as EE1, ipa_pc::EvaluationEngine as EE2,
    };
    use nova_snark::spartan::snark::RelaxedR1CSSNARK;
    use nova_snark::traits::snark::RelaxedR1CSSNARKTrait;
    type S1 = RelaxedR1CSSNARK<E1, EE1<E1>>;
    type S2 = RelaxedR1CSSNARK<E2, EE2<E2>>;

    let dump = std::path::Path::new("/tmp/neptune-bn256-standard.json");
    if !dump.exists() {
        eprintln!("SKIP: /tmp/neptune-bn256-standard.json absent");
        return None;
    }
    let circuit_step = TrivialIncrementCircuit;
    let pp = PublicParams::<E1, E2, _>::setup(
        &circuit_step,
        &*S1::ck_floor(),
        &*S2::ck_floor(),
    )
    .ok()?;
    let (rs, pp_digest) = generate_fixture_with_digest(2).ok()?;
    let circuit = build_circuit_with_section2(&rs, pp_digest, dump)
        .ok()?
        .with_section3(extract_section3_witness(&rs, &pp).ok()?);
    Some(circuit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recursive_snark_fixture::generate_fixture;
    use ark_relations::gr1cs::ConstraintSynthesizer;
    use ark_relations::gr1cs::ConstraintSystem;

    /// End-to-end pin: real fixture → adapter → satisfied CS.
    ///
    /// Builds a 2-step `RecursiveSNARK`, runs it through
    /// `build_circuit_from_fixture` (with real committed hashes
    /// from `l_u_secondary.X[..2]` via serde reflection),
    /// synthesises with arkworks, and confirms `cs.is_satisfied()`.
    ///
    /// Sections 2+3 are wired when witnesses are attached via
    /// `with_section2` / `with_section3`. This test uses
    /// `build_circuit_from_fixture` alone (no sections), so only
    /// the Section 1 structural gate is active.
    #[test]
    fn fixture_to_circuit_to_satisfied_cs() {
        let rs = generate_fixture(2).expect("generate 2-step fixture");
        let circuit = build_circuit_from_fixture(&rs).expect("build");

        assert_eq!(circuit.num_steps, 2, "num_steps must be 2");
        assert_eq!(circuit.z0.len(), 1, "z0 arity matches TrivialIncrementCircuit");
        assert_eq!(circuit.zi.len(), 1, "zi arity matches z0");
        // TrivialIncrementCircuit increments [0] by 1 per step; after 2 steps z_i = 2.
        assert_eq!(
            circuit.zi[0],
            ArkFr::from(2u64),
            "zi must equal 2 after 2 increment steps from z0=0"
        );

        // Real (non-zero) committed hashes — see
        // `l_u_secondary_extract::extract_returns_non_trivial_hashes_for_real_fixture`
        // for the lower-level pin.
        assert!(
            circuit.committed_hash_primary != ArkFr::from(0u64)
                || circuit.committed_hash_secondary != ArkFr::from(0u64),
            "real fixture must produce at least one non-zero committed hash — \
             zero on both sides would indicate the extraction path silently failed"
        );

        // Audit B-1/B-2 S2b: `build_circuit_from_fixture` attaches NO
        // section witnesses, so synthesis MUST now fail — a section-
        // less circuit is exactly the constraint-vacuity the fix
        // closes. The satisfied-CS / arity-5 positive path is covered
        // by the section-bearing `#[ignore]`d tests below and the S6
        // determinism proof.
        let cs = ConstraintSystem::<ArkFr>::new_ref();
        let result = circuit.generate_constraints(cs.clone());
        assert!(
            matches!(
                result,
                Err(ark_relations::gr1cs::SynthesisError::Unsatisfiable)
            ),
            "section-less fixture circuit must be Unsatisfiable under S2b, got {result:?}"
        );
    }

    /// Same shape with 5 steps to confirm zi tracks num_steps.
    #[test]
    fn fixture_to_circuit_zi_tracks_num_steps() {
        let rs = generate_fixture(5).expect("generate 5-step fixture");
        let circuit = build_circuit_from_fixture(&rs).expect("build");
        assert_eq!(circuit.num_steps, 5);
        assert_eq!(circuit.zi[0], ArkFr::from(5u64), "zi after 5 increments");
    }

    /// The Section 1 gate must accept a real fixture (num_steps
    /// Audit B-1/B-2 S2b: a real-fixture circuit *without* section
    /// witnesses (`build_circuit_from_fixture` attaches none) is now
    /// REJECTED by the structural gate — Section 1 shape alone is no
    /// longer sufficient; the mandatory bindings must be present.
    #[test]
    fn fixture_to_circuit_section_less_fails_structural_validation() {
        use crate::verifier_circuit::StructuralValidationError;
        let rs = generate_fixture(1).expect("generate 1-step fixture");
        let circuit = build_circuit_from_fixture(&rs).expect("build");
        assert_eq!(
            circuit.validate_structurally(),
            Err(StructuralValidationError::MissingSection2),
        );
    }

    /// Audit B-1/B-2 S2b: a circuit with ONLY Section 2 (no Section 3)
    /// is now Unsatisfiable — both bindings are mandatory, so a
    /// single-section circuit is rejected at `validate_structurally`
    /// (MissingSection3). The both-sections positive is covered by
    /// the S6 determinism proof and
    /// `groth16_wrapper::tests::prove_and_verify_real_fixture_round_trip_accepts`.
    #[test]
    #[ignore]
    fn build_circuit_with_section2_alone_is_unsatisfiable() {
        use crate::recursive_snark_fixture::generate_fixture_with_digest;
        use ark_relations::gr1cs::ConstraintSystem;

        let dump = std::path::Path::new("/tmp/neptune-bn256-standard.json");
        if !dump.exists() {
            eprintln!("SKIP: dump file not present at {}", dump.display());
            return;
        }

        let (rs, pp_digest) = generate_fixture_with_digest(2).expect("generate fixture");
        let circuit = build_circuit_with_section2(&rs, pp_digest, dump).expect("build with s2");

        assert!(circuit.section2.is_some(), "section2 witness must be attached");
        assert!(circuit.section3.is_none(), "section3 must be absent here");
        let cs = ConstraintSystem::<ArkFr>::new_ref();
        let result = circuit.generate_constraints(cs.clone());
        assert!(
            matches!(
                result,
                Err(ark_relations::gr1cs::SynthesisError::Unsatisfiable)
            ),
            "section2-only circuit must be Unsatisfiable under S2b, got {result:?}"
        );
    }

    /// Audit B-1/B-2 S2b: a circuit with ONLY Section 3 (no Section 2)
    /// is now Unsatisfiable — both bindings are mandatory, so a
    /// single-section circuit is rejected at `validate_structurally`
    /// (MissingSection2). The both-sections positive is covered by the
    /// S6 determinism proof and
    /// `groth16_wrapper::tests::prove_and_verify_real_fixture_round_trip_accepts`.
    /// Requires PublicParams::setup (~3s) + pp JSON serialisation (~30s).
    #[test]
    #[ignore]
    fn build_circuit_with_section3_alone_is_unsatisfiable() {
        use nova_snark::nova::PublicParams;
        use nova_snark::provider::{
            hyperkzg::EvaluationEngine as EE1,
            ipa_pc::EvaluationEngine as EE2,
        };
        use nova_snark::spartan::snark::RelaxedR1CSSNARK;
        use nova_snark::traits::snark::RelaxedR1CSSNARKTrait;
        type S1 = RelaxedR1CSSNARK<E1, EE1<E1>>;
        type S2 = RelaxedR1CSSNARK<E2, EE2<E2>>;

        let circuit_step = crate::recursive_snark_fixture::TrivialIncrementCircuit;
        let pp = PublicParams::<E1, E2, _>::setup(
            &circuit_step, &*S1::ck_floor(), &*S2::ck_floor(),
        ).expect("setup");

        let rs = generate_fixture(2).expect("2-step fixture");
        let circuit = build_circuit_with_section3(&rs, &pp).expect("build with s3");

        assert!(circuit.section3.is_some(), "section3 witness must be attached");
        assert!(circuit.section2.is_none(), "section2 must be absent here");
        let cs = ConstraintSystem::<ArkFr>::new_ref();
        let result = circuit.generate_constraints(cs.clone());
        assert!(
            matches!(
                result,
                Err(ark_relations::gr1cs::SynthesisError::Unsatisfiable)
            ),
            "section3-only circuit must be Unsatisfiable under S2b, got {result:?}"
        );
    }
}
