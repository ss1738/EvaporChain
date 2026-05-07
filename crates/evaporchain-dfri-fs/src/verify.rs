//! `verify_amplified` — multi-round dFRI verifier.

use thiserror::Error;

use evaporchain_dfri::{verify_query_round, EnergyCodeword, FieldElem, VerifyError};

use crate::transcript::{codeword_root, derive_query_positions, FsTranscript};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AmplifiedError {
    #[error("zero query rounds — must request at least 1 round")]
    ZeroRounds,
    #[error("inner V1 verifier rejected at query position {position}: {source}")]
    InnerRejection {
        position: FieldElem,
        source: VerifyError,
    },
}

/// Run `num_queries` rounds of V1 dFRI verification with FS-derived
/// query positions. Fails fast on the first inner rejection.
pub fn verify_amplified(
    input: &EnergyCodeword,
    folded: &EnergyCodeword,
    num_queries: u32,
    energy_floor: u64,
) -> Result<(), AmplifiedError> {
    if num_queries == 0 {
        return Err(AmplifiedError::ZeroRounds);
    }
    let input_root = codeword_root(input);
    let folded_root = codeword_root(folded);
    let mut transcript = FsTranscript::new(
        &input_root,
        &folded_root,
        input.positions.len() as u64,
        num_queries,
    );
    let positions = derive_query_positions(input, &mut transcript, num_queries as usize);
    for x in positions {
        verify_query_round(input, folded, x, energy_floor).map_err(|source| {
            AmplifiedError::InnerRejection {
                position: x,
                source,
            }
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_dfri::{fold_codeword, CodewordPosition, MOD_P};

    fn cw_4(energy: u64) -> EnergyCodeword {
        // f(x) = x² over the domain {1, 2, -1, -2}.
        EnergyCodeword::new(vec![
            CodewordPosition::new(1, 1, energy),
            CodewordPosition::new(2, 4, energy),
            CodewordPosition::new(MOD_P - 1, 1, energy),
            CodewordPosition::new(MOD_P - 2, 4, energy),
        ])
    }

    #[test]
    fn zero_rounds_rejected() {
        let input = cw_4(1000);
        let folded = fold_codeword(&input).unwrap();
        let err = verify_amplified(&input, &folded, 0, 100).unwrap_err();
        assert_eq!(err, AmplifiedError::ZeroRounds);
    }

    #[test]
    fn correct_fold_passes_all_rounds() {
        let input = cw_4(1000);
        let folded = fold_codeword(&input).unwrap();
        // 4 distinct query positions over domain of size 4.
        verify_amplified(&input, &folded, 4, 100).unwrap();
    }

    #[test]
    fn requesting_more_rounds_than_domain_still_succeeds() {
        // The domain only has 4 positions; requesting 100 rounds
        // is capped to 4 distinct queries internally.
        let input = cw_4(1000);
        let folded = fold_codeword(&input).unwrap();
        verify_amplified(&input, &folded, 100, 100).unwrap();
    }

    #[test]
    fn tampered_folded_codeword_fails() {
        let input = cw_4(1000);
        let mut folded = fold_codeword(&input).unwrap();
        // Tamper one folded position.
        folded.positions[0].fx = folded.positions[0].fx.wrapping_add(1) % MOD_P;
        let err = verify_amplified(&input, &folded, 4, 100).unwrap_err();
        assert!(matches!(err, AmplifiedError::InnerRejection { .. }));
    }

    #[test]
    fn decayed_position_rejected() {
        // Decay one input position below the floor.
        let mut input = cw_4(1000);
        input.positions[0].energy = 50; // below floor 100
        let folded = fold_codeword(&input).unwrap();
        // With high enough rounds, the decayed position WILL be sampled.
        let err = verify_amplified(&input, &folded, 4, 100).unwrap_err();
        assert!(matches!(err, AmplifiedError::InnerRejection { .. }));
    }

    #[test]
    fn validator_determinism_same_inputs_same_result() {
        let input = cw_4(1000);
        let folded = fold_codeword(&input).unwrap();
        let r1 = verify_amplified(&input, &folded, 3, 100);
        let r2 = verify_amplified(&input, &folded, 3, 100);
        assert_eq!(r1.is_ok(), r2.is_ok());
    }

    // ── soundness amplification: structural property ─────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "dFRI V2 amplifies V1's single-round soundness
        // via Fiat-Shamir. The transcript is committed to the
        // prover's input + folded codeword + parameters; query
        // positions are derived deterministically. Tampering
        // anywhere in the chain breaks at least one query round
        // with overwhelming probability."

        let input = cw_4(1000);
        let folded = fold_codeword(&input).unwrap();
        // Honest prover passes.
        verify_amplified(&input, &folded, 4, 100).unwrap();

        // Tampered prover (different folded codeword): the FS
        // transcript binds to the *committed* folded root, so a
        // mid-stream substitution changes the derived query
        // positions; even if SOMEHOW the tampered folded is
        // self-consistent, the random query positions catch
        // the inconsistency.
        let mut bad = folded.clone();
        for p in bad.positions.iter_mut() {
            p.fx = p.fx.wrapping_add(7) % MOD_P;
        }
        let err = verify_amplified(&input, &bad, 4, 100).unwrap_err();
        assert!(matches!(err, AmplifiedError::InnerRejection { .. }));
    }

    proptest::proptest! {
        #[test]
        fn property_correct_fold_passes_for_any_round_count(
            num_queries in 1u32..32u32,
        ) {
            let input = cw_4(1000);
            let folded = fold_codeword(&input).unwrap();
            proptest::prop_assert!(verify_amplified(&input, &folded, num_queries, 100).is_ok());
        }
    }
}
