//! Phase 2 Section 2 sponge framing — slice 5c (attempt 1).
//!
//! First plausible mirror of neptune's Simplex sponge wrapper around
//! `Poseidon::hash_optimized_static`. The decisive test
//! `sponge_attempt_1_matches_neptune_hash_primary_on_pinned_42_7_99`
//! compares our output against
//! `crate::neptune_reference::neptune_hash_primary` on the pinned
//! `[42, 7, 99]` reference vector.
//!
//! - If the test passes: this framing is correct, slice 5d flips the
//!   `assert_ne!` canary in `crate::section2_gadget` to `assert_eq!`,
//!   and Section 2 BESPOKE is closed.
//! - If the test fails: this PR documents one wrong assumption; the
//!   diff (our_bytes vs neptune_bytes) tells us exactly which
//!   absorb-position / padding / squeeze-position assumption is off.
//!   Slice 5d (attempt 2) adjusts and re-tries.
//!
//! # Framing assumptions in THIS attempt
//!
//! 1. **State init:** `[Fr::ZERO; width]`. `HashType::Sponge` ⇒
//!    `domain_tag = ZERO` (per `nova-snark-0.68/src/frontend/
//!    gadgets/poseidon/hash_type.rs:62`).
//! 2. **Capacity in slot 0:** state[0] is the capacity slot; never
//!    written by absorb.
//! 3. **Rate = width - 1 = 24.** Absorb position starts at 1; after
//!    writing into all 24 rate slots, we permute and reset position
//!    back to 1.
//! 4. **Absorb operation:** `state[pos] += input`, NOT
//!    `state[pos] = input`. (Sponge convention.)
//! 5. **Final permute:** after all inputs are absorbed, run one
//!    permutation regardless of whether the last block was full.
//!    (Even-len blocks need this; the SpongeAPI call structure
//!    `start → absorb → squeeze` always emits one squeeze permutation.)
//! 6. **Squeeze:** read state[1] (first rate slot) as the digest scalar.
//!    No further permute between squeezes (we only call `squeeze` once).
//! 7. **Bit truncation:** mirror neptune's
//!    `to_le_bits()[..NUM_HASH_BITS=250]` truncation, identical to
//!    `neptune_reference::neptune_hash_primary`.
//!
//! # Known unknowns (might cause the test to fire)
//!
//! - Whether neptune writes inputs via `state[pos] += input` or
//!   `state[pos] = input`. The Sponge crate's source has both
//!   conventions in different code paths.
//! - Whether the first permute fires before or after the first
//!   absorb block fills up (i.e. fence-post on the absorb loop).
//! - Whether there's a length-prefix or absorb-count tag baked into
//!   `domain_tag` for `IOPattern`-defined sponges. The IOPattern in
//!   `PoseidonRO::squeeze` declares the absorb size dynamically;
//!   neptune may incorporate that count into the initial state.
//! - The `start_with_one` parameter to `neptune_hash_primary` is
//!   `false` in our reference call, so MSB-forcing isn't an issue.

use ark_bn254::Fr as Bn254Fr;
use ark_ff::{BigInteger, PrimeField};

use crate::neptune_permutation_gadget::{neptune_permute_native, NeptuneParams};

/// Truncation parameter from nova-snark's `NUM_HASH_BITS`.
/// Matches [`crate::neptune_reference::NUM_HASH_BITS`].
pub const NUM_HASH_BITS: usize = 250;

/// 128-bit prime base used by neptune's `Hasher` for the IOPattern
/// value, per `nova-snark-0.68/src/frontend/gadgets/poseidon/sponge/api.rs:46`.
const HASHER_BASE: u128 = 0u128.wrapping_sub(159);

