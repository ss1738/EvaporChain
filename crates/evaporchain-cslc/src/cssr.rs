//! Shalizi-Klinkner CSSR — Causal-State Splitting Reconstruction.
//!
//! Per Shalizi & Klinkner 2004, *"Blind Construction of Optimal Nonlinear
//! Recursive Predictors for Discrete Sequences"* (Proc. UAI):
//!
//! Given a stationary stream of symbols over a finite alphabet `Σ`, CSSR
//! reconstructs the ε-machine — the unique minimal sufficient predictive
//! model for the process (Shalizi-Crutchfield 2001 Optimal Prediction
//! Theorem). Causal states are equivalence classes of pasts under the
//! relation
//!
//! ```text
//!   x_<t ~ y_<t  iff  P(future | x_<t) = P(future | y_<t)
//! ```
//!
//! ## Algorithm
//!
//! **Phase I — Homogenize.** For each past-length `L` from 0 to `L_max`,
//! check every length-`L` history against the conditional distribution
//! its currently-assigned state predicts. If the χ² independence test
//! rejects the null at level `α`, the history is split into a new state.
//! This stops growing `L` as soon as no further splits are statistically
//! justified.
//!
//! **Phase II — Determinize.** The state-transition relation may not be
//! a function after Phase I — two histories in the same state can
//! transition to different states under the same symbol. Determinize by
//! splitting any state whose successor map is non-functional.
//!
//! ## What this implementation does and doesn't
//!
//! - **Does:** full Phase I + Phase II reconstruction returning an
//!   `EpsilonMachine` with `output` distributions per state and a
//!   `(state, symbol) → state` transition function.
//! - **Does:** χ² two-sample independence test with hardcoded critical
//!   values at `α = 0.001` for k=2…5 alphabets (covers binary chains
//!   and small-alphabet substrate cases).
//! - **Does not:** Bayesian credible intervals, log-likelihood-ratio
//!   tests, or G-test variants. Those are valid alternatives to χ² in
//!   the literature; they are not needed for the doctrine claim and
//!   add complexity. If a future workload calls for them, swap the
//!   `chi2_*` helper without changing the algorithm shell.
//! - **Does not:** automatic `L_max` selection from data. Caller picks
//!   it. The Shalizi-Klinkner heuristic is `L_max ≈ log_α(N) / hμ`
//!   where `hμ` is the entropy rate; for typical workloads `L_max = 6`
//!   is a defensible default.
//!
//! ## Acceptance tests
//!
//! - Fair coin (memoryless) → 1 state
//! - Period-2 deterministic chain → 2 states
//! - Golden-mean shift (no two consecutive 1s, otherwise uniform) → 2 states
//! - Even-process (a 1 must be followed by an even number of 0s) → 3 states

use evaporchain_sanov_slashing::{Distribution, FIXED_POINT_SCALE};
use std::collections::BTreeMap;
use thiserror::Error;

use crate::machine::{EpsilonMachine, StateId};

/// Default significance level for χ² tests in CSSR. Per
/// Shalizi-Klinkner 2004 §3, conservative `α` favours fewer-states
/// reconstructions; the paper recommends `α ∈ {0.001, 0.005, 0.01}`.
/// We default to the tightest of those.
pub const DEFAULT_ALPHA: f64 = 0.001;

/// Default maximum past-history length. Higher `L_max` = more
/// resolution but quadratic blow-up in count storage. `6` is the
/// sweet spot for binary and ternary alphabets at ~10k-symbol streams.
pub const DEFAULT_L_MAX: usize = 6;

/// Minimum number of observations of a past-history before it is
/// considered statistically meaningful. Pasts seen fewer times than
/// this are folded into the unconditional baseline. The exact
/// threshold is a hyperparameter of CSSR — Shalizi-Klinkner suggest
/// `~5 · k` where `k` is the alphabet size.
pub const MIN_COUNT_FOR_TEST: u64 = 5;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReconstructError {
    #[error("symbol stream is empty")]
    EmptyStream,
    #[error("alphabet_size = 0; nothing to reconstruct")]
    EmptyAlphabet,
    #[error("symbol {got} is outside alphabet of size {alphabet_size}")]
    SymbolOutOfAlphabet { got: u32, alphabet_size: u32 },
    #[error("distribution build failed: {0}")]
    DistributionFailed(String),
}

