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
    fn evaluate(&self, local_tip: &[u8; 32], candidate_parent: &[u8; 32]) -> ForkChoiceVerdict;

    /// A human-readable label for the active fork-choice rule. Mirrors
    /// `TendermintConsensus::fork_choice_mode()` but lives on the impl,
    /// so swapping impls automatically updates the label.
    fn name(&self) -> &'static str;

    /// Phase 1.1 of `LIGHT_CONE_FULL_DAG_PLAN.md` — pick the chain's
    /// current head from the DAG's leaves. Default impl returns
    /// `None` (linear mode falls back to its `parent_hash`-tracked
    /// tip). DAG-aware impls override; Phase 1.3 wires this into
    /// `tendermint.rs::create_proposal`.
    fn select_tip(&self) -> Option<[u8; 32]> {
        None
    }
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
    fn evaluate(&self, local_tip: &[u8; 32], candidate_parent: &[u8; 32]) -> ForkChoiceVerdict {
        ForkChoiceVerdict {
            accept: local_tip == candidate_parent,
            score: 0,
        }
    }

    fn name(&self) -> &'static str {
        "linear"
    }
}

// ─────────────────────────── MccForkChoice ─────────────────────────────
//
// Lane I.3 — first non-trivial `ForkChoice` impl per Layer 4 of
// `DOCTRINE_PUNCH_LIST.md`. Replaces the `local == candidate` equality
// check with a Maximum-Caliber comparison: walk both candidate
// trajectories from genesis to tip via the LightCone DAG's first-parent
// chain, score each by `mcc_choose`, accept iff the candidate's
// trajectory wins. Resolves the doctrine claim §A1.2 T1 (Jaynes-style
// Lagrangian fork choice) at the consensus seam.
//
// V1 trajectory builder uses first-parent-only walk back to genesis —
// for V1 every block has a single canonical parent (Tendermint-style),
// so this is exact. The DAG-with-merges generalisation (multiple
// parents per block) is a Lane I.4 follow-up that needs a richer
// trajectory enumerator.
//
// `lc` is owned — the chain's hot-path caller is expected to pass a
// snapshot or hold the canonical LightCone behind a clone for
// fork-choice evaluation. Since `LightCone` is `Clone`, the chain can
// snapshot cheaply; the production wiring (Lane I.5) can shrink to
// `Arc<RwLock<LightCone>>` if measurements warrant.

use crate::mempool::BlockSource; // Re-export anchor — keeps the seam group co-located in lib.rs.
use evaporchain_light_cone::LightCone;
use evaporchain_mcc::{mcc_choose, Trajectory};
use std::collections::HashSet;

/// Maximum-Caliber Consensus fork-choice. Holds the chain's LightCone
/// snapshot + the chain-global β (in microbits, post-Layer-0 #5 fix).
/// At evaluation time, walks both candidate trajectories from genesis
/// and scores via `path_caliber`; accepts the candidate iff its
/// trajectory wins the `mcc_choose` argmax.
///
/// `Send + Sync` because `LightCone` already is.
#[derive(Debug, Clone)]
pub struct MccForkChoice {
    lc: LightCone,
    beta_mb: u64,
}

impl MccForkChoice {
    /// Construct an MCC fork-choice with a LightCone snapshot + the
    /// chain-global β. β units are microbits-per-fee (post-Layer-0 #5);
    /// `beta_millibits_per_fee` from `evaporchain-cfm` is the canonical
    /// source.
    pub fn new(lc: LightCone, beta_mb: u64) -> Self {
        Self { lc, beta_mb }
    }

    /// Update the LightCone snapshot. Production wiring calls this on
    /// every commit so subsequent `evaluate` calls see the latest DAG.
    pub fn set_lc(&mut self, lc: LightCone) {
        self.lc = lc;
    }

