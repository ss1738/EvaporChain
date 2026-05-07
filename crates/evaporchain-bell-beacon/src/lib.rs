//! Bell-Certified Beacon (Tier 2).
//!
//! **Companion: V2.** `evaporchain-bell-beacon-v2` hardens this V1
//! gate onto real chain data: collects concurrent-block-pair windows
//! from the LightCone DAG, runs the gate against an injected
//! coordinated-cartel synthetic, and on Pass issues a chain-attached
//! `BellCertificate` carrying a beacon seed = BLAKE3(domain || window
//! || prev_block || pair_tags). V1 ships the abstract CHSH gate at
//! integer milli-units; V2 wraps it in the chain-binding
//! certificate machinery. V1 and V2 are peers, both live.
//!
//! Per `research/INVENTION_STACK.md` §4.2:
//!
//! > **Bell-Certified Beacon** — Device-independent randomness from
//! > CHSH Bell tests; consumed by Decay-Lamport Time.
//!
//! ## CHSH inequality
//!
//! For any local-realist correlation, the CHSH "S value" is bounded:
//!
//! ```text
//!   |S| = |E(a, b) - E(a, b') + E(a', b) + E(a', b')|  ≤  2
//! ```
//!
//! Quantum-entangled systems can violate this bound (Tsirelson's
//! bound: `|S| ≤ 2√2 ≈ 2.828`). A measured `|S| > 2` is *device-
//! independent* certification that the source is genuinely random
//! (no local hidden variable can fake it).
//!
//! ## What this substrate ships
//!
//! - [`chsh`] — `chsh_s_value(e_ab, e_ab_prime, e_a_prime_b,
//!   e_a_prime_b_prime)` returning the S value in milli-units (×1000)
//!   for integer arithmetic.
//! - [`gate`] — `bell_certified(s_value_milli, threshold_milli)`
//!   returns true iff |S| × 1000 > threshold (default threshold
//!   2000 = the local-realism boundary).
//!
//! Production wires a real entangled-photon source (or reuses an
//! external CHSH-certified randomness service like NIST's beacon)
//! via the chosen `s_value_milli` plumbing. The on-chain rule is the
//! same: reject any beacon emission whose CHSH-S value is at or below
//! the local-realism boundary.

pub mod chsh;
pub mod gate;

pub use chsh::{chsh_s_value, ChshError};
pub use gate::{bell_certified, LOCAL_REALISM_S_MILLI};

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Bell-Beacon V1 ships pure-integer milli-unit
    /// CHSH S computation. Local-realism boundary = 2000 milli;
    /// `bell_certified` admits beacon emissions ONLY when S exceeds
    /// the threshold strictly (no equal-to-boundary aliasing).
    /// Out-of-range correlations fail closed; quantum Bell state at
    /// standard angles produces ≈2828 (Tsirelson)."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Local-realism boundary is the hard-coded threshold.
        assert_eq!(LOCAL_REALISM_S_MILLI, 2000);

        // Quantum Bell state at standard angles: ±707 milli
        // correlations → S ≈ 2828 (Tsirelson bound).
        let s_quantum = chsh_s_value(707, -707, 707, 707).unwrap();
        assert_eq!(s_quantum, 2828);
        // Quantum violation passes the gate.
        assert!(bell_certified(s_quantum, LOCAL_REALISM_S_MILLI));

        // Classical realisable correlations stay below the boundary.
        let s_classical = chsh_s_value(500, -500, 500, 500).unwrap();
        assert_eq!(s_classical, 2000);
        // Boundary-equal does NOT certify (must be strictly greater).
        assert!(!bell_certified(s_classical, LOCAL_REALISM_S_MILLI));

        // Uncorrelated → S = 0 → not certified.
        assert!(!bell_certified(chsh_s_value(0, 0, 0, 0).unwrap(), LOCAL_REALISM_S_MILLI));

        // Out-of-range correlations fail closed.
        assert!(matches!(
            chsh_s_value(1_500, 0, 0, 0),
            Err(ChshError::OutOfRange(_))
        ));
        assert!(matches!(
            chsh_s_value(0, 0, 0, -1_500),
            Err(ChshError::OutOfRange(_))
        ));
    }
}