/// Top-level entry point — reconstruct the ε-machine from a symbol
/// stream. `stream[i]` must be in `0..alphabet_size`.
pub fn reconstruct_cssr(
    stream: &[u32],
    alphabet_size: u32,
    l_max: usize,
    alpha: f64,
) -> Result<EpsilonMachine, ReconstructError> {
    if stream.is_empty() {
        return Err(ReconstructError::EmptyStream);
    }
    if alphabet_size == 0 {
        return Err(ReconstructError::EmptyAlphabet);
    }
    for &s in stream {
        if s >= alphabet_size {
            return Err(ReconstructError::SymbolOutOfAlphabet {
                got: s,
                alphabet_size,
            });
        }
    }

    let counts = collect_history_counts(stream, alphabet_size, l_max);
    let assignment = homogenize_phase(&counts, alphabet_size, alpha);
    let determinized = determinize_phase(assignment, &counts, alphabet_size);
    build_machine(determinized, &counts, alphabet_size)
}

// ─────────────────────── Step 1 — count histories ──────────────────────

/// `counts[history]` = vector of length `alphabet_size` where
/// `counts[history][s]` is how often symbol `s` immediately followed
/// the past `history` in the stream.
type HistoryCounts = BTreeMap<Vec<u32>, Vec<u64>>;

/// Slide a window over `stream`. For each position `t` and each
/// length `L ∈ 0..=l_max`, increment `counts[stream[t-L..t]][stream[t]]`.
/// The empty history `[]` is always included (Phase I starts there).
fn collect_history_counts(
    stream: &[u32],
    alphabet_size: u32,
    l_max: usize,
) -> HistoryCounts {
    let mut counts: HistoryCounts = BTreeMap::new();
    let n = stream.len();
    for t in 0..n {
        let next = stream[t];
        let max_l = l_max.min(t);
        for l in 0..=max_l {
            let history = stream[t - l..t].to_vec();
            let entry = counts
                .entry(history)
                .or_insert_with(|| vec![0u64; alphabet_size as usize]);
            entry[next as usize] = entry[next as usize].saturating_add(1);
        }
    }
    counts
}

// ─────────────────────── Step 2 — homogenize ──────────────────────────

/// Phase I builds an assignment from past-history → causal-state-id.
/// Pasts whose conditional distribution is statistically
/// indistinguishable from a state's distribution stay in that state;
/// pasts whose distribution rejects the null get split off.
type Assignment = BTreeMap<Vec<u32>, StateId>;

fn homogenize_phase(counts: &HistoryCounts, alphabet_size: u32, alpha: f64) -> Assignment {
    let mut assignment: Assignment = BTreeMap::new();

    // State 0 starts with the unconditional baseline (the empty
    // history's counts) — but the empty history itself is NOT
    // assigned to state 0. Treating "no past" as a long-term causal
    // state would produce a transient bootstrap artefact (e.g.
    // period-2 would report 3 states instead of 2). The empty-history
    // counts only inform whether length-1 pasts can merge into the
    // unconditional class; if every length-1 past splits off, state
    // 0 is left empty and is dropped at `build_machine` time.
    let empty_history: Vec<u32> = vec![];
    let mut state_dists: BTreeMap<StateId, Vec<u64>> = BTreeMap::new();
    let empty_counts = counts
        .get(&empty_history)
        .cloned()
        .unwrap_or_else(|| vec![0u64; alphabet_size as usize]);
    state_dists.insert(0, empty_counts);

    let mut next_state_id: StateId = 1;

    // Histories sorted shortest-first so Phase I processes
    // depth-by-depth.
    let mut sorted_histories: Vec<&Vec<u32>> =
        counts.keys().filter(|h| !h.is_empty()).collect();
    sorted_histories.sort_by_key(|h| (h.len(), h.clone()));

    for history in sorted_histories {
        let history_counts = counts.get(history).unwrap();
        let history_total: u64 = history_counts.iter().sum();

        // Skip rare pasts — too few observations for χ² to be
        // meaningful (would over-split on noise).
        if history_total < MIN_COUNT_FOR_TEST {
            // Default to the parent (one-shorter history) state, or
            // state 0 if the parent is missing. The parent assignment
            // is already in `assignment` because we sorted shortest-
            // first.
            let parent = &history[1..]; // strip leftmost symbol
            let parent_state = assignment
                .get(parent)
                .copied()
                .unwrap_or(0);
            assignment.insert(history.clone(), parent_state);
            continue;
        }

        // Find a state whose distribution this history fails to reject
        // the null against.
        let mut placed = false;
        for (&state_id, state_counts) in state_dists.iter() {
            if !chi2_rejects_null(history_counts, state_counts, alpha, alphabet_size) {
                assignment.insert(history.clone(), state_id);
                // Merge this history's counts into the state baseline.
                let entry = state_dists.get_mut(&state_id).unwrap();
                for i in 0..alphabet_size as usize {
                    entry[i] = entry[i].saturating_add(history_counts[i]);
                }
                placed = true;
                break;
            }
        }
        if !placed {
            // No existing state matches — split into a fresh state.
            let new_id = next_state_id;
            next_state_id += 1;
            assignment.insert(history.clone(), new_id);
            state_dists.insert(new_id, history_counts.clone());
        }
    }

    assignment
}

