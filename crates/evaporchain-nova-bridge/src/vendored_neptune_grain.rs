//! Phase 2.2-section-2 BESPOKE debug: vendored neptune Grain
//! implementation for direct side-by-side comparison.
//!
//! Copy-pasted from `nova-snark-0.68/src/frontend/gadgets/poseidon/
//! round_constants.rs:38-168` with `pub` fields so we can run the
//! same algorithm from outside the nova-snark crate.
//!
//! Why: the `crate::grain_lfsr` impl follows the documented
//! Poseidon algorithm but produces output that diverges from
//! neptune's `crc[0]`. Possibilities:
//!   (a) Subtle Rust impl bug in `crate::grain_lfsr`
//!   (b) Misunderstanding of the algorithm
//!
//! Side-by-side: if this vendored impl AGREES with our
//! `grain_lfsr` impl → both wrong → misunderstanding.
//! If they DISAGREE → our impl has a bug.

/// Vendored from neptune `round_constants.rs`. Returns `Vec<bool>`
/// representing the 80-bit initial seed (state[0] = first pushed).
pub fn neptune_init_sequence(
    field: u8,
    sbox: u8,
    field_size: u16,
    t: u16,
    r_f: u16,
    r_p: u16,
) -> Vec<bool> {
    let mut v: Vec<bool> = Vec::new();
    append_bits(&mut v, 2, field as u128);
    append_bits(&mut v, 4, sbox as u128);
    append_bits(&mut v, 12, field_size as u128);
    append_bits(&mut v, 12, t as u128);
    append_bits(&mut v, 10, r_f as u128);
    append_bits(&mut v, 10, r_p as u128);
    append_bits(&mut v, 30, 0b111111111111111111111111111111u128);
    assert_eq!(v.len(), 80);
    v
}

fn append_bits(vec: &mut Vec<bool>, n: usize, val: u128) {
    for i in (0..n).rev() {
        vec.push((val >> i) & 1 != 0);
    }
}

/// Vendored Grain-80 LFSR state — same data structure neptune uses
/// (`Vec<bool>`, indexed 0..79).
pub struct VendoredGrain {
    state: Vec<bool>,
}

impl VendoredGrain {
    pub fn new(init_sequence: Vec<bool>) -> Self {
        assert_eq!(80, init_sequence.len());
        let mut g = VendoredGrain { state: init_sequence };
        for _ in 0..160 {
            g.generate_new_bit();
        }
        assert_eq!(80, g.state.len());
        g
    }

    fn generate_new_bit(&mut self) -> bool {
        let new_bit = self.bit(62)
            ^ self.bit(51)
            ^ self.bit(38)
            ^ self.bit(23)
            ^ self.bit(13)
            ^ self.bit(0);
        self.state.remove(0);
        self.state.push(new_bit);
        new_bit
    }

    fn bit(&self, index: usize) -> bool {
        self.state[index]
    }

    pub fn next_filtered_bit(&mut self) -> bool {
        let mut new_bit = self.generate_new_bit();
        while !new_bit {
            let _ = self.generate_new_bit();
            new_bit = self.generate_new_bit();
        }
        self.generate_new_bit()
    }

    fn next_byte(&mut self, bit_count: usize) -> u8 {
        let mut acc: u8 = 0;
        for _ in 0..bit_count {
            let bit = self.next_filtered_bit();
            acc <<= 1;
            if bit {
                acc += 1;
            }
        }
        acc
    }

    /// Vendored `get_next_bytes` from neptune for field_size=254.
    pub fn get_next_bytes_bn254(&mut self, result: &mut [u8; 32]) {
        // remainder_bits = 254 % 8 = 6
        result[0] = self.next_byte(6);
        for item in result.iter_mut().skip(1) {
            *item = self.next_byte(8);
        }
    }
}

/// Generate the first ARK Fr by running the vendored algorithm.
/// Matches neptune's `generate_constants` for one iteration.
pub fn vendored_first_ark_bn254() -> ark_bn254::Fr {
    use ark_ff::PrimeField;

    let init = neptune_init_sequence(1, 0, 254, 25, 8, 59);
    let mut g = VendoredGrain::new(init);

    loop {
        let mut repr_be = [0u8; 32];
        g.get_next_bytes_bn254(&mut repr_be);
        repr_be.reverse(); // now LE
        // Try to interpret as Fr. Bias-rejection: if value ≥ MODULUS,
        // skip and try next.
        let candidate = ark_bn254::Fr::from_le_bytes_mod_order(&repr_be);
        let canonical_le = ark_ff::BigInteger::to_bytes_le(&candidate.into_bigint());
        let mut canonical_32 = [0u8; 32];
        canonical_32[..canonical_le.len()].copy_from_slice(&canonical_le);
        if canonical_32 == repr_be {
            return candidate;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grain_lfsr::{generate_round_constants_bn254_arity_24_standard, GrainSeedParams, grain_seed_state};

    #[test]
    fn vendored_seed_matches_our_seed_bytewise() {
        let neptune_init = neptune_init_sequence(1, 0, 254, 25, 8, 59);
        let our_seed_bytes = grain_seed_state(GrainSeedParams::bn254_arity_24_standard());

        // Convert neptune's bool vec to bytes for comparison.
        let mut neptune_bytes = [0u8; 10];
        for i in 0..80 {
            if neptune_init[i] {
                let byte_idx = i / 8;
                let bit_idx = 7 - (i % 8);
                neptune_bytes[byte_idx] |= 1 << bit_idx;
            }
        }
        assert_eq!(
            neptune_bytes, our_seed_bytes,
            "vendored neptune seed must match our seed bytewise"
        );
    }

    /// THE DECISIVE TEST: run the vendored neptune algorithm to
    /// produce the first ARK Fr, compare to our `grain_lfsr` output.
    ///
    /// If they MATCH: our impl is correct; the bug is in our
    /// understanding of crc[0]'s meaning or the byte-level
    /// reading thereof.
    ///
    /// If they DIFFER: our `grain_lfsr` has a Rust-level bug
    /// despite the algorithm being correct.
    #[test]
    fn vendored_first_ark_matches_our_first_ark() {
        let vendored = vendored_first_ark_bn254();
        let ours = generate_round_constants_bn254_arity_24_standard()[0];

        use ark_ff::{BigInteger, PrimeField};
        let v_le = vendored.into_bigint().to_bytes_le();
        let o_le = ours.into_bigint().to_bytes_le();
        eprintln!("vendored LE: {v_le:?}");
        eprintln!("ours     LE: {o_le:?}");

        if vendored == ours {
            eprintln!(
                "MATCH: our LFSR impl matches the vendored algorithm. \
                 Divergence vs neptune crc[0] is in `crc` interpretation, \
                 NOT in our LFSR impl."
            );
        } else {
            eprintln!(
                "DIFFER: our LFSR impl has a Rust-level bug. The algorithm \
                 is correct but our implementation diverges."
            );
        }
        // Either way, this test PASSES — it's diagnostic.
        // We just want the printout.
    }
}