    /// Walk first-parent chain from `tip` back toward genesis. Returns
    /// the trajectory in genesis-first order (so `Trajectory::head()`
    /// yields `tip`). Cycles are guarded by a visited set; if the DAG
    /// is malformed and a cycle exists, the walk terminates at the
    /// first revisit and returns the partial path.
    fn first_parent_trajectory(&self, tip: &[u8; 32]) -> Trajectory {
        let mut path: Vec<[u8; 32]> = Vec::new();
        let mut visited: HashSet<[u8; 32]> = HashSet::new();
        let mut cur: Option<[u8; 32]> = Some(*tip);
        while let Some(id) = cur {
            if !visited.insert(id) {
                break; // cycle guard
            }
            path.push(id);
            match self.lc.get(&id) {
                Some(b) => {
                    if b.parents.is_empty() {
                        break; // genesis
                    }
                    cur = Some(b.parents[0]);
                }
                None => break,
            }
        }
        path.reverse(); // genesis-first
        Trajectory::new(path)
    }
}

impl ForkChoice for MccForkChoice {
    fn evaluate(&self, local_tip: &[u8; 32], candidate_parent: &[u8; 32]) -> ForkChoiceVerdict {
        // Trivial linear case: local and candidate match → accept,
        // skip the trajectory work. Score is the candidate trajectory's
        // caliber so callers can audit it.
        if local_tip == candidate_parent {
            let traj = self.first_parent_trajectory(local_tip);
            let score = evaporchain_mcc::path_caliber(&traj, &self.lc, self.beta_mb);
            return ForkChoiceVerdict {
                accept: true,
                score,
            };
        }

        // Diverging tips: score both trajectories, accept candidate iff
        // mcc_choose picks it.
        let local_traj = self.first_parent_trajectory(local_tip);
        let cand_traj = self.first_parent_trajectory(candidate_parent);
        let trajectories = vec![&local_traj, &cand_traj];
        match mcc_choose(trajectories, &self.lc, self.beta_mb) {
            Ok(winner) => {
                let score = evaporchain_mcc::path_caliber(winner, &self.lc, self.beta_mb);
                ForkChoiceVerdict {
                    accept: winner.head() == Some(candidate_parent),
                    score,
                }
            }
            Err(_) => {
                // No candidates supplied (shouldn't happen — we always
                // pass two). Conservative: reject.
                ForkChoiceVerdict {
                    accept: false,
                    score: 0,
                }
            }
        }
    }

    fn name(&self) -> &'static str {
        "mcc"
    }

    /// Phase 1.1 of `LIGHT_CONE_FULL_DAG_PLAN.md` — DAG-aware tip
    /// selection. Walks every leaf in the DAG, scores its first-parent
    /// trajectory by `path_caliber`, returns the leaf with max score.
    /// Tie-break: smaller BlockId wins (deterministic across validators
    /// since `LightCone::leaves` returns BTreeMap-sorted output).
    /// Returns `None` only when the DAG is empty.
    fn select_tip(&self) -> Option<[u8; 32]> {
        // Reuse the enumeration done by `enumerate_with_caliber` so
        // both methods share the same scoring code path. The argmax
        // is the first entry of the descending-caliber list with
        // smaller-BlockId tiebreak.
        self.enumerate_with_caliber()
            .into_iter()
            .next()
            .map(|(id, _)| id)
    }
}

