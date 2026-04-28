//! `PAdicKey<P>` — a `u64` interpreted as a p-adic integer, with a
//! base-`P` digit decomposition that the Merkle tree uses to walk paths.
//!
//! Design choice: low-order digits first. The depth-`d` ultrametric ball
//! containing `key` is determined by `key mod P^d` — i.e., the first `d`
//! low-order base-`P` digits. This matches the convention used in
//! `valuation::v_p(n)` (which also reads from the low order) and lets a
//! key's valuation be read directly as the depth at which it first
//! becomes "alone" in its ball.

use serde::{Deserialize, Serialize};

use crate::valuation::const_assert_p_ge_2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PAdicKey<const P: usize>(pub u64);

impl<const P: usize> PAdicKey<P> {
    pub const fn new(raw: u64) -> Self {
        const_assert_p_ge_2::<P>();
        Self(raw)
    }

    /// Raw `u64`.
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Base-`P` digit at position `i` (0 = lowest order).
    /// For `i` past the highest non-zero digit returns 0.
    pub fn digit_at(self, i: u32) -> u8 {
        let p = P as u64;
        let mut x = self.0;
        for _ in 0..i {
            x /= p;
            if x == 0 {
                return 0;
            }
        }
        (x % p) as u8
    }

    /// First `depth` base-`P` digits (low order first). `digits(d)[i]` =
    /// `self.digit_at(i)`.
    pub fn digits(self, depth: u32) -> Vec<u8> {
        (0..depth).map(|i| self.digit_at(i)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p2_digits_are_bits_low_first() {
        // 0b1011 = 11
        let k = PAdicKey::<2>::new(0b1011);
        assert_eq!(k.digits(4), vec![1, 1, 0, 1]);
    }

    #[test]
    fn p3_digits() {
        // 14 in base 3 = 112 (= 1*9 + 1*3 + 2*1) → low-order digits: 2, 1, 1
        let k = PAdicKey::<3>::new(14);
        assert_eq!(k.digits(3), vec![2, 1, 1]);
    }

    #[test]
    fn p5_digits() {
        // 130 in base 5 = 1010 (= 1*125 + 0*25 + 1*5 + 0*1)
        // low-order digits: 0, 1, 0, 1
        let k = PAdicKey::<5>::new(130);
        assert_eq!(k.digits(4), vec![0, 1, 0, 1]);
    }

    #[test]
    fn digits_past_high_order_are_zero() {
        let k = PAdicKey::<2>::new(0b101);
        assert_eq!(k.digits(8), vec![1, 0, 1, 0, 0, 0, 0, 0]);
    }
}
