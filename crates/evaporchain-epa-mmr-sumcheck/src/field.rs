//! Mersenne-31 field arithmetic + multilinear-extension evaluation.

pub type FieldElem = u64;
pub const MOD_P: u64 = 2_147_483_647; // 2³¹ − 1

pub fn add_p(a: FieldElem, b: FieldElem) -> FieldElem {
    ((a as u128 + b as u128) % MOD_P as u128) as u64
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
    ((a as u128 * b as u128) % MOD_P as u128) as u64
}

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

pub fn inverse_p(a: FieldElem) -> Option<FieldElem> {
    if a % MOD_P == 0 {
        None
    } else {
        Some(pow_p(a, MOD_P - 2))
    }
}

/// Evaluate the multilinear extension of `values` (length 2^v)
/// at the point `point` (length v). The MLE is the unique
/// multilinear polynomial that interpolates `values` at the
/// hypercube `{0, 1}^v`. Pure integer over MOD_P.
///
/// Algorithm: for each variable i, "fold in" the i-th coordinate
/// via the standard MLE recurrence
///   table[j] := (1 - p_i) · table[j] + p_i · table[j + 2^i]
/// halving the table length each round.
pub fn multilinear_extend(values: &[FieldElem], point: &[FieldElem]) -> Option<FieldElem> {
    let n = values.len();
    if n == 0 {
        return None;
    }
    if !n.is_power_of_two() {
        return None;
    }
    let v = n.trailing_zeros() as usize;
    if point.len() != v {
        return None;
    }
    let mut table: Vec<FieldElem> = values.iter().map(|x| x % MOD_P).collect();
    let mut len = n;
    for &p in point {
        let half = len / 2;
        // table[j] := (1 − p) · table[j] + p · table[j + half]
        let one_minus_p = sub_p(1, p);
        for j in 0..half {
            let lo = table[j];
            let hi = table[j + half];
            table[j] = add_p(mul_p(one_minus_p, lo), mul_p(p, hi));
        }
        len = half;
    }
    Some(table[0])
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
        assert_eq!(mul_p(MOD_P - 1, MOD_P - 1), 1); // (-1)·(-1)=1 mod P
    }

    #[test]
    fn inverse_property() {
        let inv2 = inverse_p(2).unwrap();
        assert_eq!(mul_p(2, inv2), 1);
        assert!(inverse_p(0).is_none());
    }

    // ── multilinear extension ─────────────────────────────────────

    #[test]
    fn mle_rejects_non_power_of_two() {
        let values = vec![1u64, 2, 3];
        assert!(multilinear_extend(&values, &[0]).is_none());
    }

    #[test]
    fn mle_rejects_wrong_point_length() {
        let values = vec![1u64, 2, 3, 4];
        // 4-element vector → v=2 dims → point must be length 2.
        assert!(multilinear_extend(&values, &[0]).is_none());
        assert!(multilinear_extend(&values, &[0, 0, 0]).is_none());
    }

    #[test]
    fn mle_at_hypercube_corner_returns_corresponding_value() {
        // MSB-first convention: index = 2·b₀ + b₁ where point = (b₀, b₁).
        // values[0]=10 ↔ (0,0); values[1]=20 ↔ (0,1);
        // values[2]=30 ↔ (1,0); values[3]=40 ↔ (1,1).
        let v = vec![10u64, 20, 30, 40];
        assert_eq!(multilinear_extend(&v, &[0, 0]).unwrap(), 10);
        assert_eq!(multilinear_extend(&v, &[0, 1]).unwrap(), 20);
        assert_eq!(multilinear_extend(&v, &[1, 0]).unwrap(), 30);
        assert_eq!(multilinear_extend(&v, &[1, 1]).unwrap(), 40);
    }

    #[test]
    fn mle_at_one_dim_corner() {
        let v = vec![7u64, 11];
        assert_eq!(multilinear_extend(&v, &[0]).unwrap(), 7);
        assert_eq!(multilinear_extend(&v, &[1]).unwrap(), 11);
    }

    #[test]
    fn mle_at_one_dim_midpoint_is_average() {
        // f(0)=7, f(1)=11. f(0.5) = 9 in real arithmetic.
        // In micros: point = (P+1)/2 (= 1/2 mod P), result = (7+11) · inv(2)
        // mod P = 18 · inv(2) = 9.
        let v = vec![7u64, 11];
        let inv2 = (MOD_P + 1) / 2;
        let r = multilinear_extend(&v, &[inv2]).unwrap();
        assert_eq!(r, 9);
    }

    #[test]
    fn mle_constant_function_returns_constant() {
        // All values = 42 → MLE evaluates to 42 anywhere.
        let v = vec![42u64; 8];
        let arbitrary_point = vec![17u64, 23, 31];
        assert_eq!(multilinear_extend(&v, &arbitrary_point).unwrap(), 42);
    }

    #[test]
    fn mle_is_deterministic() {
        let v = vec![1u64, 2, 3, 4, 5, 6, 7, 8];
        let p = vec![3u64, 5, 7];
        let r1 = multilinear_extend(&v, &p).unwrap();
        let r2 = multilinear_extend(&v, &p).unwrap();
        assert_eq!(r1, r2);
    }
}
