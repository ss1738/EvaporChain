//! Audit B-1/B-2 S4a: extract a real fixture's secondary Pedersen
//! commitment data and bind it to the proven MSM gadget.
//!
//! Enforces `r_U_secondary.comm_W == Σ Wᵢ·ckᵢ + r_W·h` for a REAL
//! `RecursiveSNARK` — the constraint that closes the "W behind
//! Section 3 ≠ W behind comm_W" hole for the secondary instance.
//!
//! Every decoder is an exact reuse of proven codebase paths (see
//! `S4_DESIGN.md` "complete executable spec"):
//! - points (`ck[i]`, `h`, `comm_W`): the `section2_witness`
//!   `nova-grumpkin` `GroupEncoding` path — no `halo2curves`
//!   normal-dependency;
//! - scalars (`W`, `r_W`): the endianness-"verified empirically"
//!   `l_u_secondary_extract::parse_secondary_scalar_hex` →
//!   `scalar_adapter::secondary_to_ark_fq` (exact, same-field).
//!
//! JSON paths pinned from nova 0.68 source + a real
//! `serde_json::to_value(pp)` box dump (`ck_secondary` = bare 64-hex
//! compressed affines, 16384 bases).

use crate::grumpkin_config::GrumpkinConfig;
use crate::l_u_secondary_extract::{parse_secondary_scalar_hex, ExtractError};
use crate::scalar_adapter::{primary_to_ark_fr, secondary_to_ark_fq};
use ark_bn254::Fq as ArkFq;
use ark_ec::short_weierstrass::Affine;
use group::GroupEncoding;
use nova_snark::provider::bn256_grumpkin::grumpkin::Affine as GrumpkinAffine;
use serde_json::Value;

type GAffine = Affine<GrumpkinConfig>;

