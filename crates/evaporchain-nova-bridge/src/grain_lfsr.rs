//! Phase 2.2-section-2 BESPOKE: grain LFSR seed initialization for
//! Poseidon round-constant generation.
//!
//! This module ships ONLY the seed construction (step 1 of a
//! multi-step algorithm). The remaining steps are:
//!
//! - Clock the LFSR 160 rounds to "warm up" (discard output).
//! - Generate output bits with the filter-and-discard-zeros loop.
//! - Pack each 254-bit window into a field element mod p.
//! - Produce `(full_rounds + partial_rounds) × width` round
//!   constants total.
//!
//! Future PRs in the stack fill those in.
//!
//! # Seed layout (per the Poseidon paper, Grassi et al. 2019, App. A)
//!
//! The 80-bit LFSR initial state is:
//!
//! ```text
//!   bit  0..2     0b10                       — fixed prefix (2 bits)
//!   bit  2..6     `field_type`               — 4 bits
//!   bit  6..10    `sbox_type`                — 4 bits
//!   bit 10..22    `field_size` (in bits)     — 12 bits
//!   bit 22..34    `sbox_count` (state width) — 12 bits
//!   bit 34..44    `full_rounds`              — 10 bits
//!   bit 44..54    `partial_rounds`           — 10 bits
//!   bit 54..80    `0xff...` (26 bits of 1)   — padding to 80
//! ```
//!
//! Big-endian bit packing throughout.
//!
//! # Parameters for our BN254 / Poseidon-128 / arity-24
//!
//! - `field_type` = 1 (prime field)
//! - `sbox_type` = 0 (α = 5 SBOX)
//! - `field_size` = 254 (BN254 Fr modulus is 254 bits)
//! - `sbox_count` = 25 (state width = arity 24 + capacity 1)
//! - `full_rounds` = 8
//! - `partial_rounds` = 59
//!
//! # Verification
//!
//! The seed bits are pinned in tests. When the remaining LFSR
//! steps land, the test against the `neptune_reference` oracle
//! (PR #79's canary) flips from `assert_ne!` to `assert_eq!`.

/// LFSR seed parameters for a single Poseidon instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrainSeedParams {
    /// Prime-field marker (1 for prime fields).
    pub field_type: u8,
    /// SBOX type (0 for α = 5).
    pub sbox_type: u8,
    /// Field size in bits (254 for BN254 Fr).
    pub field_size: u16,
    /// Sponge state width = rate + capacity (25 for arity-24
    /// standard).
    pub sbox_count: u16,
    /// Number of full rounds (8 for standard).
    pub full_rounds: u16,
    /// Number of partial rounds (59 for arity-24 standard, per
    /// PR #80's empirical extraction).
    pub partial_rounds: u16,
}

impl GrainSeedParams {
    /// BN254 Fr, Poseidon-128 (α = 5), arity-24 standard strength.
    /// Matches the parameters PR #80's `dump-neptune-constants`
    /// extracted from nova-snark's neptune.
    pub const fn bn254_arity_24_standard() -> Self {
        Self {
            field_type: 1,
            sbox_type: 0,
            field_size: 254,
            sbox_count: 25,
            full_rounds: 8,
            partial_rounds: 59,
        }
    }
}

