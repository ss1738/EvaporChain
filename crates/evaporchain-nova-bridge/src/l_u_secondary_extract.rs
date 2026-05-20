//! Extract the two committed hashes `l_u_secondary.X[..2]` from a
//! `RecursiveSNARK` via serde JSON reflection.
//!
//! # Why this exists
//!
//! `RecursiveSNARK` declares `l_u_secondary: R1CSInstance<E2>` as a
//! private field (nova-snark-0.68 `src/nova/mod.rs:338`) with no
//! public accessor. The Section 2 in-circuit transcript check
//! needs the two scalars `l_u_secondary.X[0]` and
//! `l_u_secondary.X[1]` as witness values. The proper resolution
//! is an upstream PR adding `pub fn l_u_secondary(&self) -> &R1CSInstance<E2>`;
//! until that lands, this module performs the extraction via
//! `serde_json::to_value`, which works because `RecursiveSNARK`
//! and `R1CSInstance` both have `#[derive(Serialize)]` with named
//! fields.
//!
//! # JSON shape (verified empirically against nova-snark 0.68)
//!
//! ```text
//! {
//!   "z0": [...],
//!   "r_W_primary": {...},
//!   "r_U_primary": {...},
//!   "ri_primary": "0x...",
//!   "r_W_secondary": {...},
//!   "r_U_secondary": {...},
//!   "ri_secondary": "0x...",
//!   "l_w_secondary": {...},
//!   "l_u_secondary": {
//!     "comm_W": {...},
//!     "X": ["0x...", "0x..."]      ← here
//!   },
//!   "i": <int>,
//!   "zi": [...],
//!   "_p": null
//! }
//! ```
//!
//! `R1CSInstance.X` serializes through `#[serde_as(as = "Vec<EvmCompatSerde>")]`,
//! which emits each scalar as a `0x`-prefixed big-endian hex string.
//!
//! # Brittleness
//!
//! - If a future nova-snark release renames `l_u_secondary` or
//!   the `X` field, this code breaks at runtime (typed errors).
//! - If `EvmCompatSerde`'s string format changes (e.g. switches
//!   from hex to base64), the parser needs updating.
//! - The pinned `nova_snark::nova::RecursiveSNARK<E1, E2, TrivialIncrementCircuit>`
//!   type couples us to the `recursive_snark_fixture` step circuit;
//!   generalising to other step circuits is straightforward (the
//!   JSON path is identical).
//!
//! Tests pin the JSON shape so a future nova-snark bump that
//! changes the layout fires loudly.

use crate::recursive_snark_fixture::{TrivialIncrementCircuit, E1, E2};
use crate::scalar_adapter::{
    ark_fr_to_primary, primary_to_ark_fr, secondary_to_ark_fr_lossy, SecondaryScalar,
};
use ark_bn254::Fr as ArkFr;
use ff::PrimeField;
use nova_snark::nova::{PublicParams, RecursiveSNARK};

/// Errors surfaced by [`extract_committed_hashes_via_serde`].
#[derive(Debug, Clone, thiserror::Error)]
pub enum ExtractError {
    /// `serde_json::to_value` failed on the RecursiveSNARK.
    #[error("serde_json::to_value failed: {0}")]
    Serialize(String),

    /// The JSON tree did not have the expected
    /// `l_u_secondary.X[..]` path. Likely a nova-snark version bump
    /// that renamed something.
    #[error("expected JSON path `l_u_secondary.X` missing or wrong shape (nova-snark layout drift?)")]
    MissingPath,

    /// `X` had fewer than 2 entries. The Section 2 contract
    /// requires exactly 2 committed hashes.
    #[error("l_u_secondary.X had {0} entries; expected ≥ 2")]
    TooFewHashes(usize),

    /// One of the hex strings couldn't be parsed as a 32-byte
    /// secondary-side scalar.
    #[error("could not parse hex scalar at l_u_secondary.X[{index}]: {reason}")]
    HexParseFailed {
        /// Index in `X` that failed.
        index: usize,
        /// Underlying parser error message.
        reason: String,
    },

    /// A serde operation failed during Section 3 extraction.
    #[error("serde error: {0}")]
    SerdeError(String),

    /// A required field was missing or malformed during Section 3 extraction.
    #[error("missing or malformed field: {0}")]
    MissingField(String),

    /// Section B off-chain adapter binding-gate failure:
    /// `RecursiveSNARK::verify` rejected the proof, so the adapter
    /// refuses to emit a PI bundle.
    #[error("RecursiveSNARK::verify rejected: {0}")]
    VerifyRejected(String),

