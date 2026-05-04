//! `CodewordPosition` — one (x, f(x), energy) triple.

use serde::{Deserialize, Serialize};

/// Toy prime field for V1: 2^31 - 1 (Mersenne 31). Production
/// dFRI would plug in a prime-field crate (Goldilocks, BabyBear,
/// or a 256-bit prime).
pub const MOD_P: u64 = 2_147_483_647; // 2³¹ − 1

pub type FieldElem = u64;

/// One position in the codeword.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CodewordPosition {
    /// Domain element x ∈ F_p.
    pub x: FieldElem,
    /// Evaluation f(x) ∈ F_p.
    pub fx: FieldElem,
    /// Energy at this position. Decays under chain's λ.
    pub energy: u64,
}

impl CodewordPosition {
    pub fn new(x: FieldElem, fx: FieldElem, energy: u64) -> Self {
        Self {
            x: x % MOD_P,
            fx: fx % MOD_P,
            energy,
        }
    }
}

/// An energy-tagged codeword: a sequence of `CodewordPosition`s.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnergyCodeword {
    pub positions: Vec<CodewordPosition>,
}

impl EnergyCodeword {
    pub fn new(positions: Vec<CodewordPosition>) -> Self {
        Self { positions }
    }

    pub fn len(&self) -> usize {
        self.positions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }

    /// Lookup by `x` (linear scan; V1 has no Merkle commitment).
    pub fn evaluate_at(&self, x: FieldElem) -> Option<&CodewordPosition> {
        let xn = x % MOD_P;
        self.positions.iter().find(|p| p.x == xn)
    }
}

/// Field addition modulo MOD_P.
pub fn add_p(a: FieldElem, b: FieldElem) -> FieldElem {
    ((a as u128 + b as u128) % (MOD_P as u128)) as u64
}

/// Field subtraction modulo MOD_P.
pub fn sub_p(a: FieldElem, b: FieldElem) -> FieldElem {
    let am = a % MOD_P;
    let bm = b % MOD_P;
    if am >= bm {
        am - bm
    } else {
        MOD_P - (bm - am)
    }
}

/// Field multiplication modulo MOD_P.
pub fn mul_p(a: FieldElem, b: FieldElem) -> FieldElem {
    ((a as u128 * b as u128) % (MOD_P as u128)) as u64
}

/// Negate (additive inverse) in F_p.
pub fn neg_p(a: FieldElem) -> FieldElem {
    sub_p(0, a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_wraps_at_modulus() {
        assert_eq!(add_p(MOD_P - 1, 1), 0);
        assert_eq!(add_p(MOD_P - 1, 2), 1);
        assert_eq!(add_p(0, 0), 0);
    }

    #[test]
    fn sub_wraps_correctly() {
        assert_eq!(sub_p(1, 2), MOD_P - 1);
        assert_eq!(sub_p(MOD_P - 1, MOD_P - 1), 0);
    }

    #[test]
    fn mul_handles_large_operands() {
        let a = MOD_P - 1;
        let b = MOD_P - 1;
        let r = mul_p(a, b);
        // (-1) * (-1) = 1 mod P.
        assert_eq!(r, 1);
    }

    #[test]
    fn neg_is_additive_inverse() {
        let a = 12345u64;
        assert_eq!(add_p(a, neg_p(a)), 0);
    }

    #[test]
    fn position_normalises_x_and_fx_modulo_p() {
        let p = CodewordPosition::new(MOD_P + 5, MOD_P + 7, 1000);
        assert_eq!(p.x, 5);
        assert_eq!(p.fx, 7);
    }

    #[test]
    fn evaluate_at_finds_position() {
        let cw = EnergyCodeword::new(vec![
            CodewordPosition::new(1, 100, 1000),
            CodewordPosition::new(2, 200, 1000),
            CodewordPosition::new(3, 300, 1000),
        ]);
        let p = cw.evaluate_at(2).unwrap();
        assert_eq!(p.fx, 200);
    }

    #[test]
    fn evaluate_at_returns_none_for_missing() {
        let cw = EnergyCodeword::new(vec![CodewordPosition::new(1, 100, 1000)]);
        assert!(cw.evaluate_at(99).is_none());
    }

    #[test]
    fn round_trip_serde() {
        let cw = EnergyCodeword::new(vec![
            CodewordPosition::new(1, 100, 1000),
            CodewordPosition::new(2, 200, 500),
        ]);
        let s = serde_json::to_string(&cw).unwrap();
        let back: EnergyCodeword = serde_json::from_str(&s).unwrap();
        assert_eq!(cw, back);
    }
}