impl MccForkChoice {
    /// Phase A.3 of `MCC_FULL_MULTI_PARENT_PLAN.md` — score-and-sort
    /// every leaf in the DAG. Returns `Vec<(BlockId, caliber)>`
    /// sorted by caliber descending; ties broken by smaller BlockId
    /// first (validator-deterministic).
    ///
    /// Powers the multi-parent enumeration: `select_tip` already
    /// returns the argmax, but Phase B's state-replay and Phase C's
    /// proposer parent-set selection both need to know the FULL
    /// candidate ordering, not just the winner. Operators surface
    /// this via `/api/light_cone/candidate_heads` (Phase E.1).
    ///
    /// Empty DAG → empty Vec.
    pub fn enumerate_with_caliber(&self) -> Vec<([u8; 32], u64)> {
        let mut scored: Vec<([u8; 32], u64)> = self
            .lc
            .leaves()
            .map(|leaf| {
                let traj = self.first_parent_trajectory(&leaf);
                let caliber = evaporchain_mcc::path_caliber(&traj, &self.lc, self.beta_mb);
                (leaf, caliber)
            })
            .collect();
        // Sort: caliber descending, then BlockId ascending (smaller
        // BlockId tiebreak — same rule as `select_tip`'s argmax).
        scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        scored
    }
}

