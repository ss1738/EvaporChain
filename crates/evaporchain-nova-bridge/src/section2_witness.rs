//! Witness extraction for the Section 2 Neptune transcript hash check.
//!
//! Extracts all 18 absorb-sequence elements for the primary-side
//! hash from a live `RecursiveSNARK` and bundles them with the
//! `NeptuneParams` needed by `section2_gadget::enforce_neptune_sponge_primary`.
//!
//! # Absorb sequence (z_arity = 1, 18 elements)
//!
//! Matches nova-snark `nova/mod.rs:598-609`:
//!
//! ```text
//! [pp.digest, num_steps, z0[0], zi[0],
//!  r_U_secondary.comm_W.x, r_U_secondary.comm_W.y,
//!  r_U_secondary.comm_E.x, r_U_secondary.comm_E.y,
//!  scalar_as_base(r_U_secondary.u),
//!  nat_to_limbs(r_U_secondary.X[0], 64, 4)[0..4],
//!  nat_to_limbs(r_U_secondary.X[1], 64, 4)[0..4],
//!  ri_primary]
//! ```
//!
//! # JSON path (nova-snark 0.68 layout)
//!
//! All fields come from `serde_json::to_value(rs)`:
//!
//! | Field | JSON path |
//! |---|---|
//! | `comm_W_{x,y}` | `r_U_secondary.comm_W.comm` (32-byte hex → decompress) |
//! | `comm_E_{x,y}` | `r_U_secondary.comm_E.comm` (32-byte hex → decompress) |
//! | `u_as_base` | `r_U_secondary.u` (secondary-scalar hex → reinterpret as primary) |
//! | `x0_limbs` | `r_U_secondary.X[0]` (secondary-scalar → 4 x u64 limbs) |
//! | `x1_limbs` | `r_U_secondary.X[1]` (secondary-scalar → 4 x u64 limbs) |
//! | `ri_primary` | `ri_primary` (primary-scalar hex) |
//!
//! `pp_digest` is not in the `RecursiveSNARK` JSON — pass it from
//! `PublicParams::digest()` via `generate_fixture_with_digest`.

use crate::l_u_secondary_extract::ExtractError;
use crate::neptune_permutation_gadget::{
    params_from_dump_path, params_from_embedded, NeptuneParams,
};
use crate::recursive_snark_fixture::{Scalar1, TrivialIncrementCircuit, E1, E2};
use crate::scalar_adapter::{primary_to_ark_fr, secondary_to_ark_fr_lossy, SecondaryScalar};
use ark_bn254::Fr as ArkFr;
use ff::PrimeField;
use group::GroupEncoding;
use nova_snark::{
    nova::RecursiveSNARK, provider::bn256_grumpkin::grumpkin::Affine as GrumpkinAffine,
};
use serde_json::Value;

/// All witness data required by the Section 2 in-circuit Neptune hash check.
///
/// Constructed via [`extract_section2_witness`] from a live
/// `RecursiveSNARK`, or built manually in tests.
#[derive(Clone, Debug)]
#[allow(non_snake_case)]
pub struct Section2Witness {
    /// Neptune permutation parameters (width-25 BN254 standard).
    /// Loaded from the `dump-neptune-constants` JSON dump.
    pub params: NeptuneParams<ArkFr>,
    /// `pp.digest()` pre-converted to `ArkFr` via `primary_to_ark_fr`.
    pub pp_digest: ArkFr,
    /// x-coordinate of `r_U_secondary.comm_W` (grumpkin base = primary scalar).
    pub comm_W_x: ArkFr,
    /// y-coordinate of `r_U_secondary.comm_W`.
    pub comm_W_y: ArkFr,
    /// x-coordinate of `r_U_secondary.comm_E`.
    pub comm_E_x: ArkFr,
    /// y-coordinate of `r_U_secondary.comm_E`.
    pub comm_E_y: ArkFr,
    /// `scalar_as_base(r_U_secondary.u)` — secondary scalar reinterpreted
    /// as a primary-field element (LE bytes, lossy cross-field cast).
    pub u_as_base: ArkFr,
    /// `nat_to_limbs(r_U_secondary.X[0], 64, 4)` — four 64-bit limbs
    /// of the first public instance scalar, each as `ArkFr`.
    pub x0_limbs: [ArkFr; 4],
    /// `nat_to_limbs(r_U_secondary.X[1], 64, 4)` — four 64-bit limbs
    /// of the second public instance scalar, each as `ArkFr`.
    pub x1_limbs: [ArkFr; 4],
    /// `ri_primary` — running primary hash from the prior IVC step,
    /// pre-converted to `ArkFr` via `primary_to_ark_fr`.
    pub ri_primary: ArkFr,
}

