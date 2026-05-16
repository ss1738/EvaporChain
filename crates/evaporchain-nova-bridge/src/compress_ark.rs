//! Port of neptune's `compress_round_constants` (`preprocessing.rs`).
//!
//! Reproduces neptune's compressed-ARK byte-for-byte from our LFSR
//! plain ARK + neptune's `mds.m_inv`. With this in place, the
//! constants layer of Section 2 is byte-correct vs neptune
//! (PR #103: `FULL compressed parity: 0 of 259 mismatches`).
//!
//! # Public API
//!
//! - [`compress_first_full_rounds`]: first slice only (round 0
//!   plain + rounds 1..3 inverse-MDS-transformed = 100 entries).
//!   Matches `neptune crc[0..100]`. Useful for isolated testing.
//! - [`compress_full`]: full algorithm — emits all 259 entries
//!   matching neptune `crc[0..259]` for BN254 / arity-24 /
//!   `Strength::Standard`.
//!
//! # Parameters (BN254 / arity-24 / Standard)
//!
//! `full_rounds = 8`, `partial_rounds = 59`, `width = 25`,
//! `half_full_rounds = 4`, `partial_preprocessed = partial_rounds`
//! (so `unpreprocessed = 0`). Output length:
//! `full_rounds * width + partial_rounds = 200 + 59 = 259`.
//!
//! # Algorithm layout (matches neptune `preprocessing.rs:30-170`)
//!
//! 1. Round 0 plain (25 entries)
//! 2. `inverse_mds * rounds[1..3]` (3 × 25 = 75 entries)
//! 3. Backward partial-round fold over 59 rounds → `round_acc` +
//!    `partial_keys`
//! 4. `inverse_mds * round_acc` (25 entries)
//! 5. `partial_keys` popped in reverse (59 entries)
//! 6. `inverse_mds * rounds[64..66]` (3 × 25 = 75 entries — second
//!    half-full)
//!
//! Total: 25 + 75 + 25 + 59 + 75 = 259.

use crate::mds_linalg::{left_apply_matrix, vec_add};
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