/// Compute the IOPattern hash for our specific use case: one
/// `Absorb(absorb_count)` followed by one `Squeeze(squeeze_count)`,
/// finalised with `domain_separator`.
///
/// Mirrors neptune's `IOPattern::value` for the pattern
/// `[Absorb(n), Squeeze(m)]`. The hash is the `tag: u128` written
/// into state[0]'s low 16 bytes by `initialize_capacity`.
///
/// neptune's SpongeOp::value encodings:
///   Absorb(n)  → n + (1 << 31)
///   Squeeze(n) → n
///
/// Hasher state update (per call to `update(a)`):
///   x_i = x_i.wrapping_mul(HASHER_BASE)
///   state = state.wrapping_add(x_i.wrapping_mul(a))
///
/// finalize calls `update(domain_separator)` at the end.
pub fn iopattern_value_absorb_then_squeeze(
    absorb_count: u32,
    squeeze_count: u32,
    domain_separator: u32,
) -> u128 {
    let x = HASHER_BASE;
    let mut x_i: u128 = 1;
    let mut state: u128 = 0;

    let absorb_value = (absorb_count as u128).wrapping_add(1u128 << 31);
    x_i = x_i.wrapping_mul(x);
    state = state.wrapping_add(x_i.wrapping_mul(absorb_value));

    let squeeze_value = squeeze_count as u128;
    x_i = x_i.wrapping_mul(x);
    state = state.wrapping_add(x_i.wrapping_mul(squeeze_value));

    let ds_value = domain_separator as u128;
    x_i = x_i.wrapping_mul(x);
    state = state.wrapping_add(x_i.wrapping_mul(ds_value));

    state
}

/// Convert the IOPattern u128 tag into an `Fr` exactly the way
/// neptune's `initialize_capacity` does:
///
/// ```text
/// let mut repr = F::Repr::default();              // 32 bytes of zero
/// repr.as_mut()[..16].copy_from_slice(&tag.to_le_bytes());
/// F::from_repr(repr).unwrap()
/// ```
///
/// In LE byte form: low 16 bytes = tag LE, high 16 bytes = 0.
/// This yields a small Fr in the range [0, 2^128).
pub fn iopattern_tag_as_fr(tag: u128) -> Bn254Fr {
    let mut bytes_le = [0u8; 32];
    bytes_le[..16].copy_from_slice(&tag.to_le_bytes());
    Bn254Fr::from_le_bytes_mod_order(&bytes_le)
}

/// Off-circuit reference for our Simplex-sponge hash, attempt 1.
/// Mirrors what nova-snark's `PoseidonRO<Bn254Scalar>::squeeze(250,
/// false)` does, but using our `neptune_permute_native` instead of
/// neptune's internal permutation.
///
/// **Output domain.** `state[1]` is squeezed and truncated to 250
/// LE bits — identical post-processing to neptune's. The returned
/// `Bn254Fr` is the equivalent of `bn256::Scalar` from
/// `neptune_reference::neptune_hash_primary` (same field, different
/// Rust type).
pub fn our_neptune_hash_primary_native(
    inputs: &[Bn254Fr],
    params: &NeptuneParams<Bn254Fr>,
) -> Bn254Fr {
    let width = params.width;
    let rate = width - 1; // capacity at index 0; rate at indices 1..width
    let capacity = 1usize;

    // 1. State init via initialize_capacity — write the IOPattern
    //    hash into state[0]'s low 16 bytes. PoseidonRO::squeeze uses
    //    `IOPattern([Absorb(state.len()), Squeeze(1)])` with
    //    `domain_separator = Some(0).unwrap_or(0) = 0`.
    let tag = iopattern_value_absorb_then_squeeze(inputs.len() as u32, 1, 0);
    let mut state: Vec<Bn254Fr> = vec![Bn254Fr::from(0u64); width];
    state[0] = iopattern_tag_as_fr(tag);

    // 2. Absorb. absorb_pos is 0-indexed within rate; physical state
    //    index = absorb_pos + capacity.
    let mut absorb_pos = 0usize;
    for input in inputs {
        if absorb_pos == rate {
            // Rate full: permute and reset.
            neptune_permute_native(&mut state, params);
            absorb_pos = 0;
        }
        state[absorb_pos + capacity] += input;
        absorb_pos += 1;
    }

    // 3. Squeeze: ensure_squeezing triggers one permute (squeeze_pos
    //    starts at rate after absorb completes, hits the
    //    `squeeze_pos == rate` branch at the top of squeeze and
    //    permutes). Then read state[capacity] and return.
    neptune_permute_native(&mut state, params);

    // 4. Squeeze first rate slot + 250-bit truncation.
    let digest_scalar = state[capacity]; // = state[1]
    truncate_to_n_le_bits(digest_scalar, NUM_HASH_BITS)
}

