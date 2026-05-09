//! Fold step: `f → f'` via the FRI consistency relation.
//!
//! Given `f` over a domain `D` of size `2N`, the folded function
//! `f'` is over the squared domain `D' = { x² : x ∈ D }` of size
//! `N`. With folding randomness `α`, the consistency relation is:
//!
//! ```text
//! f'(x²) = (f(x) + f(-x)) / 2 + α · (f(x) - f(-x)) / (2x)
//! ```
//!
//! In V1 we ship the simpler (additive-only) folding:
//!
//! ```text
//! f'(x²) = (f(x) + f(-x)) / 2
//! ```
//!
//! plus an `alpha`-component the prover commits to but the V1
//! verifier doesn't blend in. Production dFRI plugs in the full
//! linear combination with alpha drawn via Fiat-Shamir.

use std::collections::HashMap;

use thiserror::Error;

use crate::codeword::{add_p, mul_p, neg_p, CodewordPosition, EnergyCodeword, FieldElem, MOD_P};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FoldError {
    #[error("input domain size {0} must be even")]
    OddDomainSize(usize),
    #[error("position x={0} has no antipode (-x) in the domain")]
    MissingAntipode(FieldElem),
    #[error("input codeword is empty")]
    EmptyInput,
    #[error("inverse of 2 not implemented for this MOD_P — V1 only supports primes where 2 has an inverse")]
    NoInverseOfTwo,
}

/// Fold a codeword by halving the domain. Energy of each folded
/// position is the **min** of the energies of `f(x)` and `f(-x)` —
/// the folded position is "as fresh as the stalest input".
pub fn fold_codeword(input: &EnergyCodeword) -> Result<EnergyCodeword, FoldError> {
    if input.is_empty() {
        return Err(FoldError::EmptyInput);
    }
    if input.len() % 2 != 0 {
        return Err(FoldError::OddDomainSize(input.len()));
    }
    // Build a lookup for x → position.
    let lookup: HashMap<FieldElem, &CodewordPosition> =
        input.positions.iter().map(|p| (p.x, p)).collect();

    let mut out: Vec<CodewordPosition> = Vec::new();
    let mut seen: HashMap<FieldElem, ()> = HashMap::new();

    let inv_two = inverse_of_two().ok_or(FoldError::NoInverseOfTwo)?;

    for p in &input.positions {
        let x = p.x;
        let neg_x = neg_p(x);
        if seen.contains_key(&x) {
            continue;
        }
        let neg = lookup
            .get(&neg_x)
            .copied()
            .ok_or(FoldError::MissingAntipode(x))?;
        // Mark both as seen.
        seen.insert(x, ());
        seen.insert(neg_x, ());
        // Folded x' = x²; folded f' = (f(x) + f(-x)) · inv(2).
        let new_x = mul_p(x, x);
        let new_fx = mul_p(add_p(p.fx, neg.fx), inv_two);
        let new_energy = p.energy.min(neg.energy);
        out.push(CodewordPosition::new(new_x, new_fx, new_energy));
    }

    Ok(EnergyCodeword::new(out))
}