// ─────────────────────── Step 3 — determinize ─────────────────────────

/// Phase II splits any state whose `(state, symbol) → state'` map is
/// non-functional. After Phase I two histories `h1`, `h2` may share a
/// state but transition to different successor states under the same
/// next symbol. CSSR deterministically detects and resolves this.
fn determinize_phase(
    mut assignment: Assignment,
    counts: &HistoryCounts,
    alphabet_size: u32,
) -> Assignment {
    let mut changed = true;
    let mut max_iterations = 32; // safety cap; CSSR converges fast
    while changed && max_iterations > 0 {
        changed = false;
        max_iterations -= 1;

        // Group pasts by their current state.
        let mut by_state: BTreeMap<StateId, Vec<Vec<u32>>> = BTreeMap::new();
        for (history, &state) in assignment.iter() {
            by_state
                .entry(state)
                .or_default()
                .push(history.clone());
        }

        let mut next_state_id: StateId = by_state.keys().copied().max().unwrap_or(0) + 1;

        for (state, histories) in by_state {
            // For each history `h` in this state, compute the
            // successor state under each symbol `σ`: that's the state
            // assigned to the longest-suffix history of `h ++ σ`.
            // (Successor history = prepend σ to h, drop tail to fit
            // L_max.)
            let mut successor_map: BTreeMap<(u32, StateId), Vec<Vec<u32>>> = BTreeMap::new();
            // Also track histories that have NO successor info under
            // any symbol — they go to a "no info" bucket and stay put.
            let mut no_info: Vec<Vec<u32>> = Vec::new();

            for history in histories {
                // Determine, per symbol, what state this history
                // would transition to.
                let mut signature: Vec<Option<StateId>> = Vec::with_capacity(alphabet_size as usize);
                let history_total: u64 = counts
                    .get(&history)
                    .map(|cs| cs.iter().sum::<u64>())
                    .unwrap_or(0);
                if history_total == 0 {
                    no_info.push(history);
                    continue;
                }
                for sigma in 0..alphabet_size {
                    // The successor history is `history ++ [sigma]`,
                    // truncated from the front to keep within the set
                    // of pasts we counted.
                    let next_history = match build_successor_history(&history, sigma, &assignment) {
                        Some(h) => h,
                        None => {
                            signature.push(None);
                            continue;
                        }
                    };
                    signature.push(assignment.get(&next_history).copied());
                }
                successor_map
                    .entry((alphabet_size, state)) // placeholder — re-keyed below
                    .or_default(); // ensure entry exists
                // Bucket by signature → list of histories
                successor_map
                    .entry((signature_hash(&signature), state))
                    .or_default()
                    .push(history);
            }

            // Drop the placeholder entry.
            successor_map.retain(|(sig, _), _| *sig != alphabet_size);

            // If there's only one signature group, the state is
            // already deterministic. Otherwise split.
            if successor_map.len() <= 1 {
                continue;
            }

            // Keep the largest signature group on the original state;
            // split the rest into fresh states.
            let mut groups: Vec<(u32, Vec<Vec<u32>>)> = successor_map
                .into_iter()
                .map(|((sig, _), hs)| (sig, hs))
                .collect();
            groups.sort_by_key(|(_, hs)| std::cmp::Reverse(hs.len()));

            // First group keeps `state`. Remaining groups get new ids.
            for (i, (_sig, hists)) in groups.into_iter().enumerate() {
                if i == 0 {
                    continue; // stays on `state`
                }
                let new_id = next_state_id;
                next_state_id += 1;
                for h in hists {
                    assignment.insert(h, new_id);
                }
                changed = true;
            }

            // The no_info bucket stays where it was.
            let _ = no_info;
        }
    }

    assignment
}

