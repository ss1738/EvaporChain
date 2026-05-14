//! Grain LFSR for Poseidon round-constant generation.
//!
//! Byte-correct port of neptune's `round_constants.rs` Grain-80
//! LFSR + the surrounding `generate_constants` driver. Produces
//! plain ARK matching neptune's `round_keys()` byte-for-byte
//! (verified at PR #97 via `lfsr_first_25_plain_round_0_parity`).
//!
//! # What ships
//!
//! - [`GrainSeedParams`] + [`grain_seed_state`] — 80-bit seed
//!   construction from `(field_type, sbox_type, field_size,
//!   sbox_count, full_rounds, partial_rounds)`.
//! - [`GrainLfsr`] — clock + warmup + filter loop + byte-packing
//!   + field-element emission with bias rejection.
//! - [`generate_round_constants_bn254_arity_24_standard`] —
//!   convenience function producing the full
//!   `(full_rounds + partial_rounds) × width = 1,675` Fr vector
//!   for our chain's Poseidon-128 parameters.
//!
//! # Seed layout (per the Poseidon paper / neptune `round_constants.rs`)
//!
//! The 80-bit LFSR initial state is:
//!
//! ```text
//!   bit  0..2     `field_type`               — 2 bits
//!   bit  2..6     `sbox_type`                — 4 bits
//!   bit  6..18    `field_size` (in bits)     — 12 bits
//!   bit 18..30    `sbox_count` (state width) — 12 bits
//!   bit 30..40    `full_rounds`              — 10 bits
//!   bit 40..50    `partial_rounds`           — 10 bits
//!   bit 50..80    `0xff...` (30 bits of 1)   — padding to 80
//! ```
//!
//! Verified against neptune's `Grain::new` constructor in
//! `nova-snark/src/frontend/gadgets/poseidon/round_constants.rs:96-112`.
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
    ///
    /// **`sbox_type = 1`** for the x^5 S-Box, per neptune's
    /// `mod.rs:46`: `const SBOX: u8 = 1; // x^5`. (Earlier PRs
    /// used 0 here, which was a real bug — caught by reading
    /// neptune's mod.rs after the vendored algorithm matched our
    /// impl but neither matched neptune's actual `crc[0]`.)
    pub const fn bn254_arity_24_standard() -> Self {
        Self {
            field_type: 1,
            sbox_type: 1,
            field_size: 254,
            sbox_count: 25,
            full_rounds: 8,
            partial_rounds: 59,
        }
    }
}

/// BN254 Fr field size in bits (254). Filtered bits are
/// accumulated into a 254-bit big-endian integer per Poseidon
/// paper conventions.
pub const BN254_FR_BITS: usize = 254;

/// In-memory Grain-80 LFSR state. Holds 80 bits as a `[u8; 10]`
/// where the **MSB of `state[0]` is bit position 0** (left-most,
/// matching the Poseidon paper convention).
///
/// Clocking shifts left by 1 — bit 0 falls out, the new feedback
/// bit enters at position 79.
#[derive(Clone, Debug)]
pub struct GrainLfsr {
    state: [u8; 10],
}

impl GrainLfsr {
    /// Initialize from a 10-byte seed (e.g. from [`grain_seed_state`]).
    pub fn from_seed(seed: [u8; 10]) -> Self {
        Self { state: seed }
    }

    /// Read the bit at logical position `i` (0 = MSB of `state[0]`,
    /// 79 = LSB of `state[9]`).
    fn bit(&self, i: usize) -> u8 {
        let byte = i / 8;
        let bit_in_byte = 7 - (i % 8);
        (self.state[byte] >> bit_in_byte) & 1
    }

    /// Compute the feedback bit per Grain-80's polynomial:
    ///   `f = state[0] ⊕ state[13] ⊕ state[23] ⊕ state[38] ⊕ state[51] ⊕ state[62]`
    fn feedback(&self) -> u8 {
        self.bit(0) ^ self.bit(13) ^ self.bit(23) ^ self.bit(38) ^ self.bit(51) ^ self.bit(62)
    }

