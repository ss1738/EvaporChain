//! Phase 2.2-section-2 BESPOKE: port of neptune's
//! `compress_round_constants` (first slice).
//!
//! This module begins the final BESPOKE wedge — closing the residual
//! hash-parity gap between our `fully_aligned_poseidon_config` and
//! neptune's actual sponge output (PR #100 narrowed the gap to
//! SBOX-trick partial-round fusion).
//!
//! # What's ported in THIS PR
//!
//! The FIRST piece of neptune's `preprocessing.rs::compress_round_constants`:
//!
//! ```text
//! res.extend(round_keys(0));          // already correct — plain ARK
//!
//! let end = if unpreprocessed > 0 {   // = half_full_rounds - 1 for our params
//!     half_full_rounds
//! } else {
//!     half_full_rounds - 1
//! };
//! for i in 0..end {
//!     let next_round = round_keys(i + 1);
//!     let inverted = left_apply_matrix(inverse_matrix, next_round);
//!     res.extend(inverted);
//! }
//! ```
//!
//! For BN254 / arity-24 / Strength::Standard:
//! - `full_rounds = 8`, `partial_rounds = 59`
//! - `half_full_rounds = 4`
//! - `partial_preprocessed = partial_rounds = 59` → `unpreprocessed = 0`
//! - `end = half_full_rounds - 1 = 3`
//!
//! So `compress_first_full_rounds` emits 1 (plain round 0) + 3
//! (inverse-transformed rounds 1, 2, 3) = 4 rounds × 25 width =
//! **100 entries**. They match neptune's `crc[0..100]`.
//!
//! Rounds 4..7 (the SECOND half-full set) + 59 partial rounds get
//! the backward-fold preprocessing in a follow-up PR.

use crate::mds_linalg::left_apply_matrix;
use ark_bn254::Fr;

/// Compress the first 4 full rounds per neptune's
/// `compress_round_constants` first slice.
///
/// Inputs:
/// - `plain_ark`: flat Vec<Fr> of length ≥ `4 * width` (typically
///   the full `(rf + rp) * width` ARK vector from
///   `grain_lfsr::generate_round_constants_bn254_arity_24_standard`)
/// - `inverse_mds`: real neptune `mds.m_inv` from PR #80's JSON dump
/// - `width`: state width (rate + capacity = 25 for our params)
/// - `half_full_rounds`: = full_rounds / 2 = 4 for our params
///
/// Returns: `Vec<Fr>` of length `half_full_rounds * width = 100`
/// matching neptune's `crc[0..100]` byte-for-byte (if the LFSR
/// ARK is correct, which PR #97 confirmed for round 0).
///
/// # Layout of output
///
/// - `output[0..width]`       = plain ARK round 0
/// - `output[width..2*width]` = `inverse_mds · plain_ark_round[1]`
/// - `output[2*width..3*width]` = `inverse_mds · plain_ark_round[2]`
/// - `output[3*width..4*width]` = `inverse_mds · plain_ark_round[3]`
pub fn compress_first_full_rounds(
    plain_ark: &[Fr],
    inverse_mds: &[Vec<Fr>],
    width: usize,
    half_full_rounds: usize,
) -> Vec<Fr> {
    assert!(
        plain_ark.len() >= half_full_rounds * width,
        "plain_ark too short: have {}, need ≥ {}",
        plain_ark.len(),
        half_full_rounds * width
    );
    assert_eq!(
        inverse_mds.len(),
        width,
        "inverse_mds must be width×width"
    );

    let mut res: Vec<Fr> = Vec::with_capacity(half_full_rounds * width);
    // Round 0: plain ARK extended.
    res.extend_from_slice(&plain_ark[0..width]);
    // Rounds 1..half_full_rounds: inverse-MDS-transformed.
    // The `end` from neptune's code is `half_full_rounds - 1` when
    // `unpreprocessed > 0` is false (our case: all partial rounds
    // are preprocessed).
    let end = half_full_rounds.saturating_sub(1);
    for i in 0..end {
        let next_round = &plain_ark[(i + 1) * width..(i + 2) * width];
        let inverted = left_apply_matrix(inverse_mds, next_round);
        res.extend_from_slice(&inverted);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grain_lfsr::generate_round_constants_bn254_arity_24_standard;
    use crate::neptune_dump_parser::{extract_compressed_round_constants, extract_mds_inverse_matrix};

    /// **THE empirical test.** Run the first-half-full preprocessing
    /// using our LFSR-generated plain ARK + real neptune
    /// `mds.m_inv`. Compare to neptune's `crc[0..100]` byte-for-byte.
    ///
    /// If MATCHES: the FIRST piece of `compress_round_constants` is
    /// correct, and crc[0..100] is now byte-verified end-to-end.
    /// Remaining work: the second-half + partial-round backward fold.
    ///
    /// If MISMATCH: something's still off in our LFSR (which is
    /// unlikely after PR #97), or in the inverse-MDS interpretation,
    /// or in our row/col convention for `left_apply_matrix`.
    #[test]
    #[ignore = "requires /tmp/neptune-bn256-standard.json"]
    fn first_100_compressed_match_neptune_crc() {
        let plain_ark = generate_round_constants_bn254_arity_24_standard();
        let m_inv = extract_mds_inverse_matrix("/tmp/neptune-bn256-standard.json")
            .expect("load m_inv");
        let crc = extract_compressed_round_constants("/tmp/neptune-bn256-standard.json")
            .expect("load crc");

        let ours = compress_first_full_rounds(&plain_ark, &m_inv, 25, 4);
        assert_eq!(ours.len(), 100);

        let mut mismatches: Vec<usize> = Vec::new();
        for i in 0..100 {
            if ours[i] != crc[i] {
                mismatches.push(i);
            }
        }
        eprintln!(
            "first-half compressed parity: {} of 100 mismatches",
            mismatches.len()
        );
        if !mismatches.is_empty() {
            eprintln!("  Mismatch indices: {:?}", &mismatches[..mismatches.len().min(10)]);
        }
        assert!(
            mismatches.is_empty(),
            "first-half compression must match neptune crc[0..100]"
        );
    }

    /// Shape contract: output length is `half_full_rounds * width`.
    #[test]
    fn output_length_matches_contract() {
        // 4×4 identity inverse-MDS + 16-entry plain ARK = 4×4 output
        let identity_4 = vec![
            vec![Fr::from(1u64), Fr::from(0u64), Fr::from(0u64), Fr::from(0u64)],
            vec![Fr::from(0u64), Fr::from(1u64), Fr::from(0u64), Fr::from(0u64)],
            vec![Fr::from(0u64), Fr::from(0u64), Fr::from(1u64), Fr::from(0u64)],
            vec![Fr::from(0u64), Fr::from(0u64), Fr::from(0u64), Fr::from(1u64)],
        ];
        let plain_ark: Vec<Fr> = (0..16).map(|i| Fr::from(i as u64)).collect();
        let out = compress_first_full_rounds(&plain_ark, &identity_4, 4, 4);
        assert_eq!(out.len(), 16);
        // Round 0: plain extension → 0,1,2,3
        for i in 0..4 {
            assert_eq!(out[i], Fr::from(i as u64));
        }
        // Identity inverse-MDS leaves subsequent rounds unchanged.
        for r in 1..4 {
            for c in 0..4 {
                assert_eq!(out[r * 4 + c], Fr::from((r * 4 + c) as u64));
            }
        }
    }
}
