//! Fork-choice substrate seam — Lane G.3.
//!
//! Today the chain assumes single-line history: a proposal whose
//! `parent_hash` differs from `TendermintConsensus.parent_hash` is
//! rejected outright (`tendermint.rs:2526`). That hard equality check is
//! the entire fork-choice surface. The MCC doctrine claim
//! (INVENTION_STACK.md §A1.2 T1) is that fork choice picks the trajectory
//! `argmax exp(−β·E_path)` over candidates — a richer rule that requires
//! comparing scores across alternative chain heads, not just an equality
//! check.
//!
//! This module defines the abstract contract and ships a default
//! [`LinearForkChoice`] that reproduces today's behaviour bit-for-bit.
//! When the Maximum-Caliber Consensus (MCC) impl lands as
//! `McCForkChoice`, the only line of consensus code that changes is the
//! choice of which `Box<dyn ForkChoice>` is wired in at startup.
//!
//! Lane G.3 is the trait-definition step. The migration of
//! `TendermintConsensus`'s inline parent-hash check to a trait dispatch
//! is a follow-on (Lane G.5 — "wire trait into hot path"). Until then
//! this module exists as the named seam alternative impls plug into.

use serde::{Deserialize, Serialize};

/// Decision returned by [`ForkChoice::evaluate`] — richer than a bool so
/// MCC variants can return a tie-break score for diagnostics + light-
/// client reasoning. The hot path only needs `accept`; the score is for
/// audit / observability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkChoiceVerdict {
    /// True iff the candidate parent is accepted as a valid continuation
    /// of our chain.
    pub accept: bool,
    /// Optional score (e.g. path-entropy / max-caliber score). Linear
    /// fork-choice returns 0; MCC returns the actual score for audit.
    pub score: u64,
}

/// Substrate contract for fork-choice rules. `Send + Sync` so consensus
/// engines can hold this behind locks.
///
/// Implementations:
/// - [`LinearForkChoice`] — bit-for-bit reproduction of today's
///   `local == candidate` rule. The default until MCC ships.
/// - `McCForkChoice` (future, Layer 4): path-entropy comparison over
///   the LightCone DAG. Wires into the existing
///   `evaporchain_light_cone::DAG` substrate.
pub trait ForkChoice: Send + Sync {
    /// Decide whether `candidate_parent` is acceptable as the parent of
    /// the next block, given our current chain tip `local_tip`.
    ///
    /// The linear-chain default returns `accept = (local_tip ==
    /// candidate_parent)`. Richer impls (MCC, sumset, AVPL) compute a
    /// path-entropy score over both candidates and accept the higher.
    fn evaluate(
        &self,
        local_tip: &[u8; 32],
        candidate_parent: &[u8; 32],
    ) -> ForkChoiceVerdict;

    /// A human-readable label for the active fork-choice rule. Mirrors
    /// `TendermintConsensus::fork_choice_mode()` but lives on the impl,
    /// so swapping impls automatically updates the label.
    fn name(&self) -> &'static str;
}

/// The chain's default fork-choice rule: a proposal is accepted iff its
/// `parent_hash` exactly matches our current chain tip. Reproduces
/// today's behaviour at `tendermint.rs:2526` bit-for-bit.
///
/// Score is always 0 — there's no path-entropy comparison; either the
/// hashes match or they don't.
#[derive(Debug, Clone, Copy, Default)]
pub struct LinearForkChoice;

impl ForkChoice for LinearForkChoice {
    fn evaluate(
        &self,
        local_tip: &[u8; 32],
        candidate_parent: &[u8; 32],
    ) -> ForkChoiceVerdict {
        ForkChoiceVerdict {
            accept: local_tip == candidate_parent,
            score: 0,
        }
    }

    fn name(&self) -> &'static str {
        "linear"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_accepts_matching_parent() {
        let fc = LinearForkChoice;
        let v = fc.evaluate(&[0xAA; 32], &[0xAA; 32]);
        assert!(v.accept);
        assert_eq!(v.score, 0);
        assert_eq!(fc.name(), "linear");
    }

    #[test]
    fn linear_rejects_diverging_parent() {
        let fc = LinearForkChoice;
        let v = fc.evaluate(&[0xAA; 32], &[0xBB; 32]);
        assert!(!v.accept);
        assert_eq!(v.score, 0);
    }

    #[test]
    fn linear_accepts_zero_against_zero() {
        // Genesis edge case: both tips are the all-zero hash → accept.
        let fc = LinearForkChoice;
        let v = fc.evaluate(&[0u8; 32], &[0u8; 32]);
        assert!(v.accept);
    }

    #[test]
    fn trait_object_dispatch_preserves_behaviour() {
        // Lane G.3 substrate seam: a `&dyn ForkChoice` produces results
        // bit-equal to the concrete impl. Locks in the substrate seam.
        let concrete = LinearForkChoice;
        let dyn_fc: &dyn ForkChoice = &concrete;

        let same = [0xCC; 32];
        let diff = [0xDD; 32];
        assert_eq!(
            concrete.evaluate(&same, &same),
            dyn_fc.evaluate(&same, &same)
        );
        assert_eq!(
            concrete.evaluate(&same, &diff),
            dyn_fc.evaluate(&same, &diff)
        );
        assert_eq!(concrete.name(), dyn_fc.name());
    }

    #[test]
    fn verdict_serde_roundtrip() {
        // The Verdict crosses the wire when light clients audit
        // fork-choice transitions — make sure serde stays stable.
        let v = ForkChoiceVerdict {
            accept: true,
            score: 1234,
        };
        let bytes = serde_json::to_vec(&v).unwrap();
        let v2: ForkChoiceVerdict = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v, v2);
    }
}
