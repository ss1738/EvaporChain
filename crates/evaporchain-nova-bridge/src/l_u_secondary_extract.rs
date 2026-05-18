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
use crate::scalar_adapter::{secondary_to_ark_fr_lossy, SecondaryScalar};
use ark_bn254::Fr as ArkFr;
use ff::PrimeField;
use nova_snark::nova::RecursiveSNARK;

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