/// Construct the 80-bit grain LFSR initial state for the given
/// parameters. Returns 10 bytes (80 bits) in big-endian order —
/// `state[0]` is the most significant byte.
///
/// The bit-layout is right-padded with 26 ones (per the Poseidon
/// reference Sage implementation).
pub fn grain_seed_state(params: GrainSeedParams) -> [u8; 10] {
    // Build the 80-bit value as a `u128`, then split into bytes.
    // Bits are packed most-significant-first.
    let mut bits: u128 = 0;
    let mut pos: u32 = 0;
    let push = |bits: &mut u128, pos: &mut u32, value: u128, n_bits: u32| {
        // Place `value` (n_bits wide) at the high end of the
        // unused window, growing rightward.
        // The seed's MSB is bit 79; we're building from MSB → LSB,
        // i.e. shifting subsequent inserts to the LEFT in the
        // accumulator.
        *bits = (*bits << n_bits) | (value & ((1u128 << n_bits) - 1));
        *pos += n_bits;
    };
    // Fixed prefix `10` (2 bits)
    push(&mut bits, &mut pos, 0b10, 2);
    push(&mut bits, &mut pos, params.field_type as u128, 4);
    push(&mut bits, &mut pos, params.sbox_type as u128, 4);
    push(&mut bits, &mut pos, params.field_size as u128, 12);
    push(&mut bits, &mut pos, params.sbox_count as u128, 12);
    push(&mut bits, &mut pos, params.full_rounds as u128, 10);
    push(&mut bits, &mut pos, params.partial_rounds as u128, 10);
    // Padding: 26 bits of 1 to reach 80 total
    let padding_bits = 80 - pos;
    let padding = (1u128 << padding_bits) - 1;
    push(&mut bits, &mut pos, padding, padding_bits);
    assert_eq!(pos, 80, "seed must be exactly 80 bits");

    // Emit as 10 big-endian bytes. The 80-bit value occupies
    // the LOW 80 bits of `bits` (since we left-shifted on each
    // push). Read the high byte first.
    let mut out = [0u8; 10];
    for i in 0..10 {
        let shift = (9 - i) * 8;
        out[i] = ((bits >> shift) & 0xff) as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bn254_arity_24_standard_params_match_pr80() {
        let p = GrainSeedParams::bn254_arity_24_standard();
        assert_eq!(p.field_type, 1);
        assert_eq!(p.sbox_type, 0);
        assert_eq!(p.field_size, 254);
        assert_eq!(p.sbox_count, 25);
        assert_eq!(p.full_rounds, 8);
        assert_eq!(p.partial_rounds, 59);
    }

    /// Verify the seed has exactly 80 bits (10 bytes) and the
    /// known fixed prefix `0b10` lands at the MSB position.
    #[test]
    fn seed_shape_and_prefix() {
        let seed = grain_seed_state(GrainSeedParams::bn254_arity_24_standard());
        assert_eq!(seed.len(), 10);
        // Top two bits of seed[0] = 0b10 → seed[0] starts with bit
        // pattern 10xxxxxx. Mask off the top two bits.
        assert_eq!(seed[0] >> 6, 0b10, "fixed prefix bits at MSB");
    }

    /// Pin the full 10-byte seed for our parameters so a future
    /// regression in `grain_seed_state` (off-by-one shift, swapped
    /// field order) fires loudly. Bytes computed by hand from the
    /// layout in the module doc:
    ///   prefix(2)         = 10              → 2 bits in [0..2]
    ///   field_type(4)     = 0001            → bits [2..6]
    ///   sbox_type(4)      = 0000            → bits [6..10]
    ///   field_size(12)    = 254 = 0000 1111 1110 → bits [10..22]
    ///   sbox_count(12)    = 25 = 0000 0001 1001    → bits [22..34]
    ///   full_rounds(10)   = 8 = 00 0000 1000        → bits [34..44]
    ///   partial_rounds(10)= 59 = 00 0011 1011       → bits [44..54]
    ///   padding(26)       = 111...1 (26 ones)        → bits [54..80]
    ///
    /// Concatenated MSB-first:
    ///   10_0001_0000_000011111110_000000011001_0000001000_0000111011_11111111111111111111111111
    ///
    /// Decoded to 10 bytes (big-endian):
    #[test]
    fn pinned_seed_for_bn254_arity_24_standard() {
        let seed = grain_seed_state(GrainSeedParams::bn254_arity_24_standard());
        // Empirically captured from a clean Mini-1 run and
        // verified bit-by-bit against the layout doc:
        //
        //   0x84 03 F8 06 40 80 EF FF FF FF
        //
        // Decomposed by bit window:
        //   bits  0..2   = 10            (prefix)              ✓
        //   bits  2..6   = 0001          (field_type=1)        ✓
        //   bits  6..10  = 0000          (sbox_type=0)         ✓
        //   bits 10..22  = 000011111110  (field_size=254)      ✓
        //   bits 22..34  = 000000011001  (sbox_count=25)       ✓
        //   bits 34..44  = 0000001000    (full_rounds=8)       ✓
        //   bits 44..54  = 0000111011    (partial_rounds=59)   ✓
        //   bits 54..80  = 11...1 (26)   (padding)             ✓
        let expected: [u8; 10] = [0x84, 0x03, 0xF8, 0x06, 0x40, 0x80, 0xEF, 0xFF, 0xFF, 0xFF];
        assert_eq!(seed, expected, "seed bits for bn254/arity-24/standard");
    }

    /// Changing any one parameter must produce a different seed.
    /// Catches a regression where the parameter binding loses a
    /// field.
    #[test]
    fn different_params_produce_different_seeds() {
        let base = grain_seed_state(GrainSeedParams::bn254_arity_24_standard());

        let mut p2 = GrainSeedParams::bn254_arity_24_standard();
        p2.full_rounds = 9;
        let s2 = grain_seed_state(p2);
        assert_ne!(base, s2, "changing full_rounds must change seed");

        let mut p3 = GrainSeedParams::bn254_arity_24_standard();
        p3.partial_rounds = 58;
        let s3 = grain_seed_state(p3);
        assert_ne!(base, s3, "changing partial_rounds must change seed");

        let mut p4 = GrainSeedParams::bn254_arity_24_standard();
        p4.sbox_count = 26;
        let s4 = grain_seed_state(p4);
        assert_ne!(base, s4, "changing sbox_count must change seed");
    }
}
