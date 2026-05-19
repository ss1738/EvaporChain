//! Audit B-1/B-2 S4a PHASE B.3: real-fixture PRIMARY (bn256-G1)
//! commitment binding — `r_U_primary.comm_W == Σ Wᵢ·ckᵢ + r_W·h`.
//!
//! Mirror of the verified `s4_secondary_extract`, primary side:
//! - points (`ck`, `h`, `comm_W`): nova `bn256::Affine` +
//!   `GroupEncoding` compressed decode, coords → `ark_bn254::Fq` via
//!   the proven `secondary_to_ark_fq` (nova `bn256::Base` ≡ halo2
//!   `bn256::Fq` ≡ `grumpkin::Scalar` — same type, re-exported);
//! - scalars (`W`, `r_W`): primary scalar = `bn256::Scalar` = BN254
//!   **Fr** (circuit-native) → proven exact `primary_to_ark_fr`;
//! - in-circuit MSM uses the box-verified COMPLETE gadget
//!   (`g1_scalar_mul_complete` / `g1_add_complete`) — edge-safe.
//!
//! JSON paths pinned (nova 0.68): `pp["ck_primary"]["ck"|"h"]`,
//! `rs["r_W_primary"]["W"|"r_W"]`, `rs["r_U_primary"]["comm_W"]["comm"]`.
//! Bounded-prefix on Mini 1 (complete formulas are very heavy:
//! ~983 s / 4 scalars — full-`W` is the satyawan-1/cluster scale-gate).

use crate::l_u_secondary_extract::ExtractError;
use crate::scalar_adapter::{primary_to_ark_fr, secondary_to_ark_fq, PrimaryScalar};
use ark_bn254::{Fq as ArkFq, Fr as ArkFr};
use ff::PrimeField as FfPrimeField;
use group::GroupEncoding;
use nova_snark::provider::bn256_grumpkin::bn256::Affine as Bn256Affine;
use serde_json::Value;

/// Primary scalar hex (EvmCompatSerde, unprefixed lowercase LE) →
/// nova `bn256::Scalar`. Same endianness-verified pattern as
/// `l_u_secondary_extract::parse_secondary_scalar_hex`, primary type.
fn parse_primary_scalar_hex(s: Option<&str>, idx: usize) -> Result<PrimaryScalar, ExtractError> {
    let s = s.ok_or_else(|| ExtractError::HexParseFailed {
        index: idx,
        reason: "primary scalar not a string".to_string(),
    })?;
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    let bytes = hex::decode(stripped).map_err(|e| ExtractError::HexParseFailed {
        index: idx,
        reason: format!("hex decode: {e}"),
    })?;
    if bytes.len() != 32 {
        return Err(ExtractError::HexParseFailed {
            index: idx,
            reason: format!("expected 32 bytes, got {}", bytes.len()),
        });
    }
    let mut le = [0u8; 32];
    le.copy_from_slice(&bytes);
    let repr = <PrimaryScalar as FfPrimeField>::Repr::from(le);
    PrimaryScalar::from_repr_vartime(repr).ok_or_else(|| ExtractError::HexParseFailed {
        index: idx,
        reason: "bytes not a valid bn256 scalar".to_string(),
    })
}