    /// NCR5 (re-audit 2026-05-14): R1CS shape parameter from the
    /// supplied JSON exceeds the hard sanity cap. Without this
    /// gate a malicious `RecursiveSNARK` dump could specify
    /// `num_cons / num_vars / num_io` large enough to make the
    /// Section 3 gadget's `sparse_lc` synthesis loop quadratic
    /// in `num_cons × entries.len()` — multi-hour setup DoS even
    /// off-chain.
    #[error("R1CS shape parameter `{name}` = {value} exceeds cap {cap}")]
    ShapeTooLarge {
        /// Which shape field was over the cap (`num_cons`, `num_vars`, `num_io`).
        name: &'static str,
        /// The over-cap value parsed from the dump.
        value: usize,
        /// The hard cap applied by `extract_section3_witness`.
        cap: usize,
    },
}

/// Extract `l_u_secondary.X[0]` and `l_u_secondary.X[1]` as
/// `(ArkFr, ArkFr)` ready to feed into
/// [`crate::verifier_circuit::NovaVerifierCircuit::new`]'s
/// `committed_hash_primary` / `committed_hash_secondary` slots.
///
/// Both scalars go through
/// [`crate::scalar_adapter::secondary_to_ark_fr_lossy`] — see that
/// function for the lossy-cross-field semantics.
pub fn extract_committed_hashes_via_serde(
    rs: &RecursiveSNARK<E1, E2, TrivialIncrementCircuit>,
) -> Result<(ArkFr, ArkFr), ExtractError> {
    let v = serde_json::to_value(rs).map_err(|e| ExtractError::Serialize(e.to_string()))?;
    let x = v
        .get("l_u_secondary")
        .and_then(|inst| inst.get("X"))
        .and_then(|x| x.as_array())
        .ok_or(ExtractError::MissingPath)?;

    if x.len() < 2 {
        return Err(ExtractError::TooFewHashes(x.len()));
    }

    let s0 = parse_secondary_scalar_hex(x[0].as_str(), 0)?;
    let s1 = parse_secondary_scalar_hex(x[1].as_str(), 1)?;
    Ok((secondary_to_ark_fr_lossy(s0), secondary_to_ark_fr_lossy(s1)))
}

/// B-1/B-2 1C §7 Section B step C: extract ALL Section B public-input
/// values from a `RecursiveSNARK` via the same serde-JSON reflection
/// pattern as `extract_committed_hashes_via_serde`. Returns the
/// nine fixed scalars + z0/zi arrays in the layout the Section B
/// in-circuit Poseidon gate (next iteration) will consume.
///
/// JSON paths (verified empirically by the existing
/// `debug_dump_l_u_secondary_json_shape` test pattern):
///   - `pp_digest`             — caller-supplied (lives on the vk,
///                               not on the RecursiveSNARK; passed in
///                               separately so this fn doesn't need a
///                               vk reference).
///   - `num_steps`             — caller-supplied (passed to
///                               `prove_step` n times).
///   - `z0[..]`                — `rs.z0`.
///   - `zi[..]`                — `rs.zi`.
///   - `ri_secondary`          — `rs.ri_secondary`.
///   - `r_U_primary` fields    — `rs.r_U_primary.comm_W.x/y, X[0], X[1]`.
///   - `hash_primary_reinterp` — `l_u_secondary.X[0]` (via
///                               `base_as_scalar` ≡ `secondary_to_ark_fr_lossy`).
///   - `hash_secondary_claimed`— `l_u_secondary.X[1]`.
///
/// For full source mapping to nova-snark CompressedSNARK::verify
/// L935-963, see `SECTION_B_SCOPING.md` §1.
///
/// IMPORTANT: this extraction is for the CYCLEFOLD INTEGRATION (the
/// 1C arc). The RecursiveSNARK fixture is built against
/// `TrivialIncrementCircuit` (z0/zi arity = 1) — extending to other
/// step circuits is a straightforward type-parameter change (the
/// JSON paths are identical).
#[derive(Clone, Debug)]
pub struct SectionBPiBundle {
    /// `hash_primary_reinterp` (= base_as_scalar of l_u_secondary.X[0]).
    pub hash_primary_reinterp: ArkFr,
    /// `hash_secondary_claimed` (= l_u_secondary.X[1]).
    pub hash_secondary_claimed: ArkFr,
    /// `vk.pp_digest` (caller-supplied).
    pub pp_digest: ArkFr,
    /// IVC step count (caller-supplied).
    pub num_steps: ArkFr,
    /// `rs.ri_secondary`.
    pub ri_secondary: ArkFr,
    /// `rs.r_U_primary` absorbed-in-RO fields.
    pub r_U_primary_comm_x: ArkFr,
    pub r_U_primary_comm_y: ArkFr,
    pub r_U_primary_x0: ArkFr,
    pub r_U_primary_x1: ArkFr,
    /// `rs.z0`.
    pub z0: Vec<ArkFr>,
    /// `rs.zi` (the "zn" in CompressedSNARK::verify naming).
    pub zn: Vec<ArkFr>,
}