    /// Shift the state left by 1 bit, dropping bit 0 and inserting
    /// `new_bit` at position 79. Returns the bit that just fell off
    /// (bit 0 of the pre-shift state — useful for the warmup
    /// discard).
    fn shift_in(&mut self, new_bit: u8) -> u8 {
        let dropped = self.bit(0);
        // Shift each byte left by one, carry MSB from the next byte.
        for i in 0..9 {
            self.state[i] = (self.state[i] << 1) | (self.state[i + 1] >> 7);
        }
        // Last byte: shift left + insert new_bit at LSB.
        self.state[9] = (self.state[9] << 1) | (new_bit & 1);
        dropped
    }

    /// Advance the LFSR by one step and return the new feedback
    /// bit just inserted at position 79.
    pub fn clock(&mut self) -> u8 {
        let f = self.feedback();
        self.shift_in(f);
        f
    }

    /// Run the standard 160-round warmup, discarding all output.
    /// Per the Poseidon paper, this mixes the seed into the state
    /// before any real bits are emitted.
    pub fn warmup(&mut self) {
        for _ in 0..160 {
            self.clock();
        }
    }

    /// Read the current state as raw bytes — exposed for testing
    /// the post-warmup state pin.
    pub fn state_bytes(&self) -> [u8; 10] {
        self.state
    }

    /// Emit one filtered output bit per neptune's `Iterator::next`
    /// (round_constants.rs:158-168):
    ///
    /// ```text
    /// cond = clock()
    /// while cond == 0 {
    ///     _ = clock()       // discard one bit
    ///     cond = clock()    // retry cond
    /// }
    /// output = clock()
    /// return output
    /// ```
    ///
    /// **CRITICAL: order matters.** The COND bit is clocked FIRST;
    /// the OUTPUT bit is clocked AFTER cond=1. Earlier PR (#88)
    /// had this reversed (output first, then cond) which produced
    /// a different byte stream from neptune for the same seed.
    pub fn next_filtered_bit(&mut self) -> u8 {
        let mut cond = self.clock();
        while cond == 0 {
            let _ = self.clock(); // discard one bit
            cond = self.clock(); // retry cond
        }
        self.clock()
    }

    /// Emit one `ark_bn254::Fr` field element by generating 254
    /// filtered bits (first bit = MSB of the 254-bit integer) and
    /// applying bias-rejection: re-roll if the assembled value is
    /// ≥ the BN254 Fr modulus.
    ///
    /// Order convention matches the Poseidon paper reference Sage
    /// implementation:
    ///   `int(''.join(str(b) for b in filtered_bits), 2)`
    /// where `filtered_bits[0]` is the most significant bit.
    ///
    /// **Bias rejection.** BN254 Fr modulus ≈ 2^254 × 0.756, so
    /// roughly 1 in 4 candidate uint254 values lands in the
    /// rejected interval `[MODULUS, 2^254)` and triggers a
    /// re-roll. Expected cost per emitted field element ≈
    /// `BN254_FR_BITS / (1 - 0.244) ≈ 336 filtered bits` ≈ 672
    /// raw LFSR clocks.
    pub fn next_filtered_field_element_bn254(&mut self) -> ark_bn254::Fr {
        use ark_ff::{BigInteger, PrimeField};
        loop {
            // Build the value into a 32-byte little-endian buffer.
            // Filtered bit `i` lands at integer-bit position
            // `(BN254_FR_BITS - 1 - i)` for MSB-first semantics.
            let mut buf = [0u8; 32];
            for i in 0..BN254_FR_BITS {
                let bit = self.next_filtered_bit();
                let position = BN254_FR_BITS - 1 - i; // 253, 252, ..., 0
                let byte_idx = position / 8;
                let bit_idx = position % 8;
                buf[byte_idx] |= (bit & 1) << bit_idx;
            }
            // `from_le_bytes_mod_order` will REDUCE if value ≥ MODULUS.
            // We need bias rejection — detect reduction by comparing
            // the candidate's canonical bytes to our raw input.
            let candidate = ark_bn254::Fr::from_le_bytes_mod_order(&buf);
            let canonical = candidate.into_bigint().to_bytes_le();
            let mut canonical_32 = [0u8; 32];
            canonical_32[..canonical.len()].copy_from_slice(&canonical);
            if canonical_32 == buf {
                return candidate;
            }
            // else: raw value was ≥ MODULUS, retry.
        }
    }