/// Decode a bn256-G1 affine from (bare or `0x`) compressed hex →
/// `(x, y)` as `ark_bn254::Fq`. nova `bn256::Affine` GroupEncoding.
fn decode_bn256_point(hex: &str) -> Result<(ArkFq, ArkFq), ExtractError> {
    let stripped = hex.strip_prefix("0x").unwrap_or(hex);
    let bytes = hex::decode(stripped).map_err(|e| ExtractError::HexParseFailed {
        index: 0,
        reason: format!("bn256-G1 hex decode: {e}"),
    })?;
    if bytes.len() != 32 {
        return Err(ExtractError::HexParseFailed {
            index: 0,
            reason: format!("expected 32 point bytes, got {}", bytes.len()),
        });
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    let repr: <Bn256Affine as GroupEncoding>::Repr = arr.into();
    let a = Option::<Bn256Affine>::from(Bn256Affine::from_bytes(&repr)).ok_or(
        ExtractError::HexParseFailed {
            index: 0,
            reason: "could not decompress bn256-G1 point".to_string(),
        },
    )?;
    // nova bn256::Base ≡ grumpkin::Scalar (halo2 re-export) → reuse
    // the proven exact `secondary_to_ark_fq`.
    Ok((secondary_to_ark_fq(a.x), secondary_to_ark_fq(a.y)))
}

/// `pp_json["ck_primary"]` → (`ck` bases, blinding `h`) as Fq coords.
pub fn extract_primary_ck(
    pp_json: &Value,
) -> Result<(Vec<(ArkFq, ArkFq)>, (ArkFq, ArkFq)), ExtractError> {
    let ck_p = pp_json.get("ck_primary").ok_or(ExtractError::MissingPath)?;
    let arr = ck_p
        .get("ck")
        .and_then(|c| c.as_array())
        .ok_or(ExtractError::MissingPath)?;
    let mut ck = Vec::with_capacity(arr.len());
    for e in arr {
        ck.push(decode_bn256_point(e.as_str().ok_or(ExtractError::MissingPath)?)?);
    }
    let h = decode_bn256_point(
        ck_p.get("h").and_then(|h| h.as_str()).ok_or(ExtractError::MissingPath)?,
    )?;
    Ok((ck, h))
}

/// `rs_json["r_W_primary"]` → (`W`, `r_W`) as exact `ark_bn254::Fr`
/// (primary scalar field = circuit-native).
pub fn extract_primary_witness(rs_json: &Value) -> Result<(Vec<ArkFr>, ArkFr), ExtractError> {
    let rw = rs_json.get("r_W_primary").ok_or(ExtractError::MissingPath)?;
    let warr = rw
        .get("W")
        .and_then(|w| w.as_array())
        .ok_or(ExtractError::MissingPath)?;
    let mut w = Vec::with_capacity(warr.len());
    for (i, e) in warr.iter().enumerate() {
        w.push(primary_to_ark_fr(parse_primary_scalar_hex(e.as_str(), i)?));
    }
    let r_w = primary_to_ark_fr(parse_primary_scalar_hex(
        rw.get("r_W").and_then(|r| r.as_str()),
        0,
    )?);
    Ok((w, r_w))
}

/// `rs_json["r_U_primary"]["comm_W"]["comm"]` → the primary committed
/// point (Fq coords).
pub fn extract_primary_comm_w(rs_json: &Value) -> Result<(ArkFq, ArkFq), ExtractError> {
    let s = rs_json
        .get("r_U_primary")
        .and_then(|u| u.get("comm_W"))
        .and_then(|c| c.get("comm"))
        .and_then(|c| c.as_str())
        .ok_or(ExtractError::MissingPath)?;
    decode_bn256_point(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::s4_primary_msm_gadget::{g1_add_complete, g1_scalar_mul_complete, G1AffineVar};
    use ark_ec::{short_weierstrass::Projective, AffineRepr, CurveGroup};
    use ark_r1cs_std::{
        alloc::AllocVar, convert::ToBitsGadget, fields::emulated_fp::EmulatedFpVar,
        fields::fp::FpVar, R1CSVar,
    };
    use ark_relations::r1cs::ConstraintSystem;

    type FqV = EmulatedFpVar<ArkFq, ArkFr>;

    /// B.3 bounded real-data proof: extract REAL primary
    /// `ck`/`W`/`r_W` from a fixture; the in-circuit COMPLETE-formula
    /// MSM over an N-prefix equals the out-of-circuit ark bn256-G1
    /// MSM over the SAME real prefix (proves the primary extraction
    /// decoders + edge-safe gadget compose on real data). Tiny N —
    /// complete formulas are ~983 s / 4 scalars. Full-`W` ==
    /// `r_U_primary.comm_W` = the satyawan-1/cluster scale-gate.
    #[test]
    #[ignore = "B.3: real Nova fixture + complete-formula MSM (very heavy; Mini1)"]
    fn primary_msm_binds_real_prefix() {
        use crate::recursive_snark_fixture::{
            canonical_public_params, generate_fixture_with_digest,
        };
        const N: usize = 2;

        // Extract into owned Vecs, then FREE the giant pp/rs JSON
        // Values (ck_primary = 16384 points materialized as a parsed
        // tree) BEFORE the multi-GB complete-formula circuit — they
        // must not co-reside (B.3 OOM'd at 16 GB otherwise).
        let (mut ck, w) = {
            let pp = canonical_public_params().expect("canonical pp");
            let (rs, _d) = generate_fixture_with_digest(2).expect("fixture");
            let pp_json = serde_json::to_value(&pp).expect("pp json");
            let rs_json = serde_json::to_value(&rs).expect("rs json");
            let (ck, _h) = extract_primary_ck(&pp_json).expect("ck_primary");
            let (w, _r_w) = extract_primary_witness(&rs_json).expect("W_primary");
            let _cw = extract_primary_comm_w(&rs_json).expect("comm_W decode sanity");
            (ck, w)
            // pp, rs, pp_json, rs_json all dropped here.
        };
        assert!(!w.is_empty() && ck.len() >= w.len(), "nova invariant");
        let n = w.len().min(N);
        ck.truncate(n); // free the other ~16382 unused bases
        // Out-of-circuit ark MSM over the SAME real prefix
        // (accumulate from the first term — no Zero trait needed).
        let term = |i: usize| {
            let p = ark_bn254::G1Affine::new_unchecked(ck[i].0, ck[i].1);
            assert!(p.is_on_curve(), "ck[{i}] must be on bn256-G1");
            Projective::from(p) * w[i]
        };
        let mut expected = term(0);
        for i in 1..n {
            expected += term(i);
        }
        let exp = expected.into_affine();

        let cs = ConstraintSystem::<ArkFr>::new_ref();
        let mkfq = |v: ArkFq| FqV::new_witness(cs.clone(), || Ok(v)).unwrap();
        // acc = Σ scalarᵢ·baseᵢ via the edge-safe complete gadget.
        let mut acc: Option<crate::s4_primary_msm_gadget::G1ProjVar> = None;
        for i in 0..n {
            let base = G1AffineVar { x: mkfq(ck[i].0), y: mkfq(ck[i].1) };
            let kv = FpVar::<ArkFr>::new_witness(cs.clone(), || Ok(w[i])).unwrap();
            let mut bits = kv.to_bits_le().unwrap();
            bits.reverse(); // MSB-first, full 254-bit
            let term = g1_scalar_mul_complete(&base, &bits).expect("smul");
            acc = Some(match acc {
                None => term,
                Some(a) => g1_add_complete(&a, &term).expect("acc add"),
            });
        }
        let out = acc.expect("n>=1");
        assert!(cs.is_satisfied().expect("is_satisfied"), "CS satisfied");

        // Projective (X,Y,Z) ≡ affine (ax,ay) iff X==ax·Z ∧ Y==ay·Z ∧ Z≠0.
        let (xv, yv, zv) = (
            out.x.value().unwrap(),
            out.y.value().unwrap(),
            out.z.value().unwrap(),
        );
        assert_ne!(zv, ArkFq::from(0u64), "MSM result must be non-identity");
        assert_eq!(xv, exp.x().unwrap() * zv, "primary MSM X==ax·Z (real data)");
        assert_eq!(yv, exp.y().unwrap() * zv, "primary MSM Y==ay·Z (real data)");
    }
}