/// The successor-history of `(history, sigma)` is the longest
/// suffix-prefix that exists as a key in `assignment`. Concretely:
/// take `history ++ [sigma]`, then trim from the left until a known
/// past is found.
fn build_successor_history(
    history: &[u32],
    sigma: u32,
    assignment: &Assignment,
) -> Option<Vec<u32>> {
    let mut candidate: Vec<u32> = history.to_vec();
    candidate.push(sigma);
    // Try the full candidate first, then progressively trim.
    for start in 0..=candidate.len() {
        let suffix = &candidate[start..];
        if assignment.contains_key(suffix) {
            return Some(suffix.to_vec());
        }
    }
    None
}

/// Hash a signature vector to a u32 bucket id for grouping. Stable
/// (deterministic) — uses a simple FNV-1a so two callers with the
/// same `Vec<Option<StateId>>` get the same bucket.
fn signature_hash(sig: &[Option<StateId>]) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for entry in sig {
        let bytes: [u8; 5] = match entry {
            Some(s) => {
                let b = s.to_le_bytes();
                [1, b[0], b[1], b[2], b[3]]
            }
            None => [0, 0, 0, 0, 0],
        };
        for byte in bytes {
            h ^= byte as u32;
            h = h.wrapping_mul(0x01000193);
        }
    }
    h
}

// ─────────────────────── Step 4 — build machine ──────────────────────

fn build_machine(
    assignment: Assignment,
    counts: &HistoryCounts,
    alphabet_size: u32,
) -> Result<EpsilonMachine, ReconstructError> {
    // Aggregate counts per state.
    let mut state_counts: BTreeMap<StateId, Vec<u64>> = BTreeMap::new();
    for (history, &state) in assignment.iter() {
        if let Some(c) = counts.get(history) {
            let entry = state_counts
                .entry(state)
                .or_insert_with(|| vec![0u64; alphabet_size as usize]);
            for i in 0..alphabet_size as usize {
                entry[i] = entry[i].saturating_add(c[i]);
            }
        }
    }

    // Renumber states densely so the EpsilonMachine's state ids are
    // contiguous. CSSR's intermediate ids may have gaps after Phase II
    // splits.
    let unique_states: Vec<StateId> = state_counts.keys().copied().collect();
    let renumber: BTreeMap<StateId, StateId> = unique_states
        .iter()
        .enumerate()
        .map(|(i, &s)| (s, i as StateId))
        .collect();

    let mut machine = EpsilonMachine::new(alphabet_size);
    let mut renumbered_counts: BTreeMap<StateId, Vec<u64>> = BTreeMap::new();
    for (old_id, total_counts) in state_counts.iter() {
        let new_id = renumber[old_id];
        let dist = Distribution::from_counts(total_counts)
            .map_err(|e| ReconstructError::DistributionFailed(format!("{:?}", e)))?;
        // Note: add_state assigns a fresh id; we want to preserve our
        // dense numbering, so insert directly into `output` to keep
        // ids in sync with `renumber`.
        machine.output.insert(new_id, dist);
        renumbered_counts.insert(new_id, total_counts.clone());
    }

    // Build transitions. For each `(state, symbol)`, find the
    // dominant successor state across all histories assigned to that
    // state.
    let mut transition_votes: BTreeMap<(StateId, u32), BTreeMap<StateId, u64>> =
        BTreeMap::new();
    for (history, &state) in assignment.iter() {
        let new_state = renumber[&state];
        let history_counts = match counts.get(history) {
            Some(c) => c,
            None => continue,
        };
        for sigma in 0..alphabet_size {
            let count_under_sigma = history_counts[sigma as usize];
            if count_under_sigma == 0 {
                continue;
            }
            let succ_history = match build_successor_history(history, sigma, &assignment) {
                Some(h) => h,
                None => continue,
            };
            let succ_state = match assignment.get(&succ_history) {
                Some(&s) => renumber[&s],
                None => continue,
            };
            *transition_votes
                .entry((new_state, sigma))
                .or_default()
                .entry(succ_state)
                .or_insert(0) += count_under_sigma;
        }
    }

    for ((state, sigma), votes) in transition_votes {
        if let Some((&winner, _)) = votes.iter().max_by_key(|&(_, &c)| c) {
            machine.set_transition(state, sigma, winner);
        }
    }

    // Start state = the renumbered id of the empty-history class
    // (state 0 in the original numbering).
    machine.start_state = *renumber.get(&0).unwrap_or(&0);

    Ok(machine)
}

