//! Mersenne-31 field arithmetic + multiplicative inverse via
//! Fermat's little theorem.

pub type FieldElem = u64;

/// Mersenne-31 prime: 2³¹ − 1.
pub const MOD_P: u64 = 2_147_483_647;

pub fn add_p(a: FieldElem, b: FieldElem) -> FieldElem {
    ((a as u128 + b as u128) % (MOD_P as u128)) as u64
}

pub fn sub_p(a: FieldElem, b: FieldElem) -> FieldElem {
    let am = a % MOD_P;
    let bm = b % MOD_P;
    if am >= bm {
        am - bm
    } else {
        MOD_P - (bm - am)
    }
}

pub fn mul_p(a: FieldElem, b: FieldElem) -> FieldElem {
    ((a as u128 * b as u128) % (MOD_P as u128)) as u64
}

pub fn neg_p(a: FieldElem) -> FieldElem {
    sub_p(0, a)
}

/// Modular exponentiation a^e mod P (binary method).
pub fn pow_p(mut a: FieldElem, mut e: u64) -> FieldElem {
    let mut r: FieldElem = 1;
    a %= MOD_P;
    while e > 0 {
        if e & 1 == 1 {
            r = mul_p(r, a);
        }
        a = mul_p(a, a);
        e >>= 1;
    }
    r
}

/// Multiplicative inverse via Fermat: a^(P-2) mod P.
/// Returns None if a == 0.
pub fn inverse_p(a: FieldElem) -> Option<FieldElem> {
    if a % MOD_P == 0 {
        None
    } else {
        Some(pow_p(a, MOD_P - 2))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_wraps() {
        assert_eq!(add_p(MOD_P - 1, 1), 0);
    }

    #[test]
    fn sub_wraps() {
        assert_eq!(sub_p(1, 2), MOD_P - 1);
    }

    #[test]
    fn mul_wraps() {
        let a = MOD_P - 1; // -1 mod P
        let b = MOD_P - 1;
        assert_eq!(mul_p(a, b), 1);
    }

    #[test]
    fn neg_is_additive_inverse() {
        let a = 12345u64;
        assert_eq!(add_p(a, neg_p(a)), 0);
    }

    #[test]
    fn pow_at_zero_exponent_is_one() {
        assert_eq!(pow_p(7, 0), 1);
    }

    #[test]
    fn pow_at_one_exponent_is_a() {
        assert_eq!(pow_p(7, 1), 7);
    }

    #[test]
    fn inverse_of_two_is_correct() {
        let inv2 = inverse_p(2).unwrap();
        assert_eq!(mul_p(2, inv2), 1);
    }

    #[test]
    fn inverse_of_zero_is_none() {
        assert!(inverse_p(0).is_none());
    }

    #[test]
    fn inverse_property_holds_for_arbitrary_nonzero() {
        for a in [1u64, 2, 3, 7, 12345, MOD_P - 1] {
            let inv = inverse_p(a).unwrap();
            assert_eq!(mul_p(a, inv), 1, "a={a}, inv={inv}");
        }
    }
}