impl Section2Witness {
    /// Audit B-1/B-2 S2a: canonical shape — embedded Neptune params +
    /// `pp_digest`; value fields zeroed. The Neptune sponge gadget
    /// absorbs a fixed 18-element sequence and emits fixed permutation
    /// rounds independent of values, so a zeroed witness yields a
    /// bit-identical R1CS. No proof needed; no runtime dump path.
    pub fn canonical_shape(pp_digest: Scalar1) -> Result<Self, ExtractError> {
        let z = ArkFr::from(0u64);
        Ok(Self {
            params: params_from_embedded().map_err(ExtractError::Serialize)?,
            pp_digest: primary_to_ark_fr(pp_digest),
            comm_W_x: z,
            comm_W_y: z,
            comm_E_x: z,
            comm_E_y: z,
            u_as_base: z,
            x0_limbs: [z; 4],
            x1_limbs: [z; 4],
            ri_primary: z,
        })
    }

    /// Build the 18-element absorb sequence for the primary Neptune hash.
    ///
    /// Order matches nova-snark `nova/mod.rs:598-609`. For `z_arity = 1`
    /// (used by `TrivialIncrementCircuit` and the chain's real block circuit)
    /// the sequence has exactly 18 elements.
    pub fn absorb_seq(&self, num_steps: u64, z0: &[ArkFr], zi: &[ArkFr]) -> Vec<ArkFr> {
        let mut seq = Vec::with_capacity(4 + z0.len() + zi.len() + 13);
        seq.push(self.pp_digest);
        seq.push(ArkFr::from(num_steps));
        seq.extend_from_slice(z0);
        seq.extend_from_slice(zi);
        seq.push(self.comm_W_x);
        seq.push(self.comm_W_y);
        seq.push(self.comm_E_x);
        seq.push(self.comm_E_y);
        seq.push(self.u_as_base);
        seq.extend_from_slice(&self.x0_limbs);
        seq.extend_from_slice(&self.x1_limbs);
        seq.push(self.ri_primary);
        seq
    }
}

/// Extract all Section 2 witness fields from a live `RecursiveSNARK`.
///
/// * `rs` — the running accumulator (after >= 1 `prove_step` call).
/// * `pp_digest` — `PublicParams::digest()` from the same setup call.
/// * `dump_path` — path to the `dump-neptune-constants` JSON dump
///   (typically `/tmp/neptune-bn256-standard.json`).
#[allow(non_snake_case)]
pub fn extract_section2_witness<P: AsRef<std::path::Path>>(
    rs: &RecursiveSNARK<E1, E2, TrivialIncrementCircuit>,
    pp_digest: Scalar1,
    dump_path: P,
) -> Result<Section2Witness, ExtractError> {
    let params = params_from_dump_path(dump_path).map_err(ExtractError::Serialize)?;

    let v = serde_json::to_value(rs).map_err(|e| ExtractError::Serialize(e.to_string()))?;

    let r_U = v.get("r_U_secondary").ok_or(ExtractError::MissingPath)?;

    let (comm_W_x, comm_W_y) = extract_commitment_coords(r_U, "comm_W")?;
    let (comm_E_x, comm_E_y) = extract_commitment_coords(r_U, "comm_E")?;

    let u_as_base = extract_secondary_as_base(r_U, "u")?;

    let x_arr = r_U
        .get("X")
        .and_then(|x| x.as_array())
        .ok_or(ExtractError::MissingPath)?;
    if x_arr.len() < 2 {
        return Err(ExtractError::TooFewHashes(x_arr.len()));
    }

    let x0_limbs = secondary_hex_to_limbs(x_arr[0].as_str(), 0)?;
    let x1_limbs = secondary_hex_to_limbs(x_arr[1].as_str(), 1)?;

    let ri_primary = extract_primary_hex(&v, "ri_primary")?;

    Ok(Section2Witness {
        params,
        pp_digest: primary_to_ark_fr(pp_digest),
        comm_W_x,
        comm_W_y,
        comm_E_x,
        comm_E_y,
        u_as_base,
        x0_limbs,
        x1_limbs,
        ri_primary,
    })
}

