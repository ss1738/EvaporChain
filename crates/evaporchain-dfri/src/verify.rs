//! `verify_query_round` — the dFRI verifier query gate.

use thiserror::Error;

use crate::codeword::{add_p, mul_p, neg_p, EnergyCodeword, FieldElem};
use crate::fold::inverse_of_two;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VerifyError {
    #[error("queried position x={x} not present in input codeword")]
    QueriedPositionMissing { x: FieldElem },
    #[error("queried position has energy {energy} below floor {floor}")]
    BelowFloor { energy: u64, floor: u64 },
    #[error("antipode -x={neg_x} not present in input codeword")]
    AntipodeMissing { neg_x: FieldElem },
    #[error("folded position x²={x_sq} not present in folded codeword")]
    FoldedPositionMissing { x_sq: FieldElem },
    #[error("FRI consistency check failed: expected fold({fx}, {f_neg_x}) at x²={x_sq} but folded codeword has {observed}")]
    ConsistencyMismatch {
        x_sq: FieldElem,
        fx: FieldElem,
        f_neg_x: FieldElem,
        observed: FieldElem,
    },
    #[error("inverse of 2 unavailable")]
    NoInverseOfTwo,
}

/// Verify ONE query round of dFRI:
/// 1. Energy floor check on `f(x)` and `f(-x)`.
/// 2. Lookup f(x), f(-x), f'(x²) in their respective codewords.
/// 3. Consistency: `f'(x²) ?= (f(x) + f(-x)) / 2`.
///
/// Returns Ok if all checks pass.
pub fn verify_query_round(
    input: &EnergyCodeword,
    folded: &EnergyCodeword,
    query_x: FieldElem,
    energy_floor: u64,
) -> Result<(), VerifyError> {
    let p_x = input
        .evaluate_at(query_x)
        .ok_or(VerifyError::QueriedPositionMissing { x: query_x })?;
    if p_x.energy < energy_floor {
        return Err(VerifyError::BelowFloor {
            energy: p_x.energy,
            floor: energy_floor,
        });
    }
    let neg_x = neg_p(query_x);
    let p_neg = input
        .evaluate_at(neg_x)
        .ok_or(VerifyError::AntipodeMissing { neg_x })?;
    if p_neg.energy < energy_floor {
        return Err(VerifyError::BelowFloor {
            energy: p_neg.energy,
            floor: energy_floor,
        });
    }

    let x_sq = mul_p(query_x, query_x);
    let p_folded = folded
        .evaluate_at(x_sq)
        .ok_or(VerifyError::FoldedPositionMissing { x_sq })?;

    let inv_two = inverse_of_two().ok_or(VerifyError::NoInverseOfTwo)?;
    let expected = mul_p(add_p(p_x.fx, p_neg.fx), inv_two);
    if expected != p_folded.fx {
        return Err(VerifyError::ConsistencyMismatch {
            x_sq,
            fx: p_x.fx,
            f_neg_x: p_neg.fx,
            observed: p_folded.fx,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codeword::{mul_p, CodewordPosition, EnergyCodeword, MOD_P};
    use crate::fold::fold_codeword;

    fn build_even_codeword(energy: u64) -> EnergyCodeword {
        // f(x) = x² over domain {1, 2, -1, -2}.
        EnergyCodeword::new(vec![
            CodewordPosition::new(1, 1, energy),
            CodewordPosition::new(2, 4, energy),
            CodewordPosition::new(neg_p(1), 1, energy),
            CodewordPosition::new(neg_p(2), 4, energy),
        ])
    }

    // ── consistency relation ────────────────────────────────────

    #[test]
    fn correct_fold_passes_consistency() {
        let input = build_even_codeword(1000);
        let folded = fold_codeword(&input).unwrap();
        // Query at x=1 (and x=2).
        verify_query_round(&input, &folded, 1, 100).unwrap();
        verify_query_round(&input, &folded, 2, 100).unwrap();
    }

    #[test]
    fn tampered_folded_fails_consistency() {
        let input = build_even_codeword(1000);
        let mut folded = fold_codeword(&input).unwrap();
        // Tamper: flip f' at x² = 1 (folded position 1).
        let pos = folded
            .positions
            .iter_mut()
            .find(|p| p.x == 1)
            .unwrap();
        pos.fx = add_p(pos.fx, 1);
        let err = verify_query_round(&input, &folded, 1, 100).unwrap_err();
        assert!(matches!(err, VerifyError::ConsistencyMismatch { .. }));
    }

    // ── energy-floor gate ────────────────────────────────────────

    #[test]
    fn fresh_position_passes_floor() {
        let input = build_even_codeword(1000);
        let folded = fold_codeword(&input).unwrap();
        verify_query_round(&input, &folded, 1, 100).unwrap();
    }

    #[test]
    fn decayed_position_rejected_by_floor() {
        // Build a codeword where x=1 has decayed energy.
        let input = EnergyCodeword::new(vec![
            CodewordPosition::new(1, 1, 50), // below floor 100
            CodewordPosition::new(2, 4, 1000),
            CodewordPosition::new(neg_p(1), 1, 1000),
            CodewordPosition::new(neg_p(2), 4, 1000),
        ]);
        let folded = fold_codeword(&input).unwrap();
        let err = verify_query_round(&input, &folded, 1, 100).unwrap_err();
        assert!(matches!(err, VerifyError::BelowFloor { .. }));
    }

    #[test]
    fn decayed_antipode_rejected_by_floor() {
        let input = EnergyCodeword::new(vec![
            CodewordPosition::new(1, 1, 1000),
            CodewordPosition::new(2, 4, 1000),
            CodewordPosition::new(neg_p(1), 1, 50), // below floor 100
            CodewordPosition::new(neg_p(2), 4, 1000),
        ]);
        let folded = fold_codeword(&input).unwrap();
        let err = verify_query_round(&input, &folded, 1, 100).unwrap_err();
        assert!(matches!(err, VerifyError::BelowFloor { .. }));
    }

    // ── lookup errors ────────────────────────────────────────────

    #[test]
    fn missing_query_position_rejected() {
        let input = build_even_codeword(1000);
        let folded = fold_codeword(&input).unwrap();
        let err = verify_query_round(&input, &folded, 99, 100).unwrap_err();
        assert!(matches!(err, VerifyError::QueriedPositionMissing { .. }));
    }

    #[test]
    fn missing_antipode_rejected() {
        // Codeword with x=1 but no -1.
        let input = EnergyCodeword::new(vec![
            CodewordPosition::new(1, 1, 1000),
            CodewordPosition::new(2, 4, 1000),
            CodewordPosition::new(neg_p(2), 4, 1000),
        ]);
        // Can't fold this (odd size); we'll call verify directly with
        // a manually-built folded codeword.
        let folded = EnergyCodeword::new(vec![CodewordPosition::new(1, 1, 1000)]);
        let err = verify_query_round(&input, &folded, 1, 100).unwrap_err();
        assert!(matches!(err, VerifyError::AntipodeMissing { .. }));
    }

    #[test]
    fn missing_folded_position_rejected() {
        let input = build_even_codeword(1000);
        // Folded codeword that doesn't contain x²=1.
        let folded = EnergyCodeword::new(vec![CodewordPosition::new(99, 99, 1000)]);
        let err = verify_query_round(&input, &folded, 1, 100).unwrap_err();
        assert!(matches!(err, VerifyError::FoldedPositionMissing { .. }));
    }

    // ── doctrine claim ────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Decay-FRI is the first proximity-testing
        // primitive where the verifier's queries respect the
        // chain's energy filtration. Decayed positions are
        // STRUCTURALLY INVALID query targets — even a
        // consistency-honest prover gets rejected if their
        // queried coordinates have evaporated."

        // Honest prover with fresh codeword: passes.
        let honest_input = build_even_codeword(1000);
        let honest_folded = fold_codeword(&honest_input).unwrap();
        verify_query_round(&honest_input, &honest_folded, 1, 100).unwrap();

        // Same prover, but the position they need to use has decayed.
        // The fold itself was honest (took min energy = 50), but the
        // verifier's floor check rejects.
        let decayed_input = EnergyCodeword::new(vec![
            CodewordPosition::new(1, 1, 50),
            CodewordPosition::new(2, 4, 1000),
            CodewordPosition::new(neg_p(1), 1, 50),
            CodewordPosition::new(neg_p(2), 4, 1000),
        ]);
        let decayed_folded = fold_codeword(&decayed_input).unwrap();
        // Querying at x=1 fails because energy=50 < floor=100.
        let err = verify_query_round(&decayed_input, &decayed_folded, 1, 100).unwrap_err();
        assert!(matches!(err, VerifyError::BelowFloor { .. }));
        // But querying at x=2 (still fresh) passes.
        verify_query_round(&decayed_input, &decayed_folded, 2, 100).unwrap();
    }
}
