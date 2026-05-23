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
pub(crate) fn decode_grumpkin_point(hex: &str) -> Result<GAffine, ExtractError> {
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

/// 1C (b)-2b: extract a freestanding `RelaxedR1CSInstance<
/// GrumpkinEngine>` (serde to_value) into the arkworks
/// representation the primary shell's Section R absorb expects.
/// Path schema verified by the (b)-2b premise dump (HEAD
/// `9344e97a`): top-level keys `{X, comm_E, comm_W, u}`;
/// comm_W/comm_E are `{comm: <64-hex>}`; u is `<64-hex>`; X is
/// array of `<64-hex>`. Returns commitments as Affine<GrumpkinConfig>
/// (Bn254Fr coords native) + u and X as `ArkFq`.
pub fn extract_relaxed_running_inst(
    inst_json: &Value,
) -> Result<(GAffine, GAffine, ArkFq, Vec<ArkFq>), ExtractError> {
    use crate::l_u_secondary_extract::parse_secondary_scalar_hex;
    use crate::scalar_adapter::secondary_to_ark_fq;
    let comm_w_s = inst_json
        .get("comm_W")
        .and_then(|c| c.get("comm"))
        .and_then(|c| c.as_str())
        .ok_or(ExtractError::MissingPath)?;
    let comm_e_s = inst_json
        .get("comm_E")
        .and_then(|c| c.get("comm"))
        .and_then(|c| c.as_str())
        .ok_or(ExtractError::MissingPath)?;
    let u_s = inst_json
        .get("u")
        .and_then(|u| u.as_str())
        .ok_or(ExtractError::MissingPath)?;
    let x_arr = inst_json
        .get("X")
        .and_then(|x| x.as_array())
        .ok_or(ExtractError::MissingPath)?;
    let comm_w = decode_grumpkin_point(comm_w_s)?;
    let comm_e = decode_grumpkin_point(comm_e_s)?;
    let u = secondary_to_ark_fq(parse_secondary_scalar_hex(Some(u_s), 0)?);
    let mut x = Vec::with_capacity(x_arr.len());
    for (i, e) in x_arr.iter().enumerate() {
        let s = parse_secondary_scalar_hex(e.as_str(), i)?;
        x.push(secondary_to_ark_fq(s));
    }
    Ok((comm_w, comm_e, u, x))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::s4_msm_gadget::pedersen_msm_grumpkin;
    use ark_bn254::Fr as ArkFr;
    use ark_ec::CurveGroup;
    use ark_r1cs_std::{alloc::AllocVar, fields::emulated_fp::EmulatedFpVar, GR1CSVar};
    use ark_relations::gr1cs::ConstraintSystem;

    /// Bounded-`W` real-data S4a proof (4 GB-box safe).
    ///
    /// CAPACITY (see `S4_DESIGN.md`): a full-`W` non-native MSM
    /// exhausts the 4 GB node-box. So this caps to an `N`-entry
    /// prefix of the REAL extracted `ck`/`W`/`r_W` and asserts the
    /// in-circuit Pedersen MSM equals the **out-of-circuit ark MSM
    /// over that same real prefix** (proves the extraction decoders
    /// + gadget are correct on real fixture data, at tractable
    /// memory), plus adversarial. The full-`W` `== r_U_secondary.
    /// comm_W` equality is a SEPARATE heavier run on a bigger
    /// machine — NOT this box.
    #[test]
    #[ignore = "S4a: real Nova fixture + PublicParams::setup; bounded-W (4GB-safe)"]
    fn secondary_msm_binds_real_prefix() {
        use crate::recursive_snark_fixture::{
            canonical_public_params, generate_fixture_with_digest,
        };
        use ark_ec::short_weierstrass::Projective;

        /// Prefix length — small enough for non-native MSM on 4 GB.
        const N: usize = 12;

        let pp = canonical_public_params().expect("canonical pp");
        let (rs, _digest) = generate_fixture_with_digest(2).expect("fixture");
        let pp_json = serde_json::to_value(&pp).expect("pp to_value");
        let rs_json = serde_json::to_value(&rs).expect("rs to_value");

        let (ck, h) = extract_secondary_ck(&pp_json).expect("extract ck");
        let (w, r_w) = extract_secondary_witness(&rs_json).expect("extract witness");
        // comm_W decode sanity (full binding is the bigger-box run).
        let _comm_w = extract_secondary_comm_w(&rs_json).expect("extract comm_W");
        assert!(!w.is_empty(), "secondary W must be non-empty");
        assert!(ck.len() >= w.len(), "ck must cover W (nova invariant)");

        let n = w.len().min(N);
        // Out-of-circuit ark MSM over the SAME real prefix.
        let mut expected = Projective::<GrumpkinConfig>::from(h) * r_w;
        for i in 0..n {
            expected += Projective::<GrumpkinConfig>::from(ck[i]) * w[i];
        }

        let cs = ConstraintSystem::<ArkFr>::new_ref();
        let scalars: Vec<EmulatedFpVar<ArkFq, ArkFr>> = w[..n]
            .iter()
            .map(|v| EmulatedFpVar::new_witness(cs.clone(), || Ok(*v)).unwrap())
            .collect();
        let blind = EmulatedFpVar::<ArkFq, ArkFr>::new_witness(cs.clone(), || Ok(r_w)).unwrap();
        let out = pedersen_msm_grumpkin(&scalars, &ck[..n], &blind, h).expect("msm gadget");

        assert!(cs.is_satisfied().expect("is_satisfied"));
        assert_eq!(
            out.value().expect("circuit point").into_affine(),
            expected.into_affine(),
            "in-circuit MSM over real ck/W prefix must equal out-of-circuit ark MSM"
        );

        // Adversarial: perturb W[0] → in-circuit MSM must differ.
        let cs2 = ConstraintSystem::<ArkFr>::new_ref();
        let mut wb = w[..n].to_vec();
        wb[0] += ArkFq::from(1u64);
        let sc2: Vec<EmulatedFpVar<ArkFq, ArkFr>> = wb
            .iter()
            .map(|v| EmulatedFpVar::new_witness(cs2.clone(), || Ok(*v)).unwrap())
            .collect();
        let bl2 = EmulatedFpVar::<ArkFq, ArkFr>::new_witness(cs2.clone(), || Ok(r_w)).unwrap();
        let out_bad = pedersen_msm_grumpkin(&sc2, &ck[..n], &bl2, h).expect("msm gadget adv");
        assert_ne!(
            out_bad.value().expect("adv point").into_affine(),
            expected.into_affine(),
            "perturbed W must NOT reproduce the MSM (binding logic is sound)"
        );
    }

    /// **A.3 — FULL-`W` secondary soundness closure** (scale-gate,
    /// satyawan-1 / ≫16 GB). Unlike A.2 (bounded, in-circuit ==
    /// ark-over-prefix), this asserts the FULL real `W` MSM equals
    /// the actual Section-2-bound `r_U_secondary.comm_W` —
    /// `comm_W == Σ Wᵢ·ckᵢ + r_W·h` — the real B-1 closure for the
    /// secondary instance. Plus adversarial. JSON freed pre-circuit
    /// (B.3 memory pattern); ck truncated to |W|.
    #[test]
    #[ignore = "A.3 SCALE-GATE: full-W secondary binding; run on satyawan-1 (≫16 GB)"]
    fn secondary_msm_binds_full_comm_w() {
        use crate::recursive_snark_fixture::{
            canonical_public_params, generate_fixture_with_digest,
        };
        let (mut ck, w, r_w, comm_w) = {
            let pp = canonical_public_params().expect("canonical pp");
            let (rs, _d) = generate_fixture_with_digest(2).expect("fixture");
            let pp_json = serde_json::to_value(&pp).expect("pp json");
            let rs_json = serde_json::to_value(&rs).expect("rs json");
            let (ck, h) = extract_secondary_ck(&pp_json).expect("ck");
            let (w, r_w) = extract_secondary_witness(&rs_json).expect("W");
            let cw = extract_secondary_comm_w(&rs_json).expect("comm_W");
            (ck, w, r_w, (cw, h))
            // pp/rs/json dropped here.
        };
        let (comm_w, h) = comm_w;
        assert!(!w.is_empty() && ck.len() >= w.len(), "nova invariant");
        let n = w.len();
        ck.truncate(n);

        let cs = ConstraintSystem::<ArkFr>::new_ref();
        let scalars: Vec<EmulatedFpVar<ArkFq, ArkFr>> = w
            .iter()
            .map(|v| EmulatedFpVar::new_witness(cs.clone(), || Ok(*v)).unwrap())
            .collect();
        let blind = EmulatedFpVar::<ArkFq, ArkFr>::new_witness(cs.clone(), || Ok(r_w)).unwrap();
        let out = pedersen_msm_grumpkin(&scalars, &ck, &blind, h).expect("full msm");

        assert!(cs.is_satisfied().expect("is_satisfied"), "CS satisfied");
        assert_eq!(
            out.value().expect("circuit point").into_affine(),
            comm_w,
            "FULL real-W MSM must equal the Section-2-bound r_U_secondary.comm_W"
        );

        // Adversarial: perturb W[0] → must NOT reproduce comm_W.
        let cs2 = ConstraintSystem::<ArkFr>::new_ref();
        let mut wb = w.clone();
        wb[0] += ArkFq::from(1u64);
        let sc2: Vec<EmulatedFpVar<ArkFq, ArkFr>> = wb
            .iter()
            .map(|v| EmulatedFpVar::new_witness(cs2.clone(), || Ok(*v)).unwrap())
            .collect();
        let bl2 = EmulatedFpVar::<ArkFq, ArkFr>::new_witness(cs2.clone(), || Ok(r_w)).unwrap();
        let out_bad = pedersen_msm_grumpkin(&sc2, &ck, &bl2, h).expect("adv msm");
        assert_ne!(
            out_bad.value().expect("adv point").into_affine(),
            comm_w,
            "perturbed full W must NOT reproduce comm_W (binding is sound)"
        );
    }
}