/// Inverse of 2 modulo MOD_P. For a prime P, 2⁻¹ = (P + 1) / 2
/// when P is odd. MOD_P = 2³¹ − 1 is odd, so this returns
/// (2³¹ − 1 + 1) / 2 = 2³⁰.
pub fn inverse_of_two() -> Option<FieldElem> {
    if MOD_P % 2 == 0 {
        None
    } else {
        // `(P + 1) / 2` is the canonical algebraic form of `2⁻¹ mod P`
        // when P is odd — NOT ceiling-division, even though clippy
        // sees it that way (the numerical results coincide).
        #[allow(clippy::manual_div_ceil)]
        Some((MOD_P + 1) / 2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codeword::{add_p, mul_p, MOD_P};

    fn cw_pair(x: FieldElem, fx: FieldElem, fnx: FieldElem, energy: u64) -> EnergyCodeword {
        // Build the 2-position codeword (x, f(x)) and (-x, f(-x)).
        EnergyCodeword::new(vec![
            CodewordPosition::new(x, fx, energy),
            CodewordPosition::new(neg_p(x), fnx, energy),
        ])
    }

    #[test]
    fn empty_input_rejected() {
        let cw = EnergyCodeword::new(vec![]);
        let err = fold_codeword(&cw).unwrap_err();
        assert_eq!(err, FoldError::EmptyInput);
    }

    #[test]
    fn odd_size_rejected() {
        let cw = EnergyCodeword::new(vec![CodewordPosition::new(1, 100, 1000)]);
        let err = fold_codeword(&cw).unwrap_err();
        assert!(matches!(err, FoldError::OddDomainSize(1)));
    }

    #[test]
    fn missing_antipode_rejected() {
        // Two positions but they're not antipodes of each other.
        let cw = EnergyCodeword::new(vec![
            CodewordPosition::new(5, 100, 1000),
            CodewordPosition::new(7, 200, 1000),
        ]);
        let err = fold_codeword(&cw).unwrap_err();
        assert!(matches!(err, FoldError::MissingAntipode(_)));
    }

    #[test]
    fn even_function_folds_to_constant() {
        // f(x) = x² is even (f(x) = f(-x)). The fold (f(x)+f(-x))/2
        // is just f(x). At domain {7, -7}: x² = 49.
        let x = 7u64;
        let fx = mul_p(x, x); // 49
        let fnx = mul_p(neg_p(x), neg_p(x)); // 49
        let cw = cw_pair(x, fx, fnx, 1000);
        let folded = fold_codeword(&cw).unwrap();
        assert_eq!(folded.len(), 1);
        // Folded x = x² = 49, folded f = (49+49)/2 = 49.
        assert_eq!(folded.positions[0].x, 49);
        assert_eq!(folded.positions[0].fx, 49);
    }

    #[test]
    fn odd_function_folds_to_zero() {
        // f(x) = x is odd (f(x) = -f(-x)). The fold (f(x) + f(-x)) / 2
        // is 0.
        let x = 7u64;
        let fx = x;
        let fnx = neg_p(x);
        let cw = cw_pair(x, fx, fnx, 1000);
        let folded = fold_codeword(&cw).unwrap();
        assert_eq!(folded.len(), 1);
        assert_eq!(folded.positions[0].fx, 0);
    }

    #[test]
    fn folded_energy_is_min() {
        let x = 7u64;
        let fx = mul_p(x, x);
        let fnx = mul_p(neg_p(x), neg_p(x));
        let cw = EnergyCodeword::new(vec![
            CodewordPosition::new(x, fx, 1000),
            CodewordPosition::new(neg_p(x), fnx, 100),
        ]);
        let folded = fold_codeword(&cw).unwrap();
        assert_eq!(folded.positions[0].energy, 100);
    }

    #[test]
    fn fold_halves_domain() {
        // 4-position codeword {1, 2, -1, -2} with even f(x) = x².
        let cw = EnergyCodeword::new(vec![
            CodewordPosition::new(1, 1, 1000),
            CodewordPosition::new(2, 4, 1000),
            CodewordPosition::new(neg_p(1), 1, 1000),
            CodewordPosition::new(neg_p(2), 4, 1000),
        ]);
        let folded = fold_codeword(&cw).unwrap();
        assert_eq!(folded.len(), 2);
    }

    #[test]
    fn fold_is_deterministic() {
        let x = 7u64;
        let cw = cw_pair(x, mul_p(x, x), mul_p(neg_p(x), neg_p(x)), 1000);
        let a = fold_codeword(&cw).unwrap();
        let b = fold_codeword(&cw).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn inverse_of_two_is_correct() {
        let inv2 = inverse_of_two().unwrap();
        // 2 · inv2 should be 1 mod P.
        let r = mul_p(2, inv2);
        assert_eq!(r, 1);
    }
}