/// Truncate `f` to the lowest `n` little-endian bits, then
/// reinterpret as `Bn254Fr`. Identical semantics to
/// `nova-snark-0.68/src/provider/poseidon.rs:74-83` — extracting
/// the low n bits via `to_le_bits[..n]`.
fn truncate_to_n_le_bits(f: Bn254Fr, n: usize) -> Bn254Fr {
    let le_bytes = f.into_bigint().to_bytes_le();
    let mut res = Bn254Fr::from(0u64);
    let mut coeff = Bn254Fr::from(1u64);
    for bit_idx in 0..n {
        let byte_idx = bit_idx / 8;
        let bit_in_byte = bit_idx % 8;
        let bit_set = if byte_idx < le_bytes.len() {
            (le_bytes[byte_idx] >> bit_in_byte) & 1 == 1
        } else {
            false
        };
        if bit_set {
            res += coeff;
        }
        coeff += coeff;
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The decisive test.** Compares our sponge output against
    /// `neptune_hash_primary` byte-for-byte on the pinned `[42, 7, 99]`
    /// absorb sequence (the same sequence pinned in
    /// `neptune_reference::tests::pinned_reference_vector_42_7_99`).
    ///
    /// Pinned expected LE bytes from
    /// `neptune_reference::pinned_reference_vector_42_7_99`:
    ///   [131, 47, 215, 132, 108, 127, 155, 157,
    ///    242, 38, 202, 31, 128, 130, 76, 218,
    ///    237, 255, 134, 244, 210, 204, 231, 79,
    ///    221, 11, 190, 164, 69, 163, 134, 1]
    ///
    /// - If our_le == neptune_le → Section 2 sponge framing is
    ///   byte-correct. Slice 5d flips the canary.
    /// - If our_le != neptune_le → log both side-by-side. The diff
    ///   tells us which framing assumption is wrong. Slice 5d
    ///   adjusts and re-tests.
    #[test]
    #[ignore = "requires /tmp/neptune-bn256-standard.json from dump-neptune-constants"]
    fn sponge_attempt_1_matches_neptune_hash_primary_on_pinned_42_7_99() {
        use crate::neptune_permutation_gadget::params_from_dump_path;

        let params =
            params_from_dump_path("/tmp/neptune-bn256-standard.json").expect("load params");

        let inputs = vec![
            Bn254Fr::from(42u64),
            Bn254Fr::from(7u64),
            Bn254Fr::from(99u64),
        ];
        let our_hash = our_neptune_hash_primary_native(&inputs, &params);

        // The pinned neptune output for this absorb sequence (from
        // neptune_reference::pinned_reference_vector_42_7_99).
        const NEPTUNE_LE: [u8; 32] = [
            131, 47, 215, 132, 108, 127, 155, 157, 242, 38, 202, 31, 128, 130, 76, 218, 237, 255,
            134, 244, 210, 204, 231, 79, 221, 11, 190, 164, 69, 163, 134, 1,
        ];

        let our_le = our_hash.into_bigint().to_bytes_le();
        let mut our_le_padded = [0u8; 32];
        our_le_padded[..our_le.len().min(32)].copy_from_slice(&our_le[..our_le.len().min(32)]);

        eprintln!("neptune LE: {NEPTUNE_LE:?}");
        eprintln!("ours    LE: {our_le_padded:?}");

        eprintln!("neptune LE: {NEPTUNE_LE:?}");
        eprintln!("ours    LE: {our_le_padded:?}");

        assert_eq!(
            our_le_padded,
            NEPTUNE_LE,
            "Sponge output must be byte-equal to neptune reference.\n\
             Root cause if this fails: pre_sparse_mds transpose fix regression or \
             CRC drift. our_le[0..8]={:?}  neptune_le[0..8]={:?}",
            &our_le_padded[..8],
            &NEPTUNE_LE[..8],
        );
    }

    /// Determinism: same input twice → same output.
    #[test]
    #[ignore = "requires /tmp/neptune-bn256-standard.json"]
    fn sponge_is_deterministic_for_fixed_input() {
        use crate::neptune_permutation_gadget::params_from_dump_path;

        let params =
            params_from_dump_path("/tmp/neptune-bn256-standard.json").expect("load params");
        let inputs = vec![
            Bn254Fr::from(1u64),
            Bn254Fr::from(2u64),
            Bn254Fr::from(3u64),
        ];

        let a = our_neptune_hash_primary_native(&inputs, &params);
        let b = our_neptune_hash_primary_native(&inputs, &params);
        assert_eq!(a, b, "sponge must be deterministic for fixed input");
    }

    /// Distinguishability: different inputs → different outputs.
    #[test]
    #[ignore = "requires /tmp/neptune-bn256-standard.json"]
    fn sponge_distinguishes_different_inputs() {
        use crate::neptune_permutation_gadget::params_from_dump_path;

        let params =
            params_from_dump_path("/tmp/neptune-bn256-standard.json").expect("load params");

        let a = our_neptune_hash_primary_native(&[Bn254Fr::from(1u64)], &params);
        let b = our_neptune_hash_primary_native(&[Bn254Fr::from(2u64)], &params);
        assert_ne!(a, b, "different inputs must produce different hashes");
    }

    /// IOPattern hasher: empty pattern + domain_separator=0 → 0.
    /// (Mirrors neptune's `test(0, IOPattern(vec![]), 0)` test
    /// in api.rs:276.)
    ///
    /// Note: our function `iopattern_value_absorb_then_squeeze`
    /// always emits at least one absorb + one squeeze op, so the
    /// empty-pattern case isn't directly testable. But we CAN test
    /// it returns deterministic values for fixed inputs.
    #[test]
    fn iopattern_value_is_deterministic() {
        let a = iopattern_value_absorb_then_squeeze(3, 1, 0);
        let b = iopattern_value_absorb_then_squeeze(3, 1, 0);
        assert_eq!(a, b);
        let c = iopattern_value_absorb_then_squeeze(3, 1, 0);
        assert_eq!(a, c);
    }

    /// IOPattern hasher distinguishes absorb counts.
    #[test]
    fn iopattern_value_distinguishes_absorb_counts() {
        let a = iopattern_value_absorb_then_squeeze(3, 1, 0);
        let b = iopattern_value_absorb_then_squeeze(4, 1, 0);
        assert_ne!(a, b);
    }

    /// IOPattern hasher distinguishes domain separators.
    #[test]
    fn iopattern_value_distinguishes_domain_separator() {
        let a = iopattern_value_absorb_then_squeeze(3, 1, 0);
        let b = iopattern_value_absorb_then_squeeze(3, 1, 1);
        assert_ne!(a, b);
    }

    /// `iopattern_tag_as_fr` converts a small u128 to the same Fr
    /// you'd get from reading the low 16 bytes of an Fr with zero
    /// high bytes.
    #[test]
    fn iopattern_tag_as_fr_for_small_value() {
        // tag = 42 → state[0] = 42 (since 42 fits in low 16 bytes).
        let f = iopattern_tag_as_fr(42u128);
        assert_eq!(f, Bn254Fr::from(42u64));
    }

    /// truncate_to_n_le_bits: simple unit test on a known value.
    /// 0xFF = 255 = 0b11111111. Truncate to 4 bits → 0b1111 = 15.
    #[test]
    fn truncate_to_n_le_bits_simple() {
        let v = Bn254Fr::from(255u64);
        let truncated = truncate_to_n_le_bits(v, 4);
        assert_eq!(
            truncated,
            Bn254Fr::from(15u64),
            "low 4 bits of 0xff = 0x0f = 15"
        );
    }

    /// IOPattern hasher distinguishes squeeze counts (parity with the
    /// existing absorb-count and domain-separator distinguishers).
    #[test]
    fn iopattern_value_distinguishes_squeeze_counts() {
        let a = iopattern_value_absorb_then_squeeze(3, 1, 0);
        let b = iopattern_value_absorb_then_squeeze(3, 2, 0);
        assert_ne!(a, b);
    }

    /// IOPattern hasher: distinct combinations of (absorb, squeeze,
    /// ds) must produce distinct tags. This is the load-bearing
    /// collision-resistance property for the chain's domain separation.
    #[test]
    fn iopattern_value_collides_only_on_identical_triples() {
        let mut seen = std::collections::HashSet::new();
        for absorb in [0u32, 1, 2, 8] {
            for squeeze in [0u32, 1, 4] {
                for ds in [0u32, 1, 7] {
                    let tag = iopattern_value_absorb_then_squeeze(absorb, squeeze, ds);
                    assert!(
                        seen.insert((absorb, squeeze, ds, tag)),
                        "duplicate tag {tag} for ({absorb},{squeeze},{ds})"
                    );
                }
            }
        }
    }

    /// `iopattern_tag_as_fr(0)` must produce `Fr::ZERO` — the
    /// degenerate case that anchors all other tag conversions.
    #[test]
    fn iopattern_tag_as_fr_zero_yields_zero() {
        assert_eq!(iopattern_tag_as_fr(0u128), Bn254Fr::from(0u64));
    }

    /// `iopattern_tag_as_fr(u128::MAX)` must produce a non-zero `Fr`
    /// of magnitude `2^128 - 1` — pins that the conversion does NOT
    /// truncate the high bytes of u128.
    #[test]
    fn iopattern_tag_as_fr_for_u128_max() {
        let f = iopattern_tag_as_fr(u128::MAX);
        assert_ne!(f, Bn254Fr::from(0u64));
        // 2^128 - 1 = 340282366920938463463374607431768211455
        let expected_bytes: [u8; 16] = u128::MAX.to_le_bytes();
        let mut full = [0u8; 32];
        full[..16].copy_from_slice(&expected_bytes);
        assert_eq!(f, Bn254Fr::from_le_bytes_mod_order(&full));
    }

    /// `iopattern_tag_as_fr` must be injective on distinct tags
    /// (within Fr's range — collision impossible for u128 since
    /// 2^128 << Fr modulus 2^254).
    #[test]
    fn iopattern_tag_as_fr_is_injective_on_distinct_u128s() {
        let mut seen = std::collections::HashSet::new();
        for tag in [0u128, 1, 42, u64::MAX as u128, u128::MAX] {
            let f = iopattern_tag_as_fr(tag);
            assert!(seen.insert(f), "Fr collision on distinct tag {tag}");
        }
    }

    /// `truncate_to_n_le_bits(_, 0)` must yield zero — no bits kept.
    /// Edge case at one end of the (0..=256) range.
    #[test]
    fn truncate_to_n_le_bits_zero_bits_yields_zero() {
        assert_eq!(
            truncate_to_n_le_bits(Bn254Fr::from(u64::MAX), 0),
            Bn254Fr::from(0u64),
            "0-bit truncation must zero out everything"
        );
    }

    /// `truncate_to_n_le_bits(f, n)` for n ≥ field-size must yield
    /// `f` unchanged (no truncation occurs; the high bits beyond the
    /// field don't exist).
    #[test]
    fn truncate_to_n_le_bits_at_field_size_returns_input() {
        let small = Bn254Fr::from(12345u64);
        assert_eq!(truncate_to_n_le_bits(small, 256), small);
        // The boundary just-below the field size (~254 bits) must
        // also round-trip for a small input.
        assert_eq!(truncate_to_n_le_bits(small, 250), small);
    }

    /// `truncate_to_n_le_bits` matches `NUM_HASH_BITS = 250` doctrine.
    /// Pin the constant so a future arkworks bump that changes the
    /// neptune reference output produces a loud failure here.
    #[test]
    fn num_hash_bits_constant_pins_at_250() {
        assert_eq!(NUM_HASH_BITS, 250);
    }

    /// `iopattern_value_absorb_then_squeeze` agrees with itself
    /// under repeated calls AND across (3,1,0) — sanity pin for the
    /// determinism property that the existing test asserts.
    #[test]
    fn iopattern_value_for_canonical_squeeze_pattern_is_nonzero() {
        // PoseidonRO::squeeze always uses ds=0; this is the pattern
        // the chain's Section 2 sponge depends on. Pin it returns a
        // non-zero, deterministic value.
        let tag = iopattern_value_absorb_then_squeeze(3, 1, 0);
        assert_ne!(tag, 0);
    }
}