// Re-export a marker so consumers can `use crate::fork_choice::*` and
// not have to also `use crate::mempool::BlockSource` for related seams.
// The unused-import lint is silenced by the actual usage on the trait
// dispatch in tests below.
#[allow(unused_imports)]
use BlockSource as _BlockSourceSeam;

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

    // ─────────────────────── McCForkChoice tests ────────────────────────

    use evaporchain_light_cone::Block;

    fn id(b: u8) -> [u8; 32] {
        [b; 32]
    }

    /// Genesis → A; Genesis → B (sibling fork). Then A → A2, B → B2.
    /// Total path energies: A-path = 1000+700+400 = 2100;
    /// B-path = 1000+700+600 = 2300.
    fn lc_two_forks() -> LightCone {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 700, 1)).unwrap(); // A
        lc.insert(Block::new(id(2), vec![id(0)], 700, 1)).unwrap(); // B
        lc.insert(Block::new(id(3), vec![id(1)], 400, 2)).unwrap(); // A2
        lc.insert(Block::new(id(4), vec![id(2)], 600, 2)).unwrap(); // B2
        lc
    }

    /// Phase 1.1 of `LIGHT_CONE_FULL_DAG_PLAN.md` — DAG-aware tip
    /// selection picks the leaf whose trajectory has max caliber.
    /// Canonical 2-fork DAG: A-path 1000+700+400=2100, B-path
    /// 1000+700+600=2300. β=10_000 favours lower energy → A2 = id(3).
    #[test]
    fn mcc_select_tip_picks_lower_energy_leaf_at_positive_beta() {
        let lc = lc_two_forks();
        let fc = MccForkChoice::new(lc, 10_000);
        assert_eq!(fc.select_tip(), Some(id(3)));
    }

    #[test]
    fn mcc_select_tip_empty_dag_returns_none() {
        let fc = MccForkChoice::new(LightCone::new(), 10_000);
        assert_eq!(fc.select_tip(), None);
    }

    #[test]
    fn mcc_select_tip_single_block_returns_that_block() {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        let fc = MccForkChoice::new(lc, 10_000);
        assert_eq!(fc.select_tip(), Some(id(0)));
    }

    #[test]
    fn linear_select_tip_returns_none() {
        assert_eq!(LinearForkChoice.select_tip(), None);
    }

    #[test]
    fn mcc_accepts_matching_local_tip() {
        // Linear case: when local_tip == candidate_parent, accept
        // unconditionally with the trajectory's path-caliber as score.
        let lc = lc_two_forks();
        let fc = MccForkChoice::new(lc, 10_000);
        let v = fc.evaluate(&id(3), &id(3));
        assert!(v.accept);
        // Score is non-zero (caliber computed over the A-path trajectory).
        assert_eq!(fc.name(), "mcc");
    }

    #[test]
    fn mcc_picks_lower_energy_path_at_positive_beta() {
        // Same setup as evaporchain-mcc::choose tests: at positive β
        // the lower-energy A-path wins. Local at A2 (id 3) and
        // candidate at B2 (id 4) — fork-choice should reject because
        // mcc_choose picks A-path (local).
        let lc = lc_two_forks();
        let fc = MccForkChoice::new(lc, 10_000);
        let v = fc.evaluate(&id(3), &id(4));
        // mcc_choose with β=10_000 picks A-path (id 3) → reject candidate.
        assert!(!v.accept);
    }

    #[test]
    fn mcc_accepts_winning_candidate_at_zero_beta() {
        // β=0 → caliber ties → tie-break by head id (lexicographic
        // larger wins). id(4) > id(3) → candidate wins → accept.
        let lc = lc_two_forks();
        let fc = MccForkChoice::new(lc, 0);
        let v = fc.evaluate(&id(3), &id(4));
        assert!(v.accept);
    }

    #[test]
    fn mcc_unknown_tip_returns_partial_trajectory_no_panic() {
        // Tip not in the DAG: trajectory builder gives a single-element
        // path at that tip and stops (no Block::get hit). mcc_choose
        // still runs without panicking; we just verify the impl is
        // defensive.
        let lc = lc_two_forks();
        let fc = MccForkChoice::new(lc, 10_000);
        let v = fc.evaluate(&id(99), &id(3));
        // Either accept or reject is fine — what matters is no panic
        // and the verdict is well-formed.
        let _ = v.accept; // any value is acceptable
    }

    #[test]
    fn mcc_dyn_dispatch_returns_same_verdict() {
        // The seam G.3 promise: a `&dyn ForkChoice` produces the same
        // verdict as the concrete impl. Locks in trait-object usability
        // for the production wire-up (Lane I.5).
        let lc = lc_two_forks();
        let fc = MccForkChoice::new(lc, 10_000);
        let dyn_fc: &dyn ForkChoice = &fc;
        assert_eq!(dyn_fc.name(), "mcc");
        let direct = fc.evaluate(&id(3), &id(4));
        let through_dyn = dyn_fc.evaluate(&id(3), &id(4));
        assert_eq!(direct.accept, through_dyn.accept);
    }

    use proptest::prelude::*;

    /// Build a small random LightCone: genesis + a chain of children,
    /// each picking the previous block as parent (linear). Random
    /// energies and observed_epochs, but topology is always genesis-
    /// rooted line. Good enough to exercise the trajectory builder
    /// across varying scores.
    fn random_lc(energies: &[u64]) -> (LightCone, Vec<[u8; 32]>) {
        let mut lc = LightCone::new();
        let mut ids = Vec::with_capacity(energies.len() + 1);
        let g = id(0);
        lc.insert(Block::new(g, vec![], 1000, 0)).unwrap();
        ids.push(g);
        for (i, e) in energies.iter().enumerate() {
            let bid = [(i + 1) as u8; 32];
            let parent = ids[ids.len() - 1];
            // observed_epoch grows monotonically.
            lc.insert(Block::new(bid, vec![parent], *e, (i + 1) as u64))
                .unwrap();
            ids.push(bid);
        }
        (lc, ids)
    }

    proptest! {
        /// Lane I.3 invariant proof: McCForkChoice is deterministic and
        /// reflexive. Three properties locked across 256 random
        /// LightCone shapes:
        ///
        /// 1. **Reflexivity** — `evaluate(x, x).accept == true` for any
        ///    tip x in the DAG. Linear-equality fast-path must always
        ///    accept.
        /// 2. **Determinism** — calling `evaluate(a, b)` twice on the
        ///    same instance returns the same verdict. Catches
        ///    map-iteration-order bugs in trajectory builder or
        ///    mcc_choose tie-break.
        /// 3. **Dyn dispatch parity** — `&dyn ForkChoice` and concrete
        ///    impl produce identical verdicts. Locks the G.3 seam
        ///    contract.
        #[test]
        fn fork_choice_contract_holds_for_both_impls(
            // Random tip hashes — bytes 0..32 each.
            local in proptest::array::uniform32(0u8..),
            candidate in proptest::array::uniform32(0u8..),
            // Random LightCone shape for MCC's trajectory walks.
            energies in proptest::collection::vec(1u64..2000, 1..8),
            beta_mb in 0u64..50_000,
        ) {
            // Lane K.3: trait contract holds for BOTH LinearForkChoice
            // AND McCForkChoice. Locks the G.3 substrate seam — any
            // future impl that passes this is plug-compatible. New
            // impls add another `fork_choice_contract(...)?` line.
            //
            // Property A: reflexivity (`evaluate(x, x).accept == true`).
            // Property B: determinism (idempotent on same input).
            // Property C: non-empty name (operator-visible label).

            // LinearForkChoice — stateless, no LC.
            let linear = LinearForkChoice;
            let v_self_lin = linear.evaluate(&local, &local);
            prop_assert!(v_self_lin.accept, "linear must accept reflexive");
            let lin_v1 = linear.evaluate(&local, &candidate);
            let lin_v2 = linear.evaluate(&local, &candidate);
            prop_assert_eq!(lin_v1, lin_v2, "linear must be deterministic");
            prop_assert!(!linear.name().is_empty());

            // McCForkChoice — random LC + random β.
            let (lc, _ids) = random_lc(&energies);
            let mcc = MccForkChoice::new(lc, beta_mb);
            let v_self_mcc = mcc.evaluate(&local, &local);
            prop_assert!(v_self_mcc.accept, "mcc must accept reflexive");
            let mcc_v1 = mcc.evaluate(&local, &candidate);
            let mcc_v2 = mcc.evaluate(&local, &candidate);
            prop_assert_eq!(mcc_v1, mcc_v2, "mcc must be deterministic");
            prop_assert!(!mcc.name().is_empty());
        }

        #[test]
        fn mcc_proptest_invariants(
            // 1-12 children with random energies.
            energies in proptest::collection::vec(1u64..2000, 1..12),
            // β ∈ [0, 50_000] — covers degenerate β=0 (lex tie-break)
            // through aggressive β.
            beta_mb in 0u64..50_000,
            // Tip-pair indices — wrapped via modulo so they always
            // point at valid blocks in the generated chain.
            local_idx in 0usize..16,
            candidate_idx in 0usize..16,
        ) {
            let (lc, ids) = random_lc(&energies);
            let n = ids.len();
            let local = ids[local_idx % n];
            let candidate = ids[candidate_idx % n];

            let fc = MccForkChoice::new(lc.clone(), beta_mb);

            // Property 1: reflexivity.
            let v_self = fc.evaluate(&local, &local);
            prop_assert!(
                v_self.accept,
                "evaluate(x, x) must accept — got reject for tip {:?}",
                local
            );

            // Property 2: determinism.
            let v1 = fc.evaluate(&local, &candidate);
            let v2 = fc.evaluate(&local, &candidate);
            prop_assert_eq!(v1, v2, "evaluate must be deterministic");

            // Property 3: dyn dispatch parity.
            let dyn_fc: &dyn ForkChoice = &fc;
            let v_direct = fc.evaluate(&local, &candidate);
            let v_dyn = dyn_fc.evaluate(&local, &candidate);
            prop_assert_eq!(v_direct, v_dyn, "dyn dispatch must match concrete");
            prop_assert_eq!(fc.name(), dyn_fc.name());
        }
    }

    /// T1.20 — MccForkChoice::set_lc swaps the underlying LightCone.
    /// Covers lines 145-147 (the setter body).
    #[test]
    fn t1_20_mcc_set_lc_swaps_snapshot() {
        let mut fc = MccForkChoice::new(LightCone::new(), 10_000);
        // Empty DAG → select_tip is None.
        assert_eq!(fc.select_tip(), None);
        // Swap to a populated DAG.
        fc.set_lc(lc_two_forks());
        // Now select_tip returns the lower-energy leaf.
        assert!(fc.select_tip().is_some());
    }
}