    /// Emit `n` filtered bits as a big-endian byte vector, MSB
    /// first in the first byte. Trailing bits (if `n` is not a
    /// multiple of 8) sit in the high bits of the last byte —
    /// matches the convention `from_be_bytes` would expect.
    pub fn next_filtered_bits_be_bytes(&mut self, n: usize) -> Vec<u8> {
        let mut out = vec![0u8; n.div_ceil(8)];
        for i in 0..n {
            let bit = self.next_filtered_bit();
            // MSB-first packing: bit i goes into byte (i/8),
            // bit position (7 - i%8) within that byte.
            let byte_idx = i / 8;
            let bit_idx = 7 - (i % 8);
            out[byte_idx] |= (bit & 1) << bit_idx;
        }
        out
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
    // Layout per neptune `Grain::new` (round_constants.rs:96-112):
    //   field_type (2) | sbox_type (4) | field_size (12) |
    //   sbox_count (12) | full_rounds (10) | partial_rounds (10) |
    //   30 ones (padding)
    push(&mut bits, &mut pos, params.field_type as u128, 2);
    push(&mut bits, &mut pos, params.sbox_type as u128, 4);
    push(&mut bits, &mut pos, params.field_size as u128, 12);
    push(&mut bits, &mut pos, params.sbox_count as u128, 12);
    push(&mut bits, &mut pos, params.full_rounds as u128, 10);
    push(&mut bits, &mut pos, params.partial_rounds as u128, 10);
    // Padding: 30 bits of 1 to reach 80 total
    let padding_bits = 80 - pos;
    let padding = (1u128 << padding_bits) - 1;
    push(&mut bits, &mut pos, padding, padding_bits);
    assert_eq!(pos, 80, "seed must be exactly 80 bits");

    // Emit as 10 big-endian bytes. The 80-bit value occupies
    // the LOW 80 bits of `bits` (since we left-shifted on each
    // push). Read the high byte first.
    let mut out = [0u8; 10];
    #[allow(clippy::needless_range_loop)]
    for i in 0..10 {
        let shift = (9 - i) * 8;
        out[i] = ((bits >> shift) & 0xff) as u8;
    }
    out
}

/// Generate the full plain-ARK round-constants vector for
/// Poseidon-128 BN254 arity-24 Strength::Standard. Returns
/// `(full_rounds + partial_rounds) × width = (8 + 59) × 25 =
/// 1,675` field elements emitted in order.
///
/// **Caller note.** This is the PLAIN ARK — one constant per
/// (round, lane). The output is what arkworks `PoseidonConfig::new`
/// expects (`ark: Vec<Vec<Fr>>` shape with `full_rounds +
/// partial_rounds` rows of `width` entries each). The Vec
/// returned here is FLAT (`Vec<Fr>` of length 1,675); reshape
/// at the call site.
///
/// Cost: ~`1,675 × 672 ≈ 1.1M raw LFSR clocks`. On Mini 1 this
/// runs in ~tens of milliseconds.
///
/// **NOT YET VERIFIED** against neptune's plain ARK. The
/// algorithm matches the Poseidon paper / hadeshash Sage
/// reference; whether neptune uses the SAME algorithm with the
/// SAME parameters needs separate byte-parity verification.
pub fn generate_round_constants_bn254_arity_24_standard() -> Vec<ark_bn254::Fr> {
    let seed = grain_seed_state(GrainSeedParams::bn254_arity_24_standard());
    let mut lfsr = GrainLfsr::from_seed(seed);
    lfsr.warmup();

    let params = GrainSeedParams::bn254_arity_24_standard();
    let rounds = (params.full_rounds + params.partial_rounds) as usize;
    let width = params.sbox_count as usize;
    let total = rounds * width;
    let mut out: Vec<ark_bn254::Fr> = Vec::with_capacity(total);
    for _ in 0..total {
        out.push(lfsr.next_filtered_field_element_bn254());
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
        assert_eq!(p.sbox_type, 1, "x^5 SBOX, per neptune mod.rs:46");
        assert_eq!(p.field_size, 254);
        assert_eq!(p.sbox_count, 25);
        assert_eq!(p.full_rounds, 8);
        assert_eq!(p.partial_rounds, 59);
    }

    /// Verify the seed has exactly 80 bits (10 bytes) and the
    /// 2-bit field-type prefix (value 1 = prime field for our
    /// params) lands at bits 0-1.
    #[test]
    fn seed_shape_and_field_type_prefix() {
        let seed = grain_seed_state(GrainSeedParams::bn254_arity_24_standard());
        assert_eq!(seed.len(), 10);
        // Top two bits of seed[0] = field_type = 1 → bit pattern 01xxxxxx.
        assert_eq!(seed[0] >> 6, 0b01, "field_type=1 at top 2 bits");
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
        // Matches neptune's `Grain::new` seed layout with the
        // CORRECT sbox_type=1 (per neptune `mod.rs:46`):
        //   bits  0..2   = 01            (field_type=1)
        //   bits  2..6   = 0001          (sbox_type=1 for x^5)
        //   bits  6..18  = 000011111110  (field_size=254)
        //   bits 18..30  = 000000011001  (sbox_count=25)
        //   bits 30..40  = 0000001000    (full_rounds=8)
        //   bits 40..50  = 0000111011    (partial_rounds=59)
        //   bits 50..80  = 30 × 1        (padding)
        //
        //   = 0x44 3F 80 64 08 0E FF FF FF FF
        let expected: [u8; 10] = [0x44, 0x3F, 0x80, 0x64, 0x08, 0x0E, 0xFF, 0xFF, 0xFF, 0xFF];
        assert_eq!(seed, expected, "seed bits for bn254/arity-24/standard");
    }

    /// Verify `bit()` reads the canonical position. With seed
    /// `0x84 03 F8 ...` the first 8 bits are `10000100`, so:
    ///   bit(0)=1, bit(1)=0, bit(2)=0, bit(3)=0,
    ///   bit(4)=0, bit(5)=1, bit(6)=0, bit(7)=0.
    #[test]
    fn lfsr_bit_indexing() {
        let lfsr = GrainLfsr::from_seed([0x84, 0x03, 0xF8, 0x06, 0x40, 0x80, 0xEF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(lfsr.bit(0), 1);
        assert_eq!(lfsr.bit(1), 0);
        assert_eq!(lfsr.bit(5), 1);
        assert_eq!(lfsr.bit(7), 0);
    }

    /// A single clock shifts the state left by 1 bit and inserts
    /// the feedback bit at position 79. After clocking, bit 0
    /// must equal the OLD bit 1.
    #[test]
    fn clock_shifts_left_by_one() {
        let mut lfsr = GrainLfsr::from_seed([0x84, 0x03, 0xF8, 0x06, 0x40, 0x80, 0xEF, 0xFF, 0xFF, 0xFF]);
        let old_bit_1 = lfsr.bit(1);
        lfsr.clock();
        assert_eq!(lfsr.bit(0), old_bit_1, "post-clock bit 0 must equal pre-clock bit 1");
    }

    /// Determinism: same seed + same number of clocks yields the
    /// same state. Catches any non-determinism leaking in.
    #[test]
    fn warmup_is_deterministic_and_changes_state() {
        let seed = grain_seed_state(GrainSeedParams::bn254_arity_24_standard());

        let mut a = GrainLfsr::from_seed(seed);
        let mut b = GrainLfsr::from_seed(seed);
        a.warmup();
        b.warmup();
        assert_eq!(a.state_bytes(), b.state_bytes(), "warmup deterministic");
        assert_ne!(a.state_bytes(), seed, "warmup must change the state");
        assert_ne!(a.state_bytes(), [0u8; 10], "warmup must not zero the state");
    }

    /// Filter loop emits BOTH 0 and 1 bits — confirms the
    /// discard-zeros logic isn't accidentally producing a
    /// degenerate stream. Running 200 filtered bits should give
    /// a non-trivial mix.
    #[test]
    fn filter_loop_emits_mixed_bits() {
        let seed = grain_seed_state(GrainSeedParams::bn254_arity_24_standard());
        let mut lfsr = GrainLfsr::from_seed(seed);
        lfsr.warmup();
        let mut zeros = 0;
        let mut ones = 0;
        for _ in 0..200 {
            match lfsr.next_filtered_bit() {
                0 => zeros += 1,
                1 => ones += 1,
                b => panic!("non-binary bit: {b}"),
            }
        }
        // For a healthy CSPRNG, expect ~100 zeros and ~100 ones.
        // Conservative bound: at least 30 of each (catches the
        // pathological "always 0" or "always 1" regression).
        assert!(zeros >= 30, "filter produced too few zeros: {zeros}");
        assert!(ones >= 30, "filter produced too few ones: {ones}");
        eprintln!("filter sample 200 bits: zeros={zeros} ones={ones}");
    }

    /// Filter is deterministic: same seed yields the same
    /// filtered stream.
    #[test]
    fn filter_is_deterministic() {
        let seed = grain_seed_state(GrainSeedParams::bn254_arity_24_standard());
        let mut a = GrainLfsr::from_seed(seed);
        let mut b = GrainLfsr::from_seed(seed);
        a.warmup();
        b.warmup();
        let a_bits: Vec<u8> = (0..64).map(|_| a.next_filtered_bit()).collect();
        let b_bits: Vec<u8> = (0..64).map(|_| b.next_filtered_bit()).collect();
        assert_eq!(a_bits, b_bits, "filter must be deterministic");
    }

    /// `next_filtered_bits_be_bytes` packs MSB-first. For an
    /// 8-bit fixture where the first filtered bit happens to be
    /// `b`, byte[0] must have `b` at bit position 7 (MSB).
    #[test]
    fn filtered_bits_pack_msb_first() {
        let seed = grain_seed_state(GrainSeedParams::bn254_arity_24_standard());
        let mut lfsr = GrainLfsr::from_seed(seed);
        lfsr.warmup();
        let mut probe = lfsr.clone();
        let first_bit = probe.next_filtered_bit();
        let bytes = lfsr.next_filtered_bits_be_bytes(8);
        assert_eq!(bytes.len(), 1);
        assert_eq!(
            (bytes[0] >> 7) & 1,
            first_bit,
            "MSB of byte 0 must equal first filtered bit"
        );
    }

    /// `next_filtered_bits_be_bytes(254)` produces 32 bytes (the
    /// shape we need for BN254 Fr packing). Last byte's low 2 bits
    /// are zero-padded (since 254 isn't a multiple of 8 — but the
    /// 254 emitted bits land in bit positions 7..down through 0
    /// across 32 bytes; the LAST 2 bits of byte 31 remain zero).
    #[test]
    fn filtered_bits_254_produces_32_bytes() {
        let seed = grain_seed_state(GrainSeedParams::bn254_arity_24_standard());
        let mut lfsr = GrainLfsr::from_seed(seed);
        lfsr.warmup();
        let bytes = lfsr.next_filtered_bits_be_bytes(254);
        assert_eq!(bytes.len(), 32, "254 bits → 32 bytes (with 2 trailing pad bits)");
        // Low 2 bits of byte 31 must be zero (positions 254 and 255
        // not filled).
        assert_eq!(bytes[31] & 0b11, 0, "trailing pad bits must be zero");
    }

    /// Field-element emission is deterministic across runs.
    #[test]
    fn field_element_emission_is_deterministic() {
        let seed = grain_seed_state(GrainSeedParams::bn254_arity_24_standard());
        let mut a = GrainLfsr::from_seed(seed);
        let mut b = GrainLfsr::from_seed(seed);
        a.warmup();
        b.warmup();
        let a_fes: Vec<_> = (0..5).map(|_| a.next_filtered_field_element_bn254()).collect();
        let b_fes: Vec<_> = (0..5).map(|_| b.next_filtered_field_element_bn254()).collect();
        assert_eq!(a_fes, b_fes, "field-element emission must be deterministic");
    }

    /// Distinct emitted scalars must differ — a degenerate output
    /// (always-zero, always-same) means the LFSR is stuck. Generate
    /// 5 elements and assert they're all distinct.
    #[test]
    fn emitted_field_elements_are_distinct() {
        let seed = grain_seed_state(GrainSeedParams::bn254_arity_24_standard());
        let mut lfsr = GrainLfsr::from_seed(seed);
        lfsr.warmup();
        let fes: Vec<_> = (0..5).map(|_| lfsr.next_filtered_field_element_bn254()).collect();
        for i in 0..fes.len() {
            for j in (i + 1)..fes.len() {
                assert_ne!(fes[i], fes[j], "elements {i} and {j} collided");
            }
            assert_ne!(fes[i], ark_bn254::Fr::from(0u64), "element {i} is zero");
        }
    }

    /// Pin the FIRST emitted field element for the BN254/arity-24
    /// seed. After PR #97's SBOX-type fix achieved byte-parity
    /// with neptune's plain ARK, these bytes are KNOWN-EQUAL to
    /// neptune's `crc[0]` (which IS plain `round_keys(0)[0]` per
    /// `preprocessing.rs:33`'s `res.extend(round_keys(0))`).
    ///
    /// The 32-byte LE representation is captured from a clean
    /// Mini-1 run. If this test ever fires, the LFSR has drifted
    /// from neptune's output and the downstream port
    /// (PRs #102, #103) is invalidated.
    #[test]
    fn pinned_first_field_element_for_bn254() {
        use ark_ff::{BigInteger, PrimeField};
        let seed = grain_seed_state(GrainSeedParams::bn254_arity_24_standard());
        let mut lfsr = GrainLfsr::from_seed(seed);
        lfsr.warmup();
        let fe = lfsr.next_filtered_field_element_bn254();
        let le = fe.into_bigint().to_bytes_le();
        // Pinned LE bytes = neptune crc[0] LE form (verified
        // byte-for-byte against PR #80's JSON dump):
        const EXPECTED_LE: [u8; 32] = [
            128, 67, 230, 115, 239, 141, 250, 143, 246, 136, 232, 130, 13, 3, 223, 254,
            112, 206, 1, 48, 121, 188, 29, 28, 9, 241, 131, 55, 224, 40, 65, 3,
        ];
        let mut le_32 = [0u8; 32];
        let copy_len = le.len().min(32);
        le_32[..copy_len].copy_from_slice(&le[..copy_len]);
        assert_eq!(
            le_32, EXPECTED_LE,
            "LFSR's first emitted Fr drifted from pinned neptune crc[0]"
        );
    }

    /// The full ARK vector has the expected length for our
    /// parameters: `(8 + 59) × 25 = 1,675` entries.
    #[test]
    fn full_ark_has_expected_length() {
        let ark = generate_round_constants_bn254_arity_24_standard();
        assert_eq!(ark.len(), 1675, "(8 + 59) × 25");
    }

    /// First entry of the full ARK matches the pinned-first-Fr
    /// canary from PR #89 — confirms the full-generation routine
    /// uses the same warmup + emission path as the single-element
    /// test.
    #[test]
    fn full_ark_first_entry_matches_pinned_single() {
        let ark = generate_round_constants_bn254_arity_24_standard();

        let seed = grain_seed_state(GrainSeedParams::bn254_arity_24_standard());
        let mut lfsr = GrainLfsr::from_seed(seed);
        lfsr.warmup();
        let expected_first = lfsr.next_filtered_field_element_bn254();

        assert_eq!(ark[0], expected_first);
    }

    /// All 1,675 entries are distinct (no degenerate collisions).
    /// The bias-rejected uniform sampler should give effectively
    /// zero collision probability at this scale.
    #[test]
    fn full_ark_entries_are_unique() {
        let ark = generate_round_constants_bn254_arity_24_standard();
        let unique: std::collections::HashSet<_> = ark.iter().collect();
        assert_eq!(
            unique.len(),
            ark.len(),
            "duplicate ARK entries detected — sampler is broken"
        );
    }

    /// **Direct LFSR parity vs neptune.** Per PR #84's layout
    /// analysis, neptune's `crc[0..200]` IS the plain ARK for the
    /// 8 full rounds (full-round constants stored uncompressed;
    /// only partial rounds get the SBOX-trick optimization).
    ///
    /// Compares the first 200 entries of our generated ARK to
    /// neptune's `crc[0..200]` from the JSON dump (PR #80).
    /// If MATCH: LFSR port is byte-correct; remaining parity
    /// divergence is downstream of the LFSR (sponge framing,
    /// domain tag, etc).
    /// If MISMATCH: the LFSR still has a bug.
    ///
    /// `#[ignore]` — requires the JSON dump on disk.
    #[test]
    #[ignore = "requires /tmp/neptune-bn256-standard.json from dump-neptune-constants"]
    fn lfsr_first_200_entries_match_neptune_crc() {
        use crate::neptune_dump_parser::extract_compressed_round_constants;

        let ours = generate_round_constants_bn254_arity_24_standard();
        let theirs = extract_compressed_round_constants("/tmp/neptune-bn256-standard.json")
            .expect("load dump");

        let mut mismatches = 0usize;
        let mut first_mismatch: Option<usize> = None;
        for i in 0..200 {
            if ours[i] != theirs[i] {
                if first_mismatch.is_none() {
                    first_mismatch = Some(i);
                }
                mismatches += 1;
            }
        }
        eprintln!(
            "LFSR direct parity: {} of 200 full-round constants differ; first mismatch index = {:?}",
            mismatches, first_mismatch
        );
        if mismatches == 0 {
            eprintln!("LFSR PORT BYTE-CORRECT — divergence is downstream.");
        }
        // Document the current state via assert_ne: if/when this
        // fires, the LFSR is byte-correct.
        assert_ne!(mismatches, 0,
            "LFSR matches neptune ARK byte-for-byte — flip this to `assert_eq!(mismatches, 0)`.");
    }

    /// Tightened LFSR parity check: compare only `ours[0..25]`
    /// vs `theirs[0..25]`.
    ///
    /// Per neptune's `preprocessing.rs:33-34` (`res.extend(round_keys(0))`),
    /// `crc[0..width]` = plain ARK for round 0. So this 25-entry
    /// window is the ONLY clean comparison we get (rounds 1+ get
    /// matrix-multiplied transformations in the compressed form).
    ///
    /// If even index 0 differs: the LFSR itself has a bug (the
    /// very first emitted field element is wrong).
    /// If only index >=1 differ: subtle ordering / packing bug.
    /// If all match: LFSR is correct; remaining sponge-level
    /// divergence is downstream.
    #[test]
    #[ignore = "requires /tmp/neptune-bn256-standard.json"]
    fn lfsr_first_25_plain_round_0_parity() {
        use crate::neptune_dump_parser::extract_compressed_round_constants;
        use ark_ff::{BigInteger, PrimeField};

        let ours = generate_round_constants_bn254_arity_24_standard();
        let theirs = extract_compressed_round_constants("/tmp/neptune-bn256-standard.json")
            .expect("load dump");

        let mut mismatches: Vec<usize> = Vec::new();
        for i in 0..25 {
            if ours[i] != theirs[i] {
                mismatches.push(i);
            }
        }
        eprintln!("First 25 plain-round-0 entries: {} mismatches", mismatches.len());
        if !mismatches.is_empty() {
            eprintln!("  Mismatch indices: {mismatches:?}");
            // Show byte diff for index 0
            let ours_0_le = ours[0].into_bigint().to_bytes_le();
            let theirs_0_le = theirs[0].into_bigint().to_bytes_le();
            eprintln!("  ours[0]   LE: {ours_0_le:?}");
            eprintln!("  theirs[0] LE: {theirs_0_le:?}");
            // Common prefix length
            let mut common = 0;
            while common < ours_0_le.len()
                && common < theirs_0_le.len()
                && ours_0_le[common] == theirs_0_le[common]
            {
                common += 1;
            }
            eprintln!("  common LE-prefix bytes: {common} / 32");
        }
        // PORT COMPLETE (post PR #97 sbox_type=1 fix): LFSR
        // output matches neptune crc[0..25] byte-for-byte.
        assert!(
            mismatches.is_empty(),
            "LFSR plain ARK round 0 must match neptune crc[0..25] byte-for-byte"
        );
    }

    /// Generating twice yields the same vector — pure
    /// determinism check on the full pipeline.
    #[test]
    fn full_ark_is_deterministic() {
        let a = generate_round_constants_bn254_arity_24_standard();
        let b = generate_round_constants_bn254_arity_24_standard();
        assert_eq!(a, b);
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