/// Decompress a grumpkin affine point from the `comm` hex field inside
/// a commitment object, returning `(x, y)` as primary-field elements.
fn extract_commitment_coords(
    parent: &Value,
    field_name: &str,
) -> Result<(ArkFr, ArkFr), ExtractError> {
    let hex = parent
        .get(field_name)
        .and_then(|c| c.get("comm"))
        .and_then(|c| c.as_str())
        .ok_or(ExtractError::MissingPath)?;

    let stripped = hex.strip_prefix("0x").unwrap_or(hex);
    let bytes_le = hex::decode(stripped).map_err(|e| ExtractError::HexParseFailed {
        index: 0,
        reason: format!("{field_name}.comm hex decode: {e}"),
    })?;
    if bytes_le.len() != 32 {
        return Err(ExtractError::HexParseFailed {
            index: 0,
            reason: format!(
                "{field_name}.comm: expected 32 bytes, got {}",
                bytes_le.len()
            ),
        });
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes_le);

    // Use GroupEncoding::from_bytes via Repr<32> wrapper.
    // `<GrumpkinAffine as GroupEncoding>::Repr` is `halo2curves::serde::Repr<32>`,
    // a newtype over `[u8; 32]` with `From<[u8; 32]>`.
    let repr: <GrumpkinAffine as GroupEncoding>::Repr = arr.into();
    let affine =
        Option::<GrumpkinAffine>::from(GrumpkinAffine::from_bytes(&repr)).ok_or_else(|| {
            ExtractError::HexParseFailed {
                index: 0,
                reason: format!("{field_name}.comm: could not decompress grumpkin point"),
            }
        })?;

    Ok((primary_to_ark_fr(affine.x), primary_to_ark_fr(affine.y)))
}

/// Parse a secondary scalar from `parent[field_name]` hex and reinterpret
/// as a primary-field element via `secondary_to_ark_fr_lossy`.
fn extract_secondary_as_base(parent: &Value, field_name: &str) -> Result<ArkFr, ExtractError> {
    let hex = parent
        .get(field_name)
        .and_then(|v| v.as_str())
        .ok_or(ExtractError::MissingPath)?;
    let s = parse_secondary_hex(hex, 0)?;
    Ok(secondary_to_ark_fr_lossy(s))
}

/// Split a secondary-scalar hex string into 4 x u64 LE limbs
/// (`nat_to_limbs(x, 64, 4)`), each embedded as `ArkFr`.
fn secondary_hex_to_limbs(hex: Option<&str>, index: usize) -> Result<[ArkFr; 4], ExtractError> {
    let s = parse_secondary_hex(hex.unwrap_or(""), index)?;
    let le_bytes: [u8; 32] = s.to_repr().into();
    Ok([
        ArkFr::from(u64::from_le_bytes(le_bytes[0..8].try_into().unwrap())),
        ArkFr::from(u64::from_le_bytes(le_bytes[8..16].try_into().unwrap())),
        ArkFr::from(u64::from_le_bytes(le_bytes[16..24].try_into().unwrap())),
        ArkFr::from(u64::from_le_bytes(le_bytes[24..32].try_into().unwrap())),
    ])
}

/// Parse a top-level primary-scalar hex field and convert to `ArkFr`.
fn extract_primary_hex(parent: &Value, field_name: &str) -> Result<ArkFr, ExtractError> {
    use crate::scalar_adapter::PrimaryScalar;
    let hex = parent
        .get(field_name)
        .and_then(|v| v.as_str())
        .ok_or(ExtractError::MissingPath)?;
    let stripped = hex.strip_prefix("0x").unwrap_or(hex);
    let bytes_le = hex::decode(stripped).map_err(|e| ExtractError::HexParseFailed {
        index: 0,
        reason: format!("{field_name} hex decode: {e}"),
    })?;
    if bytes_le.len() != 32 {
        return Err(ExtractError::HexParseFailed {
            index: 0,
            reason: format!("{field_name}: expected 32 bytes, got {}", bytes_le.len()),
        });
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes_le);
    let repr = <PrimaryScalar as PrimeField>::Repr::from(arr);
    let s = PrimaryScalar::from_repr_vartime(repr).ok_or_else(|| ExtractError::HexParseFailed {
        index: 0,
        reason: format!("{field_name}: bytes not a canonical primary scalar"),
    })?;
    Ok(primary_to_ark_fr(s))
}