/// Decode one Grumpkin affine from a (bare or `0x`) 32-byte
/// compressed hex string — the `section2_witness` decompression
/// path, mapped onto `GrumpkinConfig`.
fn decode_grumpkin_point(hex: &str) -> Result<GAffine, ExtractError> {
    let stripped = hex.strip_prefix("0x").unwrap_or(hex);
    let bytes = hex::decode(stripped).map_err(|e| ExtractError::HexParseFailed {
        index: 0,
        reason: format!("grumpkin point hex decode: {e}"),
    })?;
    if bytes.len() != 32 {
        return Err(ExtractError::HexParseFailed {
            index: 0,
            reason: format!("expected 32 point bytes, got {}", bytes.len()),
        });
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    let repr: <GrumpkinAffine as GroupEncoding>::Repr = arr.into();
    let a = Option::<GrumpkinAffine>::from(GrumpkinAffine::from_bytes(&repr)).ok_or(
        ExtractError::HexParseFailed {
            index: 0,
            reason: "could not decompress grumpkin point".to_string(),
        },
    )?;
    let pt = GAffine::new_unchecked(primary_to_ark_fr(a.x), primary_to_ark_fr(a.y));
    if !pt.is_on_curve() {
        return Err(ExtractError::HexParseFailed {
            index: 0,
            reason: "decoded point is not on the GrumpkinConfig curve".to_string(),
        });
    }
    Ok(pt)
}

/// `pp_json["ck_secondary"]` → (`ck` bases, blinding `h`).
pub fn extract_secondary_ck(pp_json: &Value) -> Result<(Vec<GAffine>, GAffine), ExtractError> {
    let ck_sec = pp_json.get("ck_secondary").ok_or(ExtractError::MissingPath)?;
    let ck_arr = ck_sec
        .get("ck")
        .and_then(|c| c.as_array())
        .ok_or(ExtractError::MissingPath)?;
    let mut ck = Vec::with_capacity(ck_arr.len());
    for e in ck_arr {
        let s = e.as_str().ok_or(ExtractError::MissingPath)?;
        ck.push(decode_grumpkin_point(s)?);
    }
    let h_s = ck_sec
        .get("h")
        .and_then(|h| h.as_str())
        .ok_or(ExtractError::MissingPath)?;
    let h = decode_grumpkin_point(h_s)?;
    Ok((ck, h))
}

/// `rs_json["r_W_secondary"]` → (`W` vector, blinding `r_W`) as exact
/// `ark_bn254::Fq` (Grumpkin scalar field).
pub fn extract_secondary_witness(rs_json: &Value) -> Result<(Vec<ArkFq>, ArkFq), ExtractError> {
    let rw = rs_json
        .get("r_W_secondary")
        .ok_or(ExtractError::MissingPath)?;
    let w_arr = rw
        .get("W")
        .and_then(|w| w.as_array())
        .ok_or(ExtractError::MissingPath)?;
    let mut w = Vec::with_capacity(w_arr.len());
    for (i, e) in w_arr.iter().enumerate() {
        let s = parse_secondary_scalar_hex(e.as_str(), i)?;
        w.push(secondary_to_ark_fq(s));
    }
    let r_w_s = rw.get("r_W").and_then(|r| r.as_str());
    let r_w = secondary_to_ark_fq(parse_secondary_scalar_hex(r_w_s, 0)?);
    Ok((w, r_w))
}

/// `rs_json["r_U_secondary"]["comm_W"]["comm"]` → the Section-2-bound
/// committed point.
pub fn extract_secondary_comm_w(rs_json: &Value) -> Result<GAffine, ExtractError> {
    let s = rs_json
        .get("r_U_secondary")
        .and_then(|u| u.get("comm_W"))
        .and_then(|c| c.get("comm"))
        .and_then(|c| c.as_str())
        .ok_or(ExtractError::MissingPath)?;
    decode_grumpkin_point(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::s4_msm_gadget::pedersen_msm_grumpkin;
    use ark_bn254::Fr as ArkFr;
    use ark_ec::CurveGroup;
    use ark_r1cs_std::{alloc::AllocVar, fields::emulated_fp::EmulatedFpVar, R1CSVar};
    use ark_relations::r1cs::ConstraintSystem;

    /// THE S4a BINDING PROOF (real fixture, S6-class): a real
    /// secondary `comm_W` equals the in-circuit Pedersen MSM of its
    /// real `W`/`r_W` against the real `ck`/`h`; and a perturbed `W`
    /// does NOT — the soundness binding the audit requires.
    #[test]
    #[ignore = "S4a: needs a real Nova fixture + PublicParams::setup (expensive)"]
    fn secondary_comm_w_binds_to_msm_of_real_witness() {
        use crate::recursive_snark_fixture::{
            canonical_public_params, generate_fixture_with_digest,
        };

        let pp = canonical_public_params().expect("canonical pp");
        let (rs, _digest) = generate_fixture_with_digest(2).expect("fixture");
        let pp_json = serde_json::to_value(&pp).expect("pp to_value");
        let rs_json = serde_json::to_value(&rs).expect("rs to_value");

        let (ck, h) = extract_secondary_ck(&pp_json).expect("extract ck");
        let (w, r_w) = extract_secondary_witness(&rs_json).expect("extract witness");
        let comm_w = extract_secondary_comm_w(&rs_json).expect("extract comm_W");
        assert!(!w.is_empty(), "secondary W must be non-empty");
        assert!(ck.len() >= w.len(), "ck must cover W (nova invariant)");

        let cs = ConstraintSystem::<ArkFr>::new_ref();
        let scalars: Vec<EmulatedFpVar<ArkFq, ArkFr>> = w
            .iter()
            .map(|v| EmulatedFpVar::new_witness(cs.clone(), || Ok(*v)).unwrap())
            .collect();
        let blind = EmulatedFpVar::<ArkFq, ArkFr>::new_witness(cs.clone(), || Ok(r_w)).unwrap();

        let out = pedersen_msm_grumpkin(&scalars, &ck[..w.len()], &blind, h)
            .expect("msm gadget");
        assert!(cs.is_satisfied().expect("is_satisfied"));
        assert_eq!(
            out.value().expect("circuit point").into_affine(),
            comm_w,
            "real secondary comm_W must equal in-circuit Pedersen MSM of real W/r_W"
        );

        // Adversarial: perturb W[0] → MSM must NOT equal comm_W.
        let cs2 = ConstraintSystem::<ArkFr>::new_ref();
        let mut w_bad = w.clone();
        w_bad[0] += ArkFq::from(1u64);
        let sc2: Vec<EmulatedFpVar<ArkFq, ArkFr>> = w_bad
            .iter()
            .map(|v| EmulatedFpVar::new_witness(cs2.clone(), || Ok(*v)).unwrap())
            .collect();
        let bl2 = EmulatedFpVar::<ArkFq, ArkFr>::new_witness(cs2.clone(), || Ok(r_w)).unwrap();
        let out_bad = pedersen_msm_grumpkin(&sc2, &ck[..w.len()], &bl2, h)
            .expect("msm gadget adv");
        assert_ne!(
            out_bad.value().expect("adv point").into_affine(),
            comm_w,
            "perturbed W must NOT reproduce comm_W (binding is sound)"
        );
    }
}