/// Helper: parse a primary-side hex scalar (l_u_secondary.X[..] uses
/// secondary parser, but ri_secondary / r_U_primary fields use the
/// primary parser via primary_to_ark_fr in scalar_adapter — for now
/// we route both through secondary_to_ark_fr_lossy since the bit-
/// pattern reinterpretation is what the in-circuit hash expects).
fn parse_primary_or_lossy_scalar(
    s: Option<&str>,
    index: usize,
) -> Result<ArkFr, ExtractError> {
    // Many fields on RecursiveSNARK serialize identically (32-byte
    // LE hex). The lossy reinterpret is intentional for fields the
    // hash gate will absorb as "opaque Bn254 Fr" — exactly what
    // base_as_scalar / scalar_as_base do in nova-snark verify.
    let sec = parse_secondary_scalar_hex(s, index)?;
    Ok(secondary_to_ark_fr_lossy(sec))
}

/// Decompress a nova `bn256::Affine` GroupEncoding hex blob (single
/// 32-byte string from JSON), returning the (x, y) coords as ArkFr
/// via byte-level Fq→Fr lossy reinterpretation (the in-circuit hash
/// absorbs these as opaque Bn254 Fr).
fn decompress_comm_w_as_fr(s: Option<&str>) -> Result<(ArkFr, ArkFr), ExtractError> {
    use ark_ff::PrimeField as ArkPrimeField;
    use halo2curves::group::GroupEncoding;
    use halo2curves::bn256::G1Affine as Bn256Affine;

    let s = s.ok_or_else(|| ExtractError::MissingField(
        "r_U_primary.comm_W.comm (compressed point hex)".into(),
    ))?;
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    let bytes = hex::decode(stripped).map_err(|e| ExtractError::HexParseFailed {
        index: 999,
        reason: format!("comm_W hex decode: {e}"),
    })?;
    if bytes.len() != 32 {
        return Err(ExtractError::HexParseFailed {
            index: 999,
            reason: format!("comm_W expected 32 bytes, got {}", bytes.len()),
        });
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    let repr: <Bn256Affine as GroupEncoding>::Repr = arr.into();
    let a = Option::<Bn256Affine>::from(Bn256Affine::from_bytes(&repr))
        .ok_or_else(|| ExtractError::HexParseFailed {
            index: 999,
            reason: "could not decompress bn256-G1 comm_W".into(),
        })?;

    // Reinterpret each Fq coord as ArkFr via 32-byte LE round-trip.
    // For the in-circuit hash this is the byte-level absorption
    // pattern nova-snark uses (base_as_scalar via byte identity).
    let fq_to_fr_lossy = |fq_repr: [u8; 32]| -> ArkFr {
        // Read as little-endian ArkFr; reduces mod r if needed
        // (ArkFr::from_le_bytes_mod_order is the documented lossy path).
        ArkFr::from_le_bytes_mod_order(&fq_repr)
    };
    let x_repr = halo2curves::ff::PrimeField::to_repr(&a.x);
    let y_repr = halo2curves::ff::PrimeField::to_repr(&a.y);
    let mut x_bytes = [0u8; 32];
    x_bytes.copy_from_slice(x_repr.as_ref());
    let mut y_bytes = [0u8; 32];
    y_bytes.copy_from_slice(y_repr.as_ref());
    Ok((fq_to_fr_lossy(x_bytes), fq_to_fr_lossy(y_bytes)))
}

/// B-1/B-2 1C §7 step 2 / dossier §6b: off-chain adapter for Section
/// B's PI bundle, with the RecursiveSNARK::verify binding gate.
///
/// This is the **soundness anchor** of the delegation architecture
/// (`B1_B2_AUDIT_DOSSIER.md` §6b). The on-chain Groth16 binds only
/// Section A's MSM. Section B/C/D PIs are decorative on-chain; their
/// binding lives off-chain in this adapter:
///
/// 1. Run `RecursiveSNARK::verify(pp, num_steps, z0)`. If verify
///    fails ⇒ refuse to emit PIs (no point making the verifier
///    trust a proof we ourselves don't accept).
/// 2. On verify success, the returned `zn` is the canonical final
///    state — use it in the PI bundle (NOT the `rs.zi` field, which
///    is also serialized and SHOULD match).
/// 3. Extract the rest of the PIs from the serialized rs JSON
///    via `extract_section_b_pi_bundle` and overlay the verified zn.
///
/// **Soundness:** any verifier consuming the returned bundle can
/// rely on the proof being well-formed (since this adapter verified
/// it). A malicious prover who submitted a forged rs would fail
/// step 1 and never get a PI bundle to publish. The bundle is the
/// fraud-proof-rollup-style commitment between on-chain Groth16
/// (Section A only) and off-chain `CompressedSNARK::verify`-class
/// trust.
///
/// **Failure modes:**
/// - `verify` rejects: returns `ExtractError::VerifyRejected`
/// - extraction JSON paths missing: same errors as
///   `extract_section_b_pi_bundle`
pub fn assemble_section_b_pi_bundle(
    pp: &PublicParams<E1, E2, TrivialIncrementCircuit>,
    rs: &RecursiveSNARK<E1, E2, TrivialIncrementCircuit>,
    num_steps: usize,
    z0_ark: &[ArkFr],
) -> Result<SectionBPiBundle, ExtractError> {
    // 1. Verify (the binding gate).
    let z0_nova: Vec<<E1 as nova_snark::traits::Engine>::Scalar> =
        z0_ark.iter().copied().map(ark_fr_to_primary).collect();
    let zn_nova = rs
        .verify(pp, num_steps, &z0_nova)
        .map_err(|e| ExtractError::VerifyRejected(format!("{e:?}")))?;

    // 2. pp_digest as ArkFr (primary scalar — exact, not lossy).
    let pp_digest_ark = primary_to_ark_fr(pp.digest());

    // 3. Extract the rest via the existing serde-JSON path.
    let mut bundle = extract_section_b_pi_bundle(rs, pp_digest_ark, num_steps as u64)?;

    // 4. Overlay the verified zn — this is the canonical final state
    //    according to the verifier; rs.zi (serialized) should match
    //    but the verifier-derived value is authoritative.
    let zn_ark: Vec<ArkFr> = zn_nova.iter().copied().map(primary_to_ark_fr).collect();
    bundle.zn = zn_ark;

    Ok(bundle)
}

pub fn extract_section_b_pi_bundle(
    rs: &RecursiveSNARK<E1, E2, TrivialIncrementCircuit>,
    pp_digest: ArkFr,
    num_steps: u64,
) -> Result<SectionBPiBundle, ExtractError> {
    let v = serde_json::to_value(rs)
        .map_err(|e| ExtractError::Serialize(e.to_string()))?;

    // 1. l_u_secondary.X[0..2] — the two output hashes.
    let x = v
        .get("l_u_secondary")
        .and_then(|inst| inst.get("X"))
        .and_then(|x| x.as_array())
        .ok_or(ExtractError::MissingPath)?;
    if x.len() < 2 {
        return Err(ExtractError::TooFewHashes(x.len()));
    }
    let hash_primary_reinterp = parse_primary_or_lossy_scalar(x[0].as_str(), 0)?;
    let hash_secondary_claimed = parse_primary_or_lossy_scalar(x[1].as_str(), 1)?;

    // 2. ri_secondary.
    let ri_secondary = parse_primary_or_lossy_scalar(
        v.get("ri_secondary").and_then(|x| x.as_str()),
        2,
    )?;

    // 3. r_U_primary.comm_W.comm (compressed point) + X[0..2].
    //    The comm is serialized as ONE 32-byte hex string (nova's
    //    bn256::Affine GroupEncoding), not separate x/y JSON fields.
    //    Decompress then byte-reinterpret each Fq coord as Fr
    //    (the in-circuit hash absorbs them as "opaque Bn254 Fr").
    let r_u_p = v
        .get("r_U_primary")
        .ok_or(ExtractError::MissingPath)?;
    let comm_w = r_u_p
        .get("comm_W")
        .ok_or_else(|| ExtractError::MissingField("r_U_primary.comm_W".into()))?;
    let (r_U_primary_comm_x, r_U_primary_comm_y) =
        decompress_comm_w_as_fr(
            comm_w.get("comm").and_then(|s| s.as_str()),
        )?;
    let r_u_p_x = r_u_p
        .get("X")
        .and_then(|x| x.as_array())
        .ok_or_else(|| ExtractError::MissingField("r_U_primary.X".into()))?;
    if r_u_p_x.len() < 2 {
        return Err(ExtractError::TooFewHashes(r_u_p_x.len()));
    }
    let r_U_primary_x0 = parse_primary_or_lossy_scalar(r_u_p_x[0].as_str(), 5)?;
    let r_U_primary_x1 = parse_primary_or_lossy_scalar(r_u_p_x[1].as_str(), 6)?;

    // 4. z0 and zi arrays.
    let parse_arr = |key: &str, base_idx: usize| -> Result<Vec<ArkFr>, ExtractError> {
        let arr = v
            .get(key)
            .and_then(|x| x.as_array())
            .ok_or_else(|| ExtractError::MissingField(format!("{key} (expected array)")))?;
        arr.iter()
            .enumerate()
            .map(|(i, e)| parse_primary_or_lossy_scalar(e.as_str(), base_idx + i))
            .collect()
    };
    let z0 = parse_arr("z0", 100)?;
    let zn = parse_arr("zi", 200)?;

    Ok(SectionBPiBundle {
        hash_primary_reinterp,
        hash_secondary_claimed,
        pp_digest,
        num_steps: ArkFr::from(num_steps),
        ri_secondary,
        r_U_primary_comm_x,
        r_U_primary_comm_y,
        r_U_primary_x0,
        r_U_primary_x1,
        z0,
        zn,
    })
}

/// Reused by `s4_secondary_extract` (audit B-1/B-2 S4a): the
/// endianness here is "verified empirically", so S4a reuses this
/// exact parser rather than re-deriving it.
pub(crate) fn parse_secondary_scalar_hex(
    s: Option<&str>,
    index: usize,
) -> Result<SecondaryScalar, ExtractError> {
    let s = s.ok_or_else(|| ExtractError::HexParseFailed {
        index,
        reason: "value at index was not a string".to_string(),
    })?;
    // EvmCompatSerde emits unprefixed lowercase hex of the 32-byte
    // scalar repr. Empirically (verified via debug dump test):
    // the bytes are stored in halo2curves's *native* LE order
    // — the hex string is the result of feeding `.to_repr()` (which
    // is LE for halo2curves) through a hex encoder. So no
    // byte-reverse is needed.
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    let bytes_le = hex::decode(stripped).map_err(|e| ExtractError::HexParseFailed {
        index,
        reason: format!("hex decode: {e}"),
    })?;
    if bytes_le.len() != 32 {
        return Err(ExtractError::HexParseFailed {
            index,
            reason: format!("expected 32 bytes, got {}", bytes_le.len()),
        });
    }
    let mut bytes_le_arr = [0u8; 32];
    bytes_le_arr.copy_from_slice(&bytes_le);
    let repr = <SecondaryScalar as PrimeField>::Repr::from(bytes_le_arr);
    SecondaryScalar::from_repr_vartime(repr).ok_or_else(|| {
        ExtractError::HexParseFailed {
            index,
            reason: "bytes do not canonicalise to a valid grumpkin scalar (try byte-reverse)"
                .to_string(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recursive_snark_fixture::generate_fixture;
    use ff::Field;

    /// Diagnostic — dump the raw JSON shape for a fresh fixture
    /// so the test output shows the exact hex format
    /// `EvmCompatSerde` emits. Runs first via test-name alphabetical
    /// ordering quirk; in any case can be invoked directly with
    /// `cargo test debug_dump_l_u_secondary_json_shape -- --nocapture`.
    #[test]
    fn debug_dump_l_u_secondary_json_shape() {
        let rs = generate_fixture(2).expect("fixture");
        let v = serde_json::to_value(&rs).expect("to_value");
        let l_u = v.get("l_u_secondary").expect("l_u_secondary missing");
        eprintln!(
            "l_u_secondary shape: {}",
            serde_json::to_string_pretty(&l_u).unwrap_or_default()
        );
    }

    /// (Section B off-chain adapter): `assemble_section_b_pi_bundle`
    /// runs the RecursiveSNARK::verify binding gate before emitting
    /// the PI bundle. Per the delegation trust-model decision
    /// (dossier §6b), this IS the soundness anchor for Section B.
    ///
    /// Positive test: real fixture verifies, adapter returns a
    /// bundle whose hashes match the raw extractor's hashes (parity)
    /// and whose pp_digest is non-zero (verified pp.digest() != 0).
    /// Helper: build (pp, rs) using the SAME pp instance (the
    /// fixture functions create a fresh pp each call, so their pp
    /// has a different digest than `canonical_public_params()`).
    fn fixture_with_shared_pp(
        num_steps: usize,
    ) -> (
        nova_snark::nova::PublicParams<E1, E2, TrivialIncrementCircuit>,
        RecursiveSNARK<E1, E2, TrivialIncrementCircuit>,
    ) {
        use crate::recursive_snark_fixture::{Scalar1, E1 as _E1, E2 as _E2};
        use ff::Field;
        use nova_snark::nova::PublicParams;
        use nova_snark::spartan::ppsnark::RelaxedR1CSSNARK;
        use nova_snark::traits::snark::RelaxedR1CSSNARKTrait;
        use nova_snark::provider::hyperkzg::EvaluationEngine;
        use nova_snark::provider::ipa_pc::EvaluationEngine as IpaEE;

        let circuit = TrivialIncrementCircuit;
        type S1 = RelaxedR1CSSNARK<_E1, EvaluationEngine<_E1>>;
        type S2 = RelaxedR1CSSNARK<_E2, IpaEE<_E2>>;
        let pp = PublicParams::<_E1, _E2, TrivialIncrementCircuit>::setup(
            &circuit, &*S1::ck_floor(), &*S2::ck_floor(),
        ).expect("pp setup");
        let z0: Vec<Scalar1> = vec![Scalar1::ZERO];
        let mut rs =
            RecursiveSNARK::<_E1, _E2, TrivialIncrementCircuit>::new(
                &pp, &circuit, &z0,
            ).expect("rs new");
        for _ in 0..num_steps {
            rs.prove_step(&pp, &circuit).expect("prove_step");
        }
        (pp, rs)
    }

    #[test]
    fn assemble_section_b_pi_bundle_real_fixture_verifies() {
        let (pp, rs) = fixture_with_shared_pp(2);

        // z0 for TrivialIncrementCircuit is [0]. Use ArkFr.
        let z0 = vec![ArkFr::from(0u64)];

        let bundle =
            assemble_section_b_pi_bundle(&pp, &rs, 2, &z0).expect("assemble");

        // pp_digest is exact (not lossy) and must be non-zero for a
        // real fixture.
        assert!(
            bundle.pp_digest != ArkFr::from(0u64),
            "pp.digest() must be non-zero for a real fixture"
        );

        // num_steps round-trips.
        assert_eq!(bundle.num_steps, ArkFr::from(2u64));

        // Parity vs the raw extractor on the two output hashes.
        let (h0, h1) =
            extract_committed_hashes_via_serde(&rs).expect("legacy extract");
        assert_eq!(bundle.hash_primary_reinterp, h0);
        assert_eq!(bundle.hash_secondary_claimed, h1);

        // zn comes from the verifier (canonical) — must match what
        // the raw extractor pulled from rs.zi.
        assert_eq!(bundle.z0.len(), 1, "z0 arity = 1");
        assert_eq!(bundle.zn.len(), 1, "zn arity = 1");
        // TrivialIncrementCircuit: z_{i+1} = z_i + 1, so zn after
        // 2 steps starting at z0=[0] should be [2].
        assert_eq!(bundle.zn[0], ArkFr::from(2u64),
            "TrivialIncrementCircuit z2 must be 2 from z0=0");

        eprintln!(
            "ADAPTER_BUNDLE_VERIFIED pp_digest={} num_steps={} zn[0]={}",
            bundle.pp_digest, bundle.num_steps, bundle.zn[0]
        );
    }

    /// NEGATIVE: tamper `num_steps` mismatch ⇒ verify rejects ⇒
    /// adapter refuses to emit a PI bundle.
    #[test]
    fn assemble_section_b_pi_bundle_rejects_wrong_num_steps() {
        let (pp, rs) = fixture_with_shared_pp(2);
        let z0 = vec![ArkFr::from(0u64)];

        // Caller LIES about num_steps (says 3, fixture ran 2).
        let r = assemble_section_b_pi_bundle(&pp, &rs, 3, &z0);
        assert!(
            matches!(r, Err(ExtractError::VerifyRejected(_))),
            "wrong num_steps must trigger VerifyRejected: got {r:?}"
        );
    }

    /// NEGATIVE: tamper z0 ⇒ verify rejects ⇒ adapter refuses.
    #[test]
    fn assemble_section_b_pi_bundle_rejects_wrong_z0() {
        let (pp, rs) = fixture_with_shared_pp(2);
        // Real z0 was [0]; lie and say [99].
        let z0_bad = vec![ArkFr::from(99u64)];

        let r = assemble_section_b_pi_bundle(&pp, &rs, 2, &z0_bad);
        assert!(
            matches!(r, Err(ExtractError::VerifyRejected(_))),
            "wrong z0 must trigger VerifyRejected: got {r:?}"
        );
    }

    /// (Section B step C): full Section B PI bundle extraction
    /// from a real RecursiveSNARK fixture. Validates:
    ///   - extraction succeeds (all JSON paths present)
    ///   - the 9 fixed scalars are not all trivially zero
    ///   - z0 / zn arrays have the expected arity (1 for
    ///     TrivialIncrementCircuit)
    ///   - hash_primary_reinterp / hash_secondary_claimed match the
    ///     existing `extract_committed_hashes_via_serde` output
    ///     (parity gate — the two paths must agree on the X[..]
    ///     fields they share)
    #[test]
    fn extract_section_b_pi_bundle_real_fixture() {
        let rs = generate_fixture(2).expect("fixture");
        let pp_digest_placeholder = ArkFr::from(1234u64);
        let bundle = extract_section_b_pi_bundle(
            &rs, pp_digest_placeholder, 2,
        ).expect("extract bundle");

        // Sanity: arities + caller-supplied values.
        assert_eq!(bundle.num_steps, ArkFr::from(2u64), "num_steps echoed back");
        assert_eq!(
            bundle.pp_digest, pp_digest_placeholder,
            "pp_digest is caller-supplied (vk field)"
        );
        assert_eq!(bundle.z0.len(), 1, "TrivialIncrementCircuit z0 arity = 1");
        assert_eq!(bundle.zn.len(), 1, "TrivialIncrementCircuit zi arity = 1");

        // Non-vacuity: at least one of the 9 fixed scalars is non-zero.
        let any_nonzero = [
            bundle.hash_primary_reinterp,
            bundle.hash_secondary_claimed,
            bundle.ri_secondary,
            bundle.r_U_primary_comm_x,
            bundle.r_U_primary_comm_y,
            bundle.r_U_primary_x0,
            bundle.r_U_primary_x1,
        ]
        .iter()
        .any(|f| *f != ArkFr::from(0u64));
        assert!(
            any_nonzero,
            "all 7 extracted scalars zero ⇒ JSON path silent failure"
        );

        // Parity with the original 2-hash extractor.
        let (h0, h1) =
            extract_committed_hashes_via_serde(&rs).expect("legacy extract");
        assert_eq!(
            bundle.hash_primary_reinterp, h0,
            "bundle.hash_primary_reinterp must equal legacy l_u_secondary.X[0]"
        );
        assert_eq!(
            bundle.hash_secondary_claimed, h1,
            "bundle.hash_secondary_claimed must equal legacy l_u_secondary.X[1]"
        );

        // For downstream Section B step D wiring: the bundle field
        // count matches SectionBPublicInputs::pi_count() = 9 + |z0|
        // + |zn| = 9 + 1 + 1 = 11.
        let expected_pi_count = 9 + bundle.z0.len() + bundle.zn.len();
        assert_eq!(expected_pi_count, 11, "TrivialIncrementCircuit gives 11 Section B PIs");

        eprintln!(
            "SECTION_B_BUNDLE_EXTRACTED: 9 fixed + |z0|={} + |zn|={} = {} PIs",
            bundle.z0.len(), bundle.zn.len(), expected_pi_count
        );
    }

    /// Extraction succeeds on a real fixture and returns scalars
    /// that aren't both trivially zero. (If both come back zero
    /// the JSON path is wrong but the parse didn't notice — this
    /// catches that silent failure mode.)
    #[test]
    fn extract_returns_non_trivial_hashes_for_real_fixture() {
        let rs = generate_fixture(2).expect("fixture");
        let (h0, h1) = extract_committed_hashes_via_serde(&rs).expect("extract");
        // At least one must be non-zero — nova's transcript hash
        // of (pp.digest, num_steps=2, z0=[0], zi=[2], ...) won't
        // produce all-zeros by accident.
        assert!(
            h0 != ArkFr::from(0u64) || h1 != ArkFr::from(0u64),
            "both extracted hashes must not be zero — likely a JSON-path mismatch \
             returning placeholder values silently"
        );
    }

    /// Extraction is deterministic on the *same* fixture across
    /// repeated calls. (Across fresh fixtures the hashes differ
    /// because Pedersen commitments in `r_U_secondary` use random
    /// blinding factors per call to `RecursiveSNARK::new` — those
    /// blindings flow into the Poseidon transcript and therefore
    /// into `l_u_secondary.X[..]`. So the determinism property
    /// we can pin is "same RS → same extraction", not "same num_steps →
    /// same extraction".)
    #[test]
    fn extraction_is_deterministic_for_a_single_fixture() {
        let rs = generate_fixture(3).expect("fixture");
        let (a0, a1) = extract_committed_hashes_via_serde(&rs).expect("extract a");
        let (b0, b1) = extract_committed_hashes_via_serde(&rs).expect("extract b");
        assert_eq!(a0, b0, "h0 must match across repeated extracts of same RS");
        assert_eq!(a1, b1, "h1 must match across repeated extracts of same RS");
    }

    /// Different `num_steps` should produce different committed
    /// hashes (the transcript binds num_steps).
    #[test]
    fn different_num_steps_produce_different_hashes() {
        let rs_2 = generate_fixture(2).expect("fixture 2");
        let rs_5 = generate_fixture(5).expect("fixture 5");
        let (h0_2, h1_2) = extract_committed_hashes_via_serde(&rs_2).expect("extract 2");
        let (h0_5, h1_5) = extract_committed_hashes_via_serde(&rs_5).expect("extract 5");
        // At least one hash must differ.
        assert!(
            h0_2 != h0_5 || h1_2 != h1_5,
            "different num_steps must produce different transcript hashes"
        );
    }

    #[test]
    fn parse_secondary_scalar_hex_none_input_errors() {
        let err = parse_secondary_scalar_hex(None, 0).expect_err("expected error");
        match err {
            ExtractError::HexParseFailed { index, .. } => assert_eq!(index, 0),
            other => panic!("expected HexParseFailed, got {other:?}"),
        }
    }

    #[test]
    fn parse_secondary_scalar_hex_non_hex_errors() {
        let err = parse_secondary_scalar_hex(Some("nothex"), 3).expect_err("expected error");
        match err {
            ExtractError::HexParseFailed { index, .. } => assert_eq!(index, 3),
            other => panic!("expected HexParseFailed, got {other:?}"),
        }
    }

    #[test]
    fn parse_secondary_scalar_hex_wrong_length_errors() {
        let err = parse_secondary_scalar_hex(Some("deadbeef"), 1).expect_err("expected error");
        match err {
            ExtractError::HexParseFailed { index, reason } => {
                assert_eq!(index, 1);
                assert!(reason.contains("32 bytes"), "reason was {reason}");
            }
            other => panic!("expected HexParseFailed, got {other:?}"),
        }
    }

    #[test]
    fn parse_secondary_scalar_hex_accepts_0x_prefix() {
        let zero_hex_prefixed = format!("0x{}", "00".repeat(32));
        let s = parse_secondary_scalar_hex(Some(&zero_hex_prefixed), 0)
            .expect("zero scalar parses");
        assert_eq!(s, SecondaryScalar::ZERO);
    }

    #[test]
    fn parse_secondary_scalar_hex_accepts_bare_hex() {
        let zero_hex = "00".repeat(32);
        let s = parse_secondary_scalar_hex(Some(&zero_hex), 0)
            .expect("zero scalar parses without prefix");
        assert_eq!(s, SecondaryScalar::ZERO);
    }

    #[test]
    fn extract_error_displays_all_variants() {
        assert!(ExtractError::Serialize("x".into()).to_string().contains("serde_json"));
        assert!(ExtractError::MissingPath.to_string().contains("l_u_secondary"));
        assert!(ExtractError::TooFewHashes(1).to_string().contains("1"));
        assert!(ExtractError::HexParseFailed { index: 0, reason: "r".into() }
            .to_string()
            .contains("hex scalar"));
        assert!(ExtractError::SerdeError("e".into()).to_string().contains("serde"));
        assert!(ExtractError::MissingField("f".into()).to_string().contains("malformed"));
        assert!(ExtractError::ShapeTooLarge { name: "num_cons", value: 999, cap: 1 }
            .to_string()
            .contains("num_cons"));
    }

    #[test]
    fn extract_error_clone_and_debug_work() {
        let err = ExtractError::ShapeTooLarge {
            name: "num_vars",
            value: 100,
            cap: 10,
        };
        let _cloned = err.clone();
        let _debug = format!("{err:?}");
    }
}