/// Parse a secondary scalar from a hex string (with or without "0x").
fn parse_secondary_hex(hex: &str, index: usize) -> Result<SecondaryScalar, ExtractError> {
    let stripped = hex.strip_prefix("0x").unwrap_or(hex);
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
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes_le);
    let repr = <SecondaryScalar as PrimeField>::Repr::from(arr);
    SecondaryScalar::from_repr_vartime(repr).ok_or_else(|| ExtractError::HexParseFailed {
        index,
        reason: "bytes not a canonical secondary scalar".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ff::Zero;

    fn zero_witness() -> Section2Witness {
        use crate::neptune_permutation_gadget::NeptuneSparseMatrix;
        Section2Witness {
            params: NeptuneParams {
                width: 3,
                full_rounds: 8,
                partial_rounds: 57,
                compressed_ark: vec![ArkFr::zero(); 8 * 3 + 57],
                plain_mds: vec![vec![ArkFr::zero(); 3]; 3],
                pre_sparse_mds: vec![vec![ArkFr::zero(); 3]; 3],
                sparse_matrices: vec![
                    NeptuneSparseMatrix::new(
                        vec![ArkFr::zero(); 3],
                        vec![ArkFr::zero(); 2],
                    );
                    57
                ],
            },
            pp_digest: ArkFr::from(1u64),
            comm_W_x: ArkFr::from(2u64),
            comm_W_y: ArkFr::from(3u64),
            comm_E_x: ArkFr::from(4u64),
            comm_E_y: ArkFr::from(5u64),
            u_as_base: ArkFr::from(6u64),
            x0_limbs: [
                ArkFr::from(7u64),
                ArkFr::from(8u64),
                ArkFr::from(9u64),
                ArkFr::from(10u64),
            ],
            x1_limbs: [
                ArkFr::from(11u64),
                ArkFr::from(12u64),
                ArkFr::from(13u64),
                ArkFr::from(14u64),
            ],
            ri_primary: ArkFr::from(15u64),
        }
    }

    #[test]
    fn absorb_seq_length_z_arity_1() {
        let w = zero_witness();
        let z0 = vec![ArkFr::from(0u64)];
        let zi = vec![ArkFr::from(2u64)];
        let seq = w.absorb_seq(2, &z0, &zi);
        // pp_digest + num_steps + z0[0] + zi[0] + W_x + W_y + E_x + E_y + u
        // + x0_limbs x 4 + x1_limbs x 4 + ri_primary = 18
        assert_eq!(
            seq.len(),
            18,
            "absorb sequence must be 18 elements for z_arity=1"
        );
    }

    #[test]
    fn absorb_seq_first_four_slots() {
        let w = zero_witness();
        let z0 = vec![ArkFr::from(0u64)];
        let zi = vec![ArkFr::from(3u64)];
        let seq = w.absorb_seq(3, &z0, &zi);
        assert_eq!(seq[0], ArkFr::from(1u64), "seq[0] = pp_digest");
        assert_eq!(seq[1], ArkFr::from(3u64), "seq[1] = num_steps");
        assert_eq!(seq[2], ArkFr::from(0u64), "seq[2] = z0[0]");
        assert_eq!(seq[3], ArkFr::from(3u64), "seq[3] = zi[0]");
    }

    #[test]
    fn absorb_seq_comm_and_u_slots() {
        let w = zero_witness();
        let seq = w.absorb_seq(1, &[ArkFr::from(0u64)], &[ArkFr::from(1u64)]);
        assert_eq!(seq[4], ArkFr::from(2u64), "seq[4] = comm_W_x");
        assert_eq!(seq[5], ArkFr::from(3u64), "seq[5] = comm_W_y");
        assert_eq!(seq[6], ArkFr::from(4u64), "seq[6] = comm_E_x");
        assert_eq!(seq[7], ArkFr::from(5u64), "seq[7] = comm_E_y");
        assert_eq!(seq[8], ArkFr::from(6u64), "seq[8] = u_as_base");
    }

    #[test]
    fn absorb_seq_limb_and_ri_slots() {
        let w = zero_witness();
        let seq = w.absorb_seq(1, &[ArkFr::from(0u64)], &[ArkFr::from(1u64)]);
        assert_eq!(seq[9], ArkFr::from(7u64));
        assert_eq!(seq[10], ArkFr::from(8u64));
        assert_eq!(seq[11], ArkFr::from(9u64));
        assert_eq!(seq[12], ArkFr::from(10u64));
        assert_eq!(seq[13], ArkFr::from(11u64));
        assert_eq!(seq[14], ArkFr::from(12u64));
        assert_eq!(seq[15], ArkFr::from(13u64));
        assert_eq!(seq[16], ArkFr::from(14u64));
        assert_eq!(seq[17], ArkFr::from(15u64), "seq[17] = ri_primary");
    }

    #[test]
    fn secondary_hex_to_limbs_zero_scalar() {
        let zero_hex = "0000000000000000000000000000000000000000000000000000000000000000";
        let limbs = secondary_hex_to_limbs(Some(zero_hex), 0).expect("zero limbs");
        for (i, l) in limbs.iter().enumerate() {
            assert_eq!(*l, ArkFr::zero(), "limb[{i}] must be zero for zero scalar");
        }
    }

    #[test]
    fn secondary_hex_to_limbs_one_scalar() {
        // LE: first byte 0x01, rest 0x00 -> limb[0]=1, rest=0
        let one_hex = "0100000000000000000000000000000000000000000000000000000000000000";
        let limbs = secondary_hex_to_limbs(Some(one_hex), 0).expect("one limbs");
        assert_eq!(
            limbs[0],
            ArkFr::from(1u64),
            "limb[0] must be 1 for scalar=1"
        );
        assert_eq!(limbs[1], ArkFr::from(0u64));
        assert_eq!(limbs[2], ArkFr::from(0u64));
        assert_eq!(limbs[3], ArkFr::from(0u64));
    }

    #[test]
    fn secondary_hex_to_limbs_max_u64_in_first_limb() {
        // First 8 bytes = 0xff..ff (LE u64::MAX), rest zero.
        // limb[0] = u64::MAX, limbs[1..] = 0.
        let hex = "ffffffffffffffff000000000000000000000000000000000000000000000000";
        let limbs = secondary_hex_to_limbs(Some(hex), 0).expect("max u64 limbs");
        assert_eq!(limbs[0], ArkFr::from(u64::MAX));
        assert_eq!(limbs[1], ArkFr::from(0u64));
        assert_eq!(limbs[2], ArkFr::from(0u64));
        assert_eq!(limbs[3], ArkFr::from(0u64));
    }

    #[test]
    fn secondary_hex_to_limbs_distinct_values_per_limb() {
        // Pin that limbs are read in LE-byte order. Bytes
        // [01, 0, ..., 0, 02, 0, ..., 0, 03, 0, ..., 0, 04, 0, ..., 0]
        // ⇒ limb[0]=1, limb[1]=2, limb[2]=3, limb[3]=4
        let mut bytes = [0u8; 32];
        bytes[0] = 0x01;
        bytes[8] = 0x02;
        bytes[16] = 0x03;
        bytes[24] = 0x04;
        let hex = hex::encode(bytes);
        let limbs = secondary_hex_to_limbs(Some(&hex), 0).expect("layout limbs");
        assert_eq!(limbs[0], ArkFr::from(1u64));
        assert_eq!(limbs[1], ArkFr::from(2u64));
        assert_eq!(limbs[2], ArkFr::from(3u64));
        assert_eq!(limbs[3], ArkFr::from(4u64));
    }

    #[test]
    fn parse_secondary_hex_invalid_chars_errors() {
        let err = parse_secondary_hex("nothex_yetcanonical_32byte_string_xx", 4)
            .expect_err("non-hex chars must fail");
        match err {
            ExtractError::HexParseFailed { index, reason } => {
                assert_eq!(index, 4);
                assert!(
                    reason.contains("hex") || reason.contains("byte"),
                    "reason should mention hex/byte: {reason}"
                );
            }
            other => panic!("expected HexParseFailed, got {other:?}"),
        }
    }

    #[test]
    fn parse_secondary_hex_wrong_length_errors() {
        // Valid hex but only 8 bytes (16 chars).
        let err = parse_secondary_hex("deadbeefdeadbeef", 2).expect_err("wrong length must fail");
        match err {
            ExtractError::HexParseFailed { index, reason } => {
                assert_eq!(index, 2);
                assert!(
                    reason.contains("32 bytes"),
                    "reason should mention 32 bytes: {reason}"
                );
            }
            other => panic!("expected HexParseFailed, got {other:?}"),
        }
    }

    #[test]
    fn parse_secondary_hex_accepts_0x_prefix() {
        let zero_hex_prefixed = format!("0x{}", "00".repeat(32));
        let zero_hex_bare = "00".repeat(32);
        let p_prefixed = parse_secondary_hex(&zero_hex_prefixed, 0).expect("prefixed");
        let p_bare = parse_secondary_hex(&zero_hex_bare, 0).expect("bare");
        // Both must produce the same canonical scalar.
        assert_eq!(p_prefixed, p_bare);
    }

    #[test]
    fn absorb_seq_handles_empty_z0_zi() {
        // Degenerate case: z0/zi both empty. Length should be 4 + 0 + 0 + 13 = 17.
        let w = zero_witness();
        let seq = w.absorb_seq(7, &[], &[]);
        // Layout: pp_digest, num_steps, comm_W_x/y, comm_E_x/y, u, 4*x0, 4*x1, ri
        assert_eq!(seq.len(), 2 + 4 + 1 + 4 + 4 + 1);
        assert_eq!(seq[0], ArkFr::from(1u64), "pp_digest");
        assert_eq!(seq[1], ArkFr::from(7u64), "num_steps=7");
        assert_eq!(
            seq.last().copied().unwrap(),
            ArkFr::from(15u64),
            "ri_primary tail"
        );
    }

    #[test]
    fn absorb_seq_handles_multi_element_z_arity() {
        // z_arity=2: z0 and zi each carry 2 elements. Length should be
        // 2 + 2 + 2 + 4 + 1 + 4 + 4 + 1 = 20.
        let w = zero_witness();
        let z0 = vec![ArkFr::from(100u64), ArkFr::from(101u64)];
        let zi = vec![ArkFr::from(200u64), ArkFr::from(201u64)];
        let seq = w.absorb_seq(2, &z0, &zi);
        assert_eq!(seq.len(), 20);
        assert_eq!(seq[2], ArkFr::from(100u64), "z0[0]");
        assert_eq!(seq[3], ArkFr::from(101u64), "z0[1]");
        assert_eq!(seq[4], ArkFr::from(200u64), "zi[0]");
        assert_eq!(seq[5], ArkFr::from(201u64), "zi[1]");
    }

    #[test]
    fn absorb_seq_with_zero_steps_does_not_panic() {
        // num_steps=0 is allowed (a fresh-from-`new` RecursiveSNARK).
        let w = zero_witness();
        let seq = w.absorb_seq(0, &[ArkFr::from(0u64)], &[ArkFr::from(0u64)]);
        assert_eq!(seq.len(), 18);
        assert_eq!(
            seq[1],
            ArkFr::from(0u64),
            "num_steps=0 must be encoded as Fr::zero"
        );
    }

    /// Full end-to-end integration test. Requires neptune constants dump.
    #[test]
    #[ignore]
    fn extract_section2_witness_from_real_fixture() {
        use crate::recursive_snark_fixture::generate_fixture_with_digest;

        let dump = std::path::Path::new("/tmp/neptune-bn256-standard.json");
        if !dump.exists() {
            eprintln!("SKIP: dump file not present at {}", dump.display());
            return;
        }

        let (rs, pp_digest) = generate_fixture_with_digest(2).expect("generate fixture");
        let w = extract_section2_witness(&rs, pp_digest, dump).expect("extract witness");

        let z0 = vec![ArkFr::from(0u64)];
        let zi = vec![ArkFr::from(2u64)];
        let seq = w.absorb_seq(2, &z0, &zi);
        assert_eq!(seq.len(), 18);
        assert_ne!(w.pp_digest, ArkFr::zero(), "pp_digest must be non-zero");
        assert!(
            w.comm_W_x != ArkFr::zero() || w.comm_W_y != ArkFr::zero(),
            "real fixture must produce a non-trivial comm_W point"
        );
        assert_ne!(
            w.ri_primary,
            ArkFr::zero(),
            "ri_primary non-zero after folding"
        );
    }
}
