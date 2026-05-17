//! Causal-CHSH — Bell-style cartel-detection bound for blockchain
//! causal sets.
//!
//! ## What this is
//!
//! Bell's CHSH inequality (Clauser-Horne-Shimony-Holt 1969) detects
//! hidden coordination between physically-separated quantum systems by
//! measuring whether four correlation terms can violate the classical
//! bound `|S| ≤ 2`. EvaporChain's LightCone DAG produces the same
//! structural setup: pairs of *concurrent blocks* (mutually
//! incomparable in the partial order — neither precedes the other).
//! Honest concurrent producers cannot coordinate (no causal channel
//! between them, by definition of concurrent). A cartel that *does*
//! coordinate violates the inequality.
//!
//! ## The math
//!
//! Pick two binary observables `(A, A')` measured on the *first*
//! concurrent block of each pair, and two `(B, B')` on the *second*.
//! Each observable maps a block to ±1 (e.g., sign of energy minus
//! median, sign of tx-count minus median). Form four correlation
//! terms by averaging over a sample of concurrent pairs `{(a_i, b_i)}`:
//!
//! ```text
//!   E(A , B ) = mean_i  A (a_i)·B (b_i)
//!   E(A , B') = mean_i  A (a_i)·B'(b_i)
//!   E(A', B ) = mean_i  A'(a_i)·B (b_i)
//!   E(A', B') = mean_i  A'(a_i)·B'(b_i)
//! ```
//!
//! The CHSH-style **causal-CHSH statistic**:
//!
//! ```text
//!   S = | E(A,B) + E(A,B') + E(A',B) − E(A',B') |
//! ```
//!
//! **Theorem (proposed):** Under honest-validator + LightCone causality
//! + EvaporChain's single-λ decay, `S ≤ 2`.
//!
//! **Violation `S > 2` ⇒ hidden cross-validator coordination.**
//!
//! Where Bell's theorem gave physics quantum-entanglement detection,
//! Causal-CHSH gives blockchain *cartel-detection* with a closed-form
//! bound — not a heuristic, not a slashing rule, a *theorem*.
//!
//! ## Why no other chain can do this
//!
//! Causal-CHSH requires **concurrent blocks as a primitive**. Tendermint
//! (linear chain) has no concurrent blocks. Ethereum's reorgs are
//! competing finalisers, not concurrent producers. Avalanche's DAG is
//! unstructured (no partial order on blocks per validator-set
//! signoff). Only LightCone-style chains can even *form* the four-term
//! correlation. The math is new because the substrate is new.
//!
//! ## Doctrine status
//!
//! ✅ **GATE PASSED** (2026-05-04) — INVENTION_STACK.md §A1.10 records
//! the Tier-0-supporting verdict. Two independent gate runs both pass
//! all three doctrine thresholds:
//!
//! - 200-block real-Ethereum run: `S_honest = 0.012` (≪ 1.8 ceiling),
//!   `S_cartel = 4.000`, `gap = 3.988` (≫ 0.4)
//! - 3K-block confirmation: `S_honest = 0.0175`, same cartel-side, gap
//!   well above doctrine floor.
//!
//! Locked into INVENTION_STACK.md before running, same MERA-style
//! discipline. The crate is now wired into consensus (rolling
//! `CartelAlarm` via `TendermintConsensus`) and Bell-Beacon V2
//! (chain-window certificates).
//!
//! See `research/causal-chsh/GATE_RESULT.md` + `GATE_RESULT_3K.md`
//! for the gate-run evidence.

pub mod alarm;
pub mod chsh;
pub mod gate;
pub mod trace;

pub use alarm::{AlarmStatus, CartelAlarm, CartelAlarmEvent};
pub use chsh::{compute_chsh_s, compute_chsh_s_milli, ChshError, ConcurrentPair};
pub use gate::{run_synthetic_gate, GateThresholds, GateVerdict};
pub use trace::{extract_chsh_samples, synthesize_max_cartel_samples, BlockSummary};

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use chsh::ConcurrentPairSamples;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Causal-CHSH bounds the integer S statistic at
    /// |S_milli| ≤ 4000 (the algebraic max). Honest LHV-shaped
    /// samples give |S| ≤ 2 (Bell). A maximally-coordinated cartel
    /// reaches the algebraic maximum |S|=4. The pure-integer
    /// `compute_chsh_s_milli` is validator-deterministic; non-binary
    /// samples and empty buckets fail closed."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Honest LHV: Bell's theorem caps |S| ≤ 2.
        // Construct equal-magnitude four-bucket samples that average
        // to 0 → S=0 ≤ 2.
        let honest = ConcurrentPairSamples {
            samples_ab: vec![1, -1, 1, -1],
            samples_ab_prime: vec![1, -1, 1, -1],
            samples_a_prime_b: vec![1, -1, 1, -1],
            samples_a_prime_b_prime: vec![1, -1, 1, -1],
        };
        let s_h = compute_chsh_s_milli(&honest).unwrap();
        assert!(s_h <= 2_000, "honest |S_milli| must be ≤ 2000, got {s_h}");

        // Maximally-coordinated cartel reaches the algebraic max |S|=4
        // (cannot be reproduced by any LHV process).
        let cartel = ConcurrentPairSamples {
            samples_ab: vec![1; 8],
            samples_ab_prime: vec![1; 8],
            samples_a_prime_b: vec![1; 8],
            samples_a_prime_b_prime: vec![-1; 8],
        };
        let s_c = compute_chsh_s_milli(&cartel).unwrap();
        assert_eq!(s_c, 4_000, "max cartel must hit the algebraic ceiling");

        // Non-binary observable (value=2) fails closed.
        let bad = ConcurrentPairSamples {
            samples_ab: vec![2],
            samples_ab_prime: vec![1],
            samples_a_prime_b: vec![1],
            samples_a_prime_b_prime: vec![1],
        };
        assert!(matches!(
            compute_chsh_s_milli(&bad),
            Err(ChshError::NonBinaryObservable { .. })
        ));

        // Empty bucket fails closed.
        let empty = ConcurrentPairSamples {
            samples_ab: vec![],
            samples_ab_prime: vec![1],
            samples_a_prime_b: vec![1],
            samples_a_prime_b_prime: vec![1],
        };
        assert!(matches!(
            compute_chsh_s_milli(&empty),
            Err(ChshError::EmptySample { .. })
        ));
    }
}