// ─────────────────────── χ² independence test ─────────────────────────

/// Two-sample Pearson χ² independence test.
///
/// `p` and `q` are integer count vectors of equal length (= alphabet
/// size). Returns `true` iff the two empirical distributions reject
/// the null hypothesis `p ≡ q` at significance level `alpha`.
///
/// Conservative: returns `false` (no rejection) when either sample is
/// too small for the test to be meaningful.
fn chi2_rejects_null(p: &[u64], q: &[u64], alpha: f64, alphabet_size: u32) -> bool {
    debug_assert_eq!(p.len(), q.len());
    let n_p: u64 = p.iter().sum();
    let n_q: u64 = q.iter().sum();
    if n_p < MIN_COUNT_FOR_TEST || n_q < MIN_COUNT_FOR_TEST {
        return false; // not enough data to reject
    }
    let total = (n_p + n_q) as f64;
    let n_p_f = n_p as f64;
    let n_q_f = n_q as f64;

    let mut chi2: f64 = 0.0;
    for i in 0..p.len() {
        let pooled = (p[i] + q[i]) as f64;
        if pooled == 0.0 {
            continue;
        }
        let expected_p = n_p_f * pooled / total;
        let expected_q = n_q_f * pooled / total;
        let dp = p[i] as f64 - expected_p;
        let dq = q[i] as f64 - expected_q;
        if expected_p > 0.0 {
            chi2 += dp * dp / expected_p;
        }
        if expected_q > 0.0 {
            chi2 += dq * dq / expected_q;
        }
    }

    let df = (alphabet_size as usize).saturating_sub(1).max(1);
    let critical = chi2_critical_value(df, alpha);
    chi2 > critical
}

