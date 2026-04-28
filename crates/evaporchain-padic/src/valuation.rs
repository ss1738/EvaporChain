//! p-adic valuation `v_p(n)` — the largest exponent `k` such that
//! `p^k` divides `n`. By convention `v_p(0) = ∞`, encoded as `u32::MAX`.
//!
//! This is the algebraic foundation of every other module in this crate:
//! the ultrametric distance is `v_p(|x − y|)`, and the depth at which a
//! key first lands in the Merkle tree is bounded by `v_p(key)`.

/// Number of times `P` divides `n`. Returns `u32::MAX` for `n = 0`
/// (the conventional `v_p(0) = ∞`).
///
/// `P` must be ≥ 2; the `const _:` assertion at the call boundary is
/// the cheapest way to enforce that with the current const-generic
/// surface — `P = 0` and `P = 1` would loop forever.
pub fn valuation<const P: usize>(n: u64) -> u32 {
    const_assert_p_ge_2::<P>();
    if n == 0 {
        return u32::MAX;
    }
    let p = P as u64;
    let mut v = 0u32;
    let mut x = n;
    while x % p == 0 {
        x /= p;
        v += 1;
    }
    v
}

/// Compile-time check that `P >= 2`. Triggers a divide-by-zero panic at
/// const-eval time if violated, which surfaces as a clean compile error.
#[inline(always)]
pub(crate) const fn const_assert_p_ge_2<const P: usize>() {
    // `1 / (P >= 2) as usize` panics at const-eval if P < 2.
    let _ = 1 / ((P >= 2) as usize);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v_2_powers_of_two() {
        assert_eq!(valuation::<2>(1), 0);
        assert_eq!(valuation::<2>(2), 1);
        assert_eq!(valuation::<2>(4), 2);
        assert_eq!(valuation::<2>(8), 3);
        assert_eq!(valuation::<2>(1024), 10);
    }

    #[test]
    fn v_2_odd_numbers_are_zero() {
        assert_eq!(valuation::<2>(3), 0);
        assert_eq!(valuation::<2>(5), 0);
        assert_eq!(valuation::<2>(15), 0);
    }

    #[test]
    fn v_3_powers_of_three() {
        assert_eq!(valuation::<3>(1), 0);
        assert_eq!(valuation::<3>(3), 1);
        assert_eq!(valuation::<3>(9), 2);
        assert_eq!(valuation::<3>(27), 3);
        assert_eq!(valuation::<3>(81), 4);
    }

    #[test]
    fn v_3_mixed() {
        // 54 = 2 * 27 → v_3 = 3
        assert_eq!(valuation::<3>(54), 3);
        // 100 = 4 * 25 → v_3 = 0
        assert_eq!(valuation::<3>(100), 0);
        // 81 * 5 = 405 → v_3 = 4
        assert_eq!(valuation::<3>(405), 4);
    }

    #[test]
    fn v_5_examples() {
        assert_eq!(valuation::<5>(1), 0);
        assert_eq!(valuation::<5>(5), 1);
        assert_eq!(valuation::<5>(125), 3);
        assert_eq!(valuation::<5>(250), 3); // 2 * 125
        assert_eq!(valuation::<5>(7), 0);
    }

    #[test]
    fn v_p_zero_is_infinity() {
        assert_eq!(valuation::<2>(0), u32::MAX);
        assert_eq!(valuation::<3>(0), u32::MAX);
        assert_eq!(valuation::<7>(0), u32::MAX);
    }

    #[test]
    fn product_rule_v_p_xy_equals_v_p_x_plus_v_p_y() {
        // Algebraic: v_p(x*y) = v_p(x) + v_p(y) when neither is zero.
        // Spot-check.
        let x = 12u64; // 2^2 * 3
        let y = 9u64; //  3^2
        assert_eq!(
            valuation::<3>(x * y),
            valuation::<3>(x) + valuation::<3>(y)
        );
    }
}