/// Full port of neptune's `compress_round_constants` for the
/// `partial_preprocessed = partial_rounds` case (the only one
/// nova-snark uses). Returns the complete compressed-ARK vector
/// of length `full_rounds * width + partial_rounds`.
///
/// Algorithm (from neptune `preprocessing.rs:30-170`):
///
/// 1. **First-half-full-rounds slice**: round 0 plain, rounds 1..3
///    inverse-MDS-transformed. (Already implemented in
///    `compress_first_full_rounds`.)
/// 2. **Backward partial-round fold**: walk partial rounds from
///    last to first, accumulating an `acc` state; each step
///    inverts via MDS_inv, captures the top entry as a partial
///    constant, zeros the top, then adds the previous round's
///    plain ARK.
/// 3. **Append `inverse_mds * round_acc`** (width entries).
/// 4. **Pop partial_keys in reverse** (one entry per partial round).
/// 5. **Second-half-full-rounds slice**: rounds (last 3 of the
///    8 full rounds), inverse-MDS-transformed.
///
/// For our parameters (BN254 / arity-24 / Standard):
///   full_rounds = 8, partial_rounds = 59, half_full = 4
///   output length = 8*25 + 59 = 259 (= neptune's `crc.len()`)
pub fn compress_full(
    plain_ark: &[Fr],
    inverse_mds: &[Vec<Fr>],
    width: usize,
    full_rounds: usize,
    partial_rounds: usize,
) -> Vec<Fr> {
    let half_full = full_rounds / 2;
    let total_rounds = full_rounds + partial_rounds;
    let expected_len = full_rounds * width + partial_rounds;
    assert!(
        plain_ark.len() >= total_rounds * width,
        "plain_ark too short: have {}, need ≥ {}",
        plain_ark.len(),
        total_rounds * width
    );
    assert_eq!(inverse_mds.len(), width, "inverse_mds must be width×width");

    // Helper: extract one row's worth of plain ARK as &[Fr].
    let round_keys = |r: usize| -> &[Fr] {
        &plain_ark[r * width..(r + 1) * width]
    };

    // ── Slice 1: first half-full rounds ────────────────────
    let mut res: Vec<Fr> = Vec::with_capacity(expected_len);
    res.extend_from_slice(round_keys(0));
    let end = half_full.saturating_sub(1); // 3 for our params
    for i in 0..end {
        let inverted = left_apply_matrix(inverse_mds, round_keys(i + 1));
        res.extend_from_slice(&inverted);
    }

    // ── Slice 2: backward partial fold ─────────────────────
    let mut partial_keys: Vec<Fr> = Vec::new();
    let final_round = half_full + partial_rounds; // 4 + 59 = 63
    let final_round_key: Vec<Fr> = round_keys(final_round).to_vec();
    let round_acc = (0..partial_rounds)
        .map(|i| round_keys(final_round - i - 1).to_vec())
        .fold(final_round_key, |acc, previous_round_keys| {
            let mut inverted = left_apply_matrix(inverse_mds, &acc);
            partial_keys.push(inverted[0]);
            inverted[0] = Fr::from(0u64);
            vec_add(&previous_round_keys, &inverted)
        });

    // ── Slice 3: extend with inverse_mds * round_acc ───────
    res.extend_from_slice(&left_apply_matrix(inverse_mds, &round_acc));

    // ── Slice 4: partial keys popped in reverse ────────────
    while let Some(x) = partial_keys.pop() {
        res.push(x);
    }

    // ── Slice 5: second half-full rounds ───────────────────
    // start = half_full + partial_rounds = 63 = final_round (already used)
    // Rounds processed: (1..half_full) = 1, 2, 3 → round indices
    //   start + 1, start + 2, start + 3 = 64, 65, 66
    let start = half_full + partial_rounds;
    for i in 1..half_full {
        let inverted = left_apply_matrix(inverse_mds, round_keys(i + start));
        res.extend_from_slice(&inverted);
    }

    debug_assert_eq!(
        res.len(),
        expected_len,
        "compress_full output length must equal full_rounds*width + partial_rounds"
    );
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

    /// **THE final parity test.** Run `compress_full` on our
    /// LFSR-generated plain ARK + neptune's real `mds.m_inv` and
    /// compare to neptune's full `crc[0..259]` byte-for-byte.
    ///
    /// If MATCHES: neptune's compressed ARK is now reproducible
    /// from our LFSR + matrix primitives, and the constants
    /// half of Section 2 is BYTE-COMPLETE. Plug into arkworks
    /// PoseidonSponge and the hash parity canary
    /// (PR #98) flips.
    #[test]
    #[ignore = "requires /tmp/neptune-bn256-standard.json"]
    fn full_compressed_match_neptune_crc() {
        let plain_ark = generate_round_constants_bn254_arity_24_standard();
        let m_inv = extract_mds_inverse_matrix("/tmp/neptune-bn256-standard.json")
            .expect("load m_inv");
        let crc = extract_compressed_round_constants("/tmp/neptune-bn256-standard.json")
            .expect("load crc");

        let ours = compress_full(&plain_ark, &m_inv, 25, 8, 59);
        assert_eq!(ours.len(), 259, "expected length 8*25 + 59 = 259");
        assert_eq!(crc.len(), 259, "neptune crc must also be 259");

        let mut mismatches: Vec<usize> = Vec::new();
        for i in 0..259 {
            if ours[i] != crc[i] {
                mismatches.push(i);
            }
        }
        eprintln!(
            "FULL compressed parity: {} of 259 mismatches",
            mismatches.len()
        );
        if !mismatches.is_empty() {
            eprintln!(
                "  first 20 mismatches: {:?}",
                &mismatches[..mismatches.len().min(20)]
            );
        }
        assert!(
            mismatches.is_empty(),
            "compress_full must reproduce neptune crc byte-for-byte"
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

    // ─── Coverage push (2026-05-16): lift compress_ark.rs from ~32% ───

    fn identity_n(n: usize) -> Vec<Vec<Fr>> {
        (0..n)
            .map(|i| (0..n).map(|j| Fr::from(if i == j { 1u64 } else { 0u64 })).collect())
            .collect()
    }

    /// Round 0 is plain ARK (not inverse-MDS-transformed) — sanity check.
    #[test]
    fn first_round_is_plain_ark() {
        let id = identity_n(4);
        let plain_ark: Vec<Fr> = (10..26).map(|i| Fr::from(i as u64)).collect();
        let out = compress_first_full_rounds(&plain_ark, &id, 4, 4);
        for i in 0..4 {
            assert_eq!(out[i], Fr::from((10 + i) as u64));
        }
    }

    /// `compress_first_full_rounds` panics if `plain_ark` is shorter
    /// than `half_full_rounds * width`.
    #[test]
    #[should_panic(expected = "plain_ark too short")]
    fn compress_first_panics_on_short_plain_ark() {
        let id = identity_n(4);
        let too_short: Vec<Fr> = vec![Fr::from(0u64); 8]; // need 16 for half=4, width=4
        compress_first_full_rounds(&too_short, &id, 4, 4);
    }

    /// `compress_first_full_rounds` panics if the inverse_mds matrix
    /// width doesn't match the declared `width` parameter.
    #[test]
    #[should_panic(expected = "inverse_mds must be width×width")]
    fn compress_first_panics_on_inverse_mds_wrong_size() {
        let wrong = identity_n(3); // declared width=4 below
        let plain_ark: Vec<Fr> = vec![Fr::from(0u64); 16];
        compress_first_full_rounds(&plain_ark, &wrong, 4, 4);
    }

    /// `half_full_rounds = 1` is degenerate: output = first row only,
    /// no inverse-MDS application.
    #[test]
    fn compress_first_half_one_returns_plain_round_only() {
        let id = identity_n(4);
        let plain_ark: Vec<Fr> = (0..16).map(|i| Fr::from(i as u64)).collect();
        let out = compress_first_full_rounds(&plain_ark, &id, 4, 1);
        assert_eq!(out.len(), 4);
        for i in 0..4 {
            assert_eq!(out[i], Fr::from(i as u64));
        }
    }

    /// `compress_full` produces exactly `full_rounds * width +
    /// partial_rounds` entries.
    #[test]
    fn compress_full_output_length_invariant() {
        let id = identity_n(4);
        // width=4, full=2, partial=1 → total_rounds=3, need 12 plain_ark
        let plain_ark: Vec<Fr> = (0..16).map(|i| Fr::from(i as u64)).collect();
        let out = compress_full(&plain_ark, &id, 4, 2, 1);
        // expected = 2*4 + 1 = 9
        assert_eq!(out.len(), 9);
    }

    /// `compress_full` panics on short plain_ark.
    #[test]
    #[should_panic(expected = "plain_ark too short")]
    fn compress_full_panics_on_short_plain_ark() {
        let id = identity_n(4);
        // full=2, partial=1, width=4 → need ≥ 12, give 4.
        let plain_ark: Vec<Fr> = vec![Fr::from(0u64); 4];
        compress_full(&plain_ark, &id, 4, 2, 1);
    }

    /// `compress_full` panics on inverse_mds row-count mismatch.
    #[test]
    #[should_panic(expected = "inverse_mds must be width×width")]
    fn compress_full_panics_on_inverse_mds_wrong_size() {
        let wrong = identity_n(3);
        let plain_ark: Vec<Fr> = vec![Fr::from(0u64); 16];
        compress_full(&plain_ark, &wrong, 4, 2, 1);
    }
}