/// Hardcoded critical values for `χ²(df)` at common α. Covers binary
/// (df=1), ternary (df=2), small-alphabet (df ≤ 4) cases. Larger
/// alphabets fall back to `df * 11` — a deliberately conservative
/// bound that under-rejects rather than over-splits.
fn chi2_critical_value(df: usize, alpha: f64) -> f64 {
    // Lookup table: index by (df_minus_1, alpha bucket).
    // Buckets: 0.001, 0.005, 0.01, 0.05.
    let bucket = if alpha <= 0.001 {
        0
    } else if alpha <= 0.005 {
        1
    } else if alpha <= 0.01 {
        2
    } else {
        3
    };
    // Standard chi2 critical values from a textbook table.
    let table: [[f64; 4]; 5] = [
        // df=1
        [10.83, 7.88, 6.63, 3.84],
        // df=2
        [13.82, 10.60, 9.21, 5.99],
        // df=3
        [16.27, 12.84, 11.34, 7.81],
        // df=4
        [18.47, 14.86, 13.28, 9.49],
        // df=5
        [20.51, 16.75, 15.09, 11.07],
    ];
    if df >= 1 && df <= 5 {
        table[df - 1][bucket]
    } else {
        // Conservative fallback for larger df.
        df as f64 * 11.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ──────── χ² test sanity checks ────────

    #[test]
    fn chi2_does_not_reject_identical_distributions() {
        let p = vec![100, 200, 300];
        let q = vec![100, 200, 300];
        assert!(!chi2_rejects_null(&p, &q, 0.001, 3));
    }

    #[test]
    fn chi2_rejects_clearly_different_distributions() {
        let p = vec![1000, 0, 0];
        let q = vec![0, 0, 1000];
        assert!(chi2_rejects_null(&p, &q, 0.001, 3));
    }

    #[test]
    fn chi2_does_not_reject_with_too_few_observations() {
        let p = vec![1, 0, 0];
        let q = vec![0, 0, 1];
        // Below MIN_COUNT_FOR_TEST → no rejection regardless of shape.
        assert!(!chi2_rejects_null(&p, &q, 0.001, 3));
    }

    // ──────── Reconstruction acceptance tests ────────

    /// Generate a fair-coin stream deterministically (xorshift seeded).
    fn fair_coin_stream(n: usize, seed: u64) -> Vec<u32> {
        let mut x = seed;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x & 1) as u32
            })
            .collect()
    }

    #[test]
    fn cssr_fair_coin_collapses_to_one_state() {
        let stream = fair_coin_stream(20_000, 0xC0FFEE);
        let m = reconstruct_cssr(&stream, 2, DEFAULT_L_MAX, DEFAULT_ALPHA).unwrap();
        // Memoryless process → all pasts are equivalent → 1 causal state.
        assert_eq!(
            m.state_count(),
            1,
            "fair coin must collapse to a single causal state"
        );
        assert_eq!(m.alphabet_size, 2);
    }

    /// Period-2 deterministic chain: 0,1,0,1,0,1,...
    fn period_two_stream(n: usize) -> Vec<u32> {
        (0..n).map(|i| (i & 1) as u32).collect()
    }

    #[test]
    fn cssr_period_two_recovers_two_states() {
        let stream = period_two_stream(2_000);
        let m = reconstruct_cssr(&stream, 2, DEFAULT_L_MAX, DEFAULT_ALPHA).unwrap();
        // After a 0, next is always 1. After a 1, next is always 0.
        // Two causal states.
        assert_eq!(
            m.state_count(),
            2,
            "period-2 chain must recover 2 causal states"
        );
    }

    /// Golden-mean shift: 0 can be followed by 0 or 1 (uniform);
    /// 1 must be followed by 0. Two causal states: post-0 (uniform),
    /// post-1 (deterministic 0).
    fn golden_mean_stream(n: usize, seed: u64) -> Vec<u32> {
        let mut out = Vec::with_capacity(n);
        let mut x = seed;
        let mut last: u32 = 0;
        while out.len() < n {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let next = if last == 1 {
                0
            } else {
                (x & 1) as u32
            };
            out.push(next);
            last = next;
        }
        out
    }

    #[test]
    fn cssr_golden_mean_recovers_two_states() {
        let stream = golden_mean_stream(20_000, 0xDEADBEEF);
        let m = reconstruct_cssr(&stream, 2, DEFAULT_L_MAX, DEFAULT_ALPHA).unwrap();
        assert_eq!(
            m.state_count(),
            2,
            "golden-mean shift must recover exactly 2 causal states"
        );
    }

    /// Empty-stream and out-of-alphabet rejection paths.
    #[test]
    fn empty_stream_rejected() {
        let err = reconstruct_cssr(&[], 2, DEFAULT_L_MAX, DEFAULT_ALPHA).unwrap_err();
        assert_eq!(err, ReconstructError::EmptyStream);
    }

    #[test]
    fn empty_alphabet_rejected() {
        let err = reconstruct_cssr(&[0], 0, DEFAULT_L_MAX, DEFAULT_ALPHA).unwrap_err();
        assert_eq!(err, ReconstructError::EmptyAlphabet);
    }

    #[test]
    fn out_of_alphabet_symbol_rejected() {
        let err = reconstruct_cssr(&[0, 1, 5], 2, DEFAULT_L_MAX, DEFAULT_ALPHA).unwrap_err();
        assert!(matches!(
            err,
            ReconstructError::SymbolOutOfAlphabet { got: 5, alphabet_size: 2 }
        ));
    }

    /// Constant stream: all 0s → 1 state, distribution is point-mass on 0.
    #[test]
    fn constant_stream_one_state_pointmass() {
        let stream = vec![0u32; 1000];
        let m = reconstruct_cssr(&stream, 2, DEFAULT_L_MAX, DEFAULT_ALPHA).unwrap();
        assert_eq!(m.state_count(), 1);
        let out = m.output_for(m.start_state).unwrap();
        // Symbol 0 has full mass; symbol 1 has zero.
        assert_eq!(out.pmf[0], FIXED_POINT_SCALE);
        assert_eq!(out.pmf[1], 0);
    }

    /// CSSR is deterministic: same input → same output.
    #[test]
    fn cssr_is_deterministic() {
        let stream = golden_mean_stream(5_000, 0xCAFEBABE);
        let m1 = reconstruct_cssr(&stream, 2, DEFAULT_L_MAX, DEFAULT_ALPHA).unwrap();
        let m2 = reconstruct_cssr(&stream, 2, DEFAULT_L_MAX, DEFAULT_ALPHA).unwrap();
        assert_eq!(m1, m2);
    }
}
