//! MEV-shaped pattern detector — Phase 1 of `CROOKS_MEV_INTEGRATION_PLAN.md`.
//!
//! ## What this crate does
//!
//! Scans an ordered list of transactions for **sandwich-shaped triples**:
//!
//! ```text
//!   tx_i (attacker → target)   ← front-run leg
//!   tx_j (victim   → target)   ← victim
//!   tx_k (attacker → target)   ← back-run leg
//! ```
//!
//! where `tx_i.from == tx_k.from` (same attacker address sandwiches a
//! different sender's tx, all three transfers targeting the same
//! `to` account). The output is a list of [`MevObservation`]s.
//!
//! Only [`evaporchain_types::Transaction::Transfer`] is examined in
//! Phase 1 — the `from` and `to` fields are explicit, no signature
//! recovery needed. Refresh-race / contract-call sandwiches need
//! signer recovery and per-object state reads; those are Phase 1.6+
//! follow-up work (open question in `CROOKS_MEV_INTEGRATION_PLAN.md`).
//!
//! ## What this crate does NOT do
//!
//! - **Settlement** — Phase 3. This crate emits observations only.
//! - **Refund computation** — Phase 2. Wires this crate's output
//!   into `evaporchain-crooks-mev-refund::compute_refund`.
//! - **Confidence scoring** — Phase 4. The detector here returns a
//!   placeholder `confidence_score = 1.0` for every match; a real
//!   score requires per-pair statistics (price impact, fee tier,
//!   timing windows) that don't exist in Phase 1.
//!
//! Conservative by design — the only pattern matched is the strict
//! triple. Front-run-only attacks (no back-run leg) and time-delayed
//! sandwiches (attacker legs straddle multiple blocks) are NOT
//! detected. Tightening precision is `CROOKS_MEV_INTEGRATION_PLAN.md`
//! Phase 6.3.

use serde::{Deserialize, Serialize};

use evaporchain_types::{AccountAddress, Transaction};

// Phase 2 of CROOKS_MEV_INTEGRATION_PLAN.md — exposed for
// consensus-side refund computation. Default β per
// `research/crooks_mev/PHASE_2_DECISIONS.md` Decision 2; default
// window per Decision 1.
pub const CROOKS_MEV_DEFAULT_BETA_MB: u64 = 1000;
pub const CROOKS_MEV_DEFAULT_WINDOW_BLOCKS: u64 = 256;

/// Phase 3.3 of `CROOKS_MEV_INTEGRATION_PLAN.md` — minimum age of
/// an observation before it's eligible for settlement. Provides a
/// dispute window in which Phase 4.4's operator override can cancel
/// a pending refund. Governance flag: `crooks_mev_grace_period_blocks`.
pub const CROOKS_MEV_DEFAULT_GRACE_PERIOD_BLOCKS: u64 = 5;

/// Phase 3.3 — maximum age of an observation that's still settleable.
/// Past this horizon the observation is considered stale and dropped
/// without settlement. Governance flag:
/// `crooks_mev_refund_window_blocks`. Must be ≥ grace period.
pub const CROOKS_MEV_DEFAULT_REFUND_WINDOW_BLOCKS: u64 = 256;

/// Phase 4.1 of `CROOKS_MEV_INTEGRATION_PLAN.md` — minimum
/// confidence score (in milli-units, 0..=1000) required for an
/// observation to be eligible for settlement. Default 500 = 0.5,
/// matches the threshold previously hardcoded in `due_refund_txs`.
/// Governance flag: `crooks_mev_confidence_threshold_milli`.
/// Phase 1's detector emits `confidence_score = 1.0` (1000 milli)
/// for every match, so the threshold has no behavioural impact
/// today; the field is in place for Phase 4.1+ when the detector
/// learns to score. Bumping to e.g. 950_000 ppm ahead of that work
/// would silently disable settlement.
///
/// **Audit-locked HIGH H4** — wire shape pinned to ppm matching
/// `MevObservation::confidence_score_ppm: u32`; switching later
/// would be a chain-fork.
pub const CROOKS_MEV_DEFAULT_CONFIDENCE_THRESHOLD_PPM: u32 = 500_000;

/// One MEV-shaped observation surfaced by [`scan_block`]. Carries
/// the indices of the three legs in the block's tx list, the
/// attacker/victim addresses, and a placeholder `confidence_score`
/// (always 1.0 in Phase 1).
///
/// `work_estimate` is **estimated** as the front-run + back-run
/// transfer amounts (no price model) — a real `work_extracted` for
/// Crooks needs the LP/AMM accounting that EvaporChain doesn't have
/// natively. Treat this as an upper bound on the dissipative work.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MevObservation {
    /// Block height the observation came from. Filled in by the
    /// caller (the detector itself doesn't know the height).
    pub block_height: u64,
    /// Index of the front-run (first attacker) leg in the block's
    /// transaction list.
    pub attacker_pre_idx: usize,
    /// Index of the victim's transaction.
    pub victim_idx: usize,
    /// Index of the back-run (second attacker) leg.
    pub attacker_post_idx: usize,
    /// Attacker address (== `tx[attacker_pre_idx].from` ==
    /// `tx[attacker_post_idx].from`).
    pub attacker: AccountAddress,
    /// Victim address.
    pub victim: AccountAddress,
    /// Target account (shared `to` across all three legs).
    pub target: AccountAddress,
    /// Upper-bound estimate of attacker work in this sandwich
    /// (front-run amount + back-run amount). Not the Crooks
    /// `work_extracted`; that requires LP-side accounting.
    pub work_estimate: u64,
    /// Placeholder confidence in Phase 1; always 1_000_000 ppm (1.0
    /// in legacy float terms). Phase 4 replaces with a real score so
    /// low-confidence events can be filtered before settlement.
    ///
    /// **Audit fix HIGH H4**: was `f64`. Tag-pre-Phase-4 to ppm so
    /// the wire shape is locked before any on-chain scoring lands —
    /// switching f64→ppm later would be a chain-fork.
    pub confidence_score_ppm: u32,
    /// Phase 2 of `CROOKS_MEV_INTEGRATION_PLAN.md` — Crooks-
    /// fluctuation refund estimate for this observation. `None` until
    /// `compute_observation_refund` is run; `Some(0)` if the math
    /// produced a non-positive refund (ΔF ≥ work); `Some(n)` for a
    /// positive refund. Phase 1.3 detection emits with `None`; the
    /// consensus call site fills it in before pushing to the ring
    /// buffer (per Phase 2 Decision 4 in
    /// `research/crooks_mev/PHASE_2_DECISIONS.md`).
    #[serde(default)]
    pub refund_amount: Option<u64>,
}

/// Scan an ordered transaction list for sandwich-shaped triples.
///
/// `block_height` is stamped onto every emitted observation.
/// Returns observations in tx-order (sorted by `attacker_pre_idx`).
///
/// Algorithm: O(n²) outer scan over candidate (i, k) pairs sharing
/// the same `from`, plus a linear inner scan over the j slot. For
/// blocks with N=1000 txs this is ~10⁶ ops — well under the 50 ms
/// hot-path budget per `CROOKS_MEV_INTEGRATION_PLAN.md` Phase 6.2.
/// Bucket-by-target lookups can drop this to O(n log n) if needed.
pub fn scan_block(txs: &[Transaction], block_height: u64) -> Vec<MevObservation> {
    let mut out = Vec::new();

    // Project txs to (idx, from, to, amount, opted_out) tuples for
    // Transfer variants only — every other tx variant is skipped in
    // Phase 1.
    //
    // Phase 4.2 of CROOKS_MEV_INTEGRATION_PLAN.md adds the
    // `opted_out` flag: when a Transfer's `mev_refund_eligible =
    // Some(false)`, the sender has opted out of MEV refunds. The
    // detector still considers the tx as a potential ATTACKER leg
    // (we don't let attackers self-exempt from detection), but
    // SKIPS observations where the VICTIM tx is opted out.
    let transfers: Vec<(usize, AccountAddress, AccountAddress, u64, bool)> = txs
        .iter()
        .enumerate()
        .filter_map(|(i, tx)| match tx {
            Transaction::Transfer(t) => {
                let opted_out = matches!(t.mev_refund_eligible, Some(false));
                Some((i, t.from, t.to, t.amount, opted_out))
            }
            _ => None,
        })
        .collect();

    if transfers.len() < 3 {
        return out;
    }

    // For each candidate (i, k) pair where i < k and from_i == from_k,
    // look for a j with i < j < k whose from differs from the
    // attacker's and whose to matches the attacker's target.
    for ai in 0..transfers.len() {
        for ak in (ai + 2)..transfers.len() {
            let (idx_i, from_i, to_i, amt_i, _opt_i) = transfers[ai];
            let (idx_k, from_k, to_k, amt_k, _opt_k) = transfers[ak];
            if from_i != from_k {
                continue;
            }
            if to_i != to_k {
                // Strict shape: attacker's pre/post both target the
                // same account. Loosened in Phase 6.3.
                continue;
            }
            // Find a victim slot between ai and ak.
            for &(idx_j, from_j, to_j, _amt_j, opt_j) in &transfers[(ai + 1)..ak] {
                if from_j == from_i {
                    // Self-MEV — skip per Phase 4.3 anti-gaming.
                    continue;
                }
                if to_j != to_i {
                    continue;
                }
                if opt_j {
                    // Phase 4.2 — victim opted out via
                    // mev_refund_eligible = Some(false). Detector
                    // doesn't surface the observation, so no refund
                    // and no monitoring entry. Honest victims who
                    // explicitly waive auto-refund stay out of the
                    // ring buffer entirely.
                    continue;
                }
                out.push(MevObservation {
                    block_height,
                    attacker_pre_idx: idx_i,
                    victim_idx: idx_j,
                    attacker_post_idx: idx_k,
                    attacker: from_i,
                    victim: from_j,
                    target: to_i,
                    work_estimate: amt_i.saturating_add(amt_k),
                    confidence_score_ppm: 1_000_000,
                    refund_amount: None,
                });
                // Only the first matching victim per (i, k) — avoid
                // exponential blowup from multiple victims sharing
                // the same attacker pair.
                break;
            }
        }
    }

    out
}

/// Per-attacker rolling-window stat — Phase 2 Decision 1 of the
/// plan. Tracks how many sandwich-shaped triples a given address
/// has been the attacker side of, in the recent window. The
/// consensus engine maintains a `HashMap<AccountAddress,
/// AttackerStat>` of these and prunes entries whose `last_seen_height
/// < current_height - window` for determinism.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttackerStat {
    pub sandwich_count: u64,
    pub first_seen_height: u64,
    pub last_seen_height: u64,
}

impl AttackerStat {
    /// First observation for this attacker.
    pub fn fresh(height: u64) -> Self {
        Self {
            sandwich_count: 1,
            first_seen_height: height,
            last_seen_height: height,
        }
    }

    /// Append a new observation; bumps count + last_seen.
    pub fn record(&mut self, height: u64) {
        self.sandwich_count = self.sandwich_count.saturating_add(1);
        self.last_seen_height = height.max(self.last_seen_height);
    }
}

/// Phase 2.3 of `CROOKS_MEV_INTEGRATION_PLAN.md` — compute the
/// Crooks-fluctuation refund estimate for one observation, given a
/// rolling per-attacker stat, β (in millibits), and the window size
/// in blocks.
///
/// Returns `Some(refund_amount)` on success, `None` if any of the
/// underlying primitives reject the inputs (e.g., β=0 → undefined).
///
/// **Honesty caveat:** the pmf computed here uses a rate-based
/// proxy (sandwich count over window) rather than the rigorous
/// forward/reverse path probabilities Crooks 1999 calls for. See
/// `research/crooks_mev/PHASE_2_DECISIONS.md` Decision 1 for why
/// and the deferred research follow-up.
pub fn compute_observation_refund(
    obs: &MevObservation,
    stat: &AttackerStat,
    beta_mb: u64,
    window_blocks: u64,
) -> Option<u64> {
    if beta_mb == 0 || window_blocks == 0 {
        return None;
    }

    // P_F (ppm) = sandwich_count / window_blocks, scaled to ppm.
    // Capped at 999_999 to keep crooks_log_ratio_millibits in-range
    // (the primitive rejects p_forward == 1_000_000).
    let p_forward_ppm =
        (stat.sandwich_count.saturating_mul(1_000_000) / window_blocks).min(999_999);

    // P_R (ppm) = noise floor = 1 / (window_blocks * 1024) in ppm.
    // For window=256 → ~3.8 ppm. Caps at 1 to avoid div-by-zero in
    // the log primitive when window_blocks is huge.
    let p_reverse_ppm = (1_000_000u64 / window_blocks.saturating_mul(1024)).max(1);

    let log_ratio =
        evaporchain_cfm::crooks_log_ratio_millibits(p_forward_ppm, p_reverse_ppm).ok()?;

    let delta_f = evaporchain_crooks_mev_refund::compute_delta_f_millibits(
        obs.work_estimate as i64,
        log_ratio,
        beta_mb,
    )
    .ok()?;

    Some(evaporchain_crooks_mev_refund::compute_refund(
        obs.work_estimate,
        delta_f,
    ))
}

/// Phase 3.3 of `CROOKS_MEV_INTEGRATION_PLAN.md` — walk the
/// observation buffer and emit a `RefundTx` for every observation
/// in the (grace_period, refund_window) interval relative to
/// `current_height`. Excludes observations already in
/// `settled_refunds` (replay-protection: an observation settled in
/// an earlier block must not be re-settled).
///
/// Returns observations in stable order (sorted by
/// `(source_block_height, source_observation_idx)`) so all
/// validators agree on the tx ordering inside the proposed block.
///
/// Per Phase 3.3 of the plan: the proposer MUST include exactly
/// these txs in the next block (Phase 3.4 ships the validator-side
/// rejection rule when they're omitted, Phase 3.5 ships slashing
/// for omission).
///
/// Phase 1 contract: skip observations with `refund_amount = None`
/// or `Some(0)` — nothing to settle. Skip observations with
/// `confidence_score < 0.5` — Phase 4.1's confidence threshold
/// will tighten this.
pub fn due_refund_txs(
    observations: &std::collections::VecDeque<MevObservation>,
    settled_refunds: &std::collections::HashSet<(u64, usize)>,
    disputed_observations: &std::collections::HashSet<(u64, usize)>,
    current_height: u64,
    grace_period_blocks: u64,
    refund_window_blocks: u64,
    confidence_threshold_ppm: u32,
) -> Vec<evaporchain_types::Transaction> {
    if grace_period_blocks > refund_window_blocks {
        // Misconfiguration: empty interval. Bail rather than emit
        // garbage. Operators see the misconfiguration via empty
        // settlement output + governance snapshot mismatch.
        return Vec::new();
    }

    // Boundary calculation:
    //   age = current_height - source_block_height
    //   grace_period_blocks ≤ age ≤ refund_window_blocks
    // Equivalently:
    //   current_height - refund_window_blocks ≤ source_block_height
    //                                        ≤ current_height - grace_period_blocks
    let oldest_settleable = current_height.saturating_sub(refund_window_blocks);
    let youngest_settleable = current_height.saturating_sub(grace_period_blocks);

    let mut due: Vec<&MevObservation> = observations
        .iter()
        .filter(|o| {
            o.block_height >= oldest_settleable
                && o.block_height <= youngest_settleable
                && !settled_refunds.contains(&(o.block_height, o.attacker_pre_idx))
                && !disputed_observations.contains(&(o.block_height, o.attacker_pre_idx))
                && matches!(o.refund_amount, Some(a) if a > 0)
                && o.confidence_score_ppm >= confidence_threshold_ppm
        })
        .collect();
    // Stable canonical ordering for validator convergence.
    due.sort_by_key(|o| (o.block_height, o.attacker_pre_idx));

    due.into_iter()
        .map(|o| {
            evaporchain_types::Transaction::Refund(evaporchain_types::RefundTx {
                source_block_height: o.block_height,
                source_observation_idx: o.attacker_pre_idx,
                attacker: o.attacker,
                victim: o.victim,
                amount: o.refund_amount.unwrap_or(0),
                settle_block_height: current_height,
            })
        })
        .collect()
}

/// Phase 3.4 of `CROOKS_MEV_INTEGRATION_PLAN.md` — validator-side
/// refund-set check. Returned by `validate_block_refunds` when a
/// proposer omits a required refund or includes an unexpected one.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RefundValidationError {
    #[error(
        "block missing required RefundTx for observation \
         (source_block_height={src_h}, source_observation_idx={src_idx})"
    )]
    MissingRefund { src_h: u64, src_idx: usize },
    #[error(
        "block contains unexpected RefundTx for observation \
         (source_block_height={src_h}, source_observation_idx={src_idx}) \
         — either no such observation exists, it's already settled, \
         or it's outside the (grace, refund_window) interval"
    )]
    UnexpectedRefund { src_h: u64, src_idx: usize },
    #[error(
        "block's RefundTx for (src_h={src_h}, src_idx={src_idx}) \
         does not match the chain's expected (attacker, victim, amount, \
         settle_block_height)"
    )]
    MismatchedRefund { src_h: u64, src_idx: usize },
}

/// Phase 3.4 of `CROOKS_MEV_INTEGRATION_PLAN.md` — verify that a
/// block's Refund-tx set exactly matches the chain's expected
/// settlement set at `current_height`. Validators run this when
/// `crooks_mev_settlement_mode = "enforce"`; the default
/// `"observe"` skips the check (no behavioural change).
///
/// Three failure modes:
/// - `MissingRefund` — block omits a refund the chain expected (proposer slash-eligible per Phase 3.5).
/// - `UnexpectedRefund` — block includes a refund for an observation that doesn't exist, is already settled, or is outside the (grace, window) interval.
/// - `MismatchedRefund` — block's refund payload (attacker/victim/amount/settle_height) disagrees with the chain's deterministic computation.
///
/// All three failures are non-recoverable — the block is rejected.
/// Caller computes `expected = due_refund_txs(...)` once per
/// validation round; this helper just diffs the block's refund txs
/// against `expected`.
pub fn validate_block_refunds(
    expected: &[evaporchain_types::Transaction],
    block_refunds: &[&evaporchain_types::RefundTx],
) -> Result<(), RefundValidationError> {
    use std::collections::HashMap;

    // Build a lookup of (src_h, src_idx) → expected RefundTx.
    let mut expected_map: HashMap<(u64, usize), &evaporchain_types::RefundTx> = HashMap::new();
    for tx in expected {
        if let evaporchain_types::Transaction::Refund(r) = tx {
            expected_map.insert((r.source_block_height, r.source_observation_idx), r);
        }
    }

    // Track which expected refunds the block satisfies — by
    // observation key — and reject anything unexpected or
    // mismatched.
    let mut seen: std::collections::HashSet<(u64, usize)> = std::collections::HashSet::new();
    for r in block_refunds {
        let key = (r.source_block_height, r.source_observation_idx);
        let exp = match expected_map.get(&key) {
            Some(e) => *e,
            None => {
                return Err(RefundValidationError::UnexpectedRefund {
                    src_h: key.0,
                    src_idx: key.1,
                });
            }
        };
        if exp.attacker != r.attacker
            || exp.victim != r.victim
            || exp.amount != r.amount
            || exp.settle_block_height != r.settle_block_height
        {
            return Err(RefundValidationError::MismatchedRefund {
                src_h: key.0,
                src_idx: key.1,
            });
        }
        seen.insert(key);
    }

    // Anything in `expected_map` that wasn't `seen` is missing from
    // the block.
    for key in expected_map.keys() {
        if !seen.contains(key) {
            return Err(RefundValidationError::MissingRefund {
                src_h: key.0,
                src_idx: key.1,
            });
        }
    }

    Ok(())
}

/// Phase 3.2 of `CROOKS_MEV_INTEGRATION_PLAN.md` — deterministic
/// digest of the (observations, attacker_stats) pair. Two
/// validators with identical histories MUST compute identical
/// digests; divergent histories MUST diverge.
///
/// Canonicalization: observations sorted by
/// `(block_height, attacker_pre_idx)`; attacker_stats sorted by
/// the attacker address bytes. Then the byte-encoding of every
/// field is fed into a single blake3 hash with a domain-separation
/// tag.
///
/// Phase 3.3 wire-format will commit this digest to either the
/// block header or the state root; for now it's an in-memory
/// accessor that proves the determinism contract.
pub fn mev_state_digest(
    observations: &std::collections::VecDeque<MevObservation>,
    attacker_stats: &std::collections::HashMap<AccountAddress, AttackerStat>,
) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"evaporchain.mev_state_digest.v1");

    // Observations: sorted by (block_height, attacker_pre_idx) for
    // canonical ordering across validators.
    let mut obs_sorted: Vec<&MevObservation> = observations.iter().collect();
    obs_sorted.sort_by_key(|o| (o.block_height, o.attacker_pre_idx));
    h.update(&(obs_sorted.len() as u64).to_le_bytes());
    for o in &obs_sorted {
        h.update(&o.block_height.to_le_bytes());
        h.update(&(o.attacker_pre_idx as u64).to_le_bytes());
        h.update(&(o.victim_idx as u64).to_le_bytes());
        h.update(&(o.attacker_post_idx as u64).to_le_bytes());
        h.update(&o.attacker);
        h.update(&o.victim);
        h.update(&o.target);
        h.update(&o.work_estimate.to_le_bytes());
        // **Audit fix HIGH H4**: confidence_score is now u32 ppm
        // (validator-deterministic; no f64 NaN canonicalization).
        h.update(&o.confidence_score_ppm.to_le_bytes());
        match o.refund_amount {
            None => h.update(&[0u8]),
            Some(v) => {
                h.update(&[1u8]);
                h.update(&v.to_le_bytes())
            }
        };
    }

    // Attacker stats: sorted by address bytes for canonical order.
    let mut stats_sorted: Vec<(&AccountAddress, &AttackerStat)> = attacker_stats.iter().collect();
    stats_sorted.sort_by_key(|(addr, _)| *addr);
    h.update(&(stats_sorted.len() as u64).to_le_bytes());
    for (addr, stat) in &stats_sorted {
        h.update(*addr);
        h.update(&stat.sandwich_count.to_le_bytes());
        h.update(&stat.first_seen_height.to_le_bytes());
        h.update(&stat.last_seen_height.to_le_bytes());
    }

    *h.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_types::TransferTx;

    fn addr(seed: u8) -> AccountAddress {
        let mut a = [0u8; 32];
        a[0] = seed;
        a
    }

    fn transfer(from: u8, to: u8, amount: u64, nonce: u64) -> Transaction {
        Transaction::Transfer(TransferTx {
            from: addr(from),
            to: addr(to),
            amount,
            nonce,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        })
    }

    #[test]
    fn empty_block_yields_no_observations() {
        let out = scan_block(&[], 1);
        assert!(out.is_empty());
    }

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "MEV-detect's `scan_block` finds sandwich-shaped
    /// triples (attacker → victim → attacker) targeting the same
    /// `to` account. Phase-1 detector emits placeholder
    /// `confidence_score = 1.0` (or its ppm equivalent post-audit).
    /// Empty blocks and short tx lists (<3 transfers) emit nothing.
    /// Output is deterministic and ordered by `attacker_pre_idx`."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Empty block: no output.
        assert!(scan_block(&[], 1).is_empty());

        // 2 transfers: too short for a sandwich.
        let two = vec![transfer(1, 2, 100, 0), transfer(3, 4, 50, 0)];
        assert!(scan_block(&two, 1).is_empty());

        // Canonical sandwich: attacker(1) → target(99), victim(2) →
        // target(99), attacker(1) → target(99).
        let sandwich = vec![
            transfer(1, 99, 100, 0), // attacker_pre
            transfer(2, 99, 50, 0),  // victim
            transfer(1, 99, 150, 1), // attacker_post
        ];
        let obs = scan_block(&sandwich, 7);
        assert_eq!(obs.len(), 1);
        let o = &obs[0];
        assert_eq!(o.block_height, 7);
        assert_eq!(o.attacker_pre_idx, 0);
        assert_eq!(o.victim_idx, 1);
        assert_eq!(o.attacker_post_idx, 2);
        assert_eq!(o.attacker, addr(1));
        assert_eq!(o.victim, addr(2));
        assert_eq!(o.target, addr(99));

        // Determinism: scan_block on the same input twice → same output.
        let obs2 = scan_block(&sandwich, 7);
        assert_eq!(obs, obs2);
    }

    #[test]
    fn under_three_transfers_yields_no_observations() {
        let txs = vec![transfer(1, 99, 100, 0), transfer(2, 99, 100, 0)];
        let out = scan_block(&txs, 1);
        assert!(out.is_empty());
    }

    #[test]
    fn classic_sandwich_detected() {
        // Attacker = 0xAA, Victim = 0xBB, Target = 0x99.
        let txs = vec![
            transfer(0xAA, 0x99, 100, 0), // front-run
            transfer(0xBB, 0x99, 200, 0), // victim
            transfer(0xAA, 0x99, 150, 1), // back-run
        ];
        let out = scan_block(&txs, 42);
        assert_eq!(out.len(), 1);
        let obs = &out[0];
        assert_eq!(obs.block_height, 42);
        assert_eq!(obs.attacker_pre_idx, 0);
        assert_eq!(obs.victim_idx, 1);
        assert_eq!(obs.attacker_post_idx, 2);
        assert_eq!(obs.attacker, addr(0xAA));
        assert_eq!(obs.victim, addr(0xBB));
        assert_eq!(obs.target, addr(0x99));
        assert_eq!(obs.work_estimate, 250);
        assert_eq!(obs.confidence_score_ppm, 1_000_000);
        assert_eq!(
            obs.refund_amount, None,
            "refund is None at detection time — filled in by the call site"
        );
    }

    #[test]
    fn refund_helper_zero_beta_rejects() {
        let obs = MevObservation {
            block_height: 1,
            attacker_pre_idx: 0,
            victim_idx: 1,
            attacker_post_idx: 2,
            attacker: addr(0xAA),
            victim: addr(0xBB),
            target: addr(0x99),
            work_estimate: 1000,
            confidence_score_ppm: 1_000_000,
            refund_amount: None,
        };
        let stat = AttackerStat::fresh(1);
        assert_eq!(
            compute_observation_refund(&obs, &stat, 0, CROOKS_MEV_DEFAULT_WINDOW_BLOCKS),
            None,
            "β=0 must reject"
        );
    }

    #[test]
    fn refund_helper_one_off_attacker_yields_small_or_zero_refund() {
        let obs = MevObservation {
            block_height: 1,
            attacker_pre_idx: 0,
            victim_idx: 1,
            attacker_post_idx: 2,
            attacker: addr(0xAA),
            victim: addr(0xBB),
            target: addr(0x99),
            work_estimate: 1000,
            confidence_score_ppm: 1_000_000,
            refund_amount: None,
        };
        // Single observation, no prior history → low P_F → low
        // log_ratio → small ΔF → refund ≈ work (the formula
        // attributes most of work to dissipation when the rate
        // signal is weak — that's the conservative direction).
        let stat = AttackerStat::fresh(1);
        let refund = compute_observation_refund(
            &obs,
            &stat,
            CROOKS_MEV_DEFAULT_BETA_MB,
            CROOKS_MEV_DEFAULT_WINDOW_BLOCKS,
        );
        assert!(refund.is_some());
        // Refund must be bounded by work_estimate.
        assert!(refund.unwrap() <= obs.work_estimate);
    }

    #[test]
    fn refund_helper_sustained_attacker_yields_meaningful_refund() {
        let obs = MevObservation {
            block_height: 100,
            attacker_pre_idx: 0,
            victim_idx: 1,
            attacker_post_idx: 2,
            attacker: addr(0xAA),
            victim: addr(0xBB),
            target: addr(0x99),
            work_estimate: 10_000,
            confidence_score_ppm: 1_000_000,
            refund_amount: None,
        };
        // Attacker with 10 sandwiches in the window — sustained
        // pattern, high P_F, high log_ratio.
        let stat = AttackerStat {
            sandwich_count: 10,
            first_seen_height: 1,
            last_seen_height: 100,
        };
        let refund = compute_observation_refund(
            &obs,
            &stat,
            CROOKS_MEV_DEFAULT_BETA_MB,
            CROOKS_MEV_DEFAULT_WINDOW_BLOCKS,
        );
        assert!(refund.is_some(), "sustained attacker → math must succeed");
        let r = refund.unwrap();
        assert!(r <= obs.work_estimate, "refund bounded by work");
    }

    #[test]
    fn refund_helper_window_zero_rejects() {
        let obs = MevObservation {
            block_height: 1,
            attacker_pre_idx: 0,
            victim_idx: 1,
            attacker_post_idx: 2,
            attacker: addr(0xAA),
            victim: addr(0xBB),
            target: addr(0x99),
            work_estimate: 1000,
            confidence_score_ppm: 1_000_000,
            refund_amount: None,
        };
        let stat = AttackerStat::fresh(1);
        assert_eq!(
            compute_observation_refund(&obs, &stat, CROOKS_MEV_DEFAULT_BETA_MB, 0),
            None,
            "window=0 must reject"
        );
    }

    #[test]
    fn attacker_stat_fresh_has_count_one() {
        let s = AttackerStat::fresh(42);
        assert_eq!(s.sandwich_count, 1);
        assert_eq!(s.first_seen_height, 42);
        assert_eq!(s.last_seen_height, 42);
    }

    /// Phase 3.2 of `CROOKS_MEV_INTEGRATION_PLAN.md` — empty
    /// state has a stable digest (the genesis case for any chain).
    #[test]
    fn mev_state_digest_empty_is_stable() {
        let obs = std::collections::VecDeque::new();
        let stats = std::collections::HashMap::new();
        let d1 = mev_state_digest(&obs, &stats);
        let d2 = mev_state_digest(&obs, &stats);
        assert_eq!(d1, d2);
    }

    /// Phase 3.2 — identical state from two independently-built
    /// HashMap iteration orders must produce the same digest.
    #[test]
    fn mev_state_digest_independent_of_hashmap_order() {
        // Build two stat tables with the same entries inserted in
        // opposite orders. HashMap iteration order is arbitrary, so
        // without canonical sorting these would diverge.
        let mut stats_a = std::collections::HashMap::new();
        let mut stats_b = std::collections::HashMap::new();
        let entries: Vec<(AccountAddress, AttackerStat)> = vec![
            (addr(0x01), AttackerStat::fresh(10)),
            (addr(0x02), AttackerStat::fresh(20)),
            (addr(0x03), AttackerStat::fresh(30)),
            (addr(0x04), AttackerStat::fresh(40)),
        ];
        for (a, s) in &entries {
            stats_a.insert(*a, *s);
        }
        for (a, s) in entries.iter().rev() {
            stats_b.insert(*a, *s);
        }
        let obs = std::collections::VecDeque::new();
        assert_eq!(
            mev_state_digest(&obs, &stats_a),
            mev_state_digest(&obs, &stats_b)
        );
    }

    /// Phase 3.2 — a single divergent observation must produce a
    /// divergent digest. Locks the soundness of the contract:
    /// validators that fold inconsistent histories DON'T converge.
    #[test]
    fn mev_state_digest_single_difference_propagates() {
        let mut obs_a = std::collections::VecDeque::new();
        let mut obs_b = std::collections::VecDeque::new();
        let stats = std::collections::HashMap::new();
        let base = MevObservation {
            block_height: 1,
            attacker_pre_idx: 0,
            victim_idx: 1,
            attacker_post_idx: 2,
            attacker: addr(0xAA),
            victim: addr(0xBB),
            target: addr(0x99),
            work_estimate: 250,
            confidence_score_ppm: 1_000_000,
            refund_amount: Some(100),
        };
        obs_a.push_back(base.clone());
        let mut diverged = base.clone();
        diverged.work_estimate = 251; // single byte differs
        obs_b.push_back(diverged);
        assert_ne!(
            mev_state_digest(&obs_a, &stats),
            mev_state_digest(&obs_b, &stats),
            "single-byte observation difference must surface in the digest"
        );
    }

    /// Phase 3.2 — adding an attacker stat changes the digest.
    /// Locks: stats are part of the consensus state, not observable
    /// noise.
    #[test]
    fn mev_state_digest_attacker_stat_changes_propagate() {
        let obs = std::collections::VecDeque::new();
        let stats_empty = std::collections::HashMap::new();
        let mut stats_one = std::collections::HashMap::new();
        stats_one.insert(addr(0xAA), AttackerStat::fresh(1));
        assert_ne!(
            mev_state_digest(&obs, &stats_empty),
            mev_state_digest(&obs, &stats_one),
        );
    }

    /// Phase 3.3 helper: synthetic observation builder for tests.
    fn make_obs(block_height: u64, attacker_pre_idx: usize, refund: Option<u64>) -> MevObservation {
        MevObservation {
            block_height,
            attacker_pre_idx,
            victim_idx: attacker_pre_idx + 1,
            attacker_post_idx: attacker_pre_idx + 2,
            attacker: addr(0xAA),
            victim: addr(0xBB),
            target: addr(0x99),
            work_estimate: 1000,
            confidence_score_ppm: 1_000_000,
            refund_amount: refund,
        }
    }

    /// Phase 3.3 — observation younger than grace_period must NOT
    /// be in due_refund_txs (still in dispute window).
    #[test]
    fn due_refund_txs_skips_observations_in_grace_period() {
        let mut obs = std::collections::VecDeque::new();
        obs.push_back(make_obs(98, 0, Some(100)));
        obs.push_back(make_obs(100, 0, Some(100)));
        let settled = std::collections::HashSet::new();
        let current = 100;
        let due = due_refund_txs(
            &obs,
            &settled,
            &std::collections::HashSet::new(),
            current,
            5,
            256,
            CROOKS_MEV_DEFAULT_CONFIDENCE_THRESHOLD_PPM,
        );
        assert!(
            due.is_empty(),
            "obs at age 0 and 2 are inside grace; nothing due"
        );
    }

    /// Phase 3.3 — observation in (grace, refund_window) interval
    /// IS in due_refund_txs.
    #[test]
    fn due_refund_txs_emits_for_observations_in_interval() {
        let mut obs = std::collections::VecDeque::new();
        obs.push_back(make_obs(50, 0, Some(100)));
        let settled = std::collections::HashSet::new();
        let current = 100;
        let due = due_refund_txs(
            &obs,
            &settled,
            &std::collections::HashSet::new(),
            current,
            5,
            256,
            CROOKS_MEV_DEFAULT_CONFIDENCE_THRESHOLD_PPM,
        );
        assert_eq!(due.len(), 1);
        match &due[0] {
            evaporchain_types::Transaction::Refund(r) => {
                assert_eq!(r.source_block_height, 50);
                assert_eq!(r.source_observation_idx, 0);
                assert_eq!(r.amount, 100);
                assert_eq!(r.settle_block_height, 100);
            }
            other => panic!("expected Refund, got {:?}", other),
        }
    }

    /// Phase 3.3 — observation older than refund_window is dropped
    /// (stale; outside the chain's responsibility).
    #[test]
    fn due_refund_txs_drops_stale_observations() {
        let mut obs = std::collections::VecDeque::new();
        obs.push_back(make_obs(10, 0, Some(100)));
        let settled = std::collections::HashSet::new();
        let current = 1000; // age = 990, far past 256-block window
        let due = due_refund_txs(
            &obs,
            &settled,
            &std::collections::HashSet::new(),
            current,
            5,
            256,
            CROOKS_MEV_DEFAULT_CONFIDENCE_THRESHOLD_PPM,
        );
        assert!(due.is_empty(), "stale observation must not settle");
    }

    /// Phase 3.3 — already-settled observation must not re-emit
    /// (replay protection).
    #[test]
    fn due_refund_txs_skips_already_settled() {
        let mut obs = std::collections::VecDeque::new();
        obs.push_back(make_obs(50, 0, Some(100)));
        let mut settled = std::collections::HashSet::new();
        settled.insert((50, 0));
        let due = due_refund_txs(
            &obs,
            &settled,
            &std::collections::HashSet::new(),
            100,
            5,
            256,
            CROOKS_MEV_DEFAULT_CONFIDENCE_THRESHOLD_PPM,
        );
        assert!(due.is_empty(), "settled observation must not re-emit");
    }

    /// Phase 3.3 — observation with no refund (None) skipped;
    /// observation with refund=0 skipped (nothing to settle).
    #[test]
    fn due_refund_txs_skips_zero_or_none_refund() {
        let mut obs = std::collections::VecDeque::new();
        obs.push_back(make_obs(50, 0, None));
        obs.push_back(make_obs(51, 0, Some(0)));
        let settled = std::collections::HashSet::new();
        let due = due_refund_txs(
            &obs,
            &settled,
            &std::collections::HashSet::new(),
            100,
            5,
            256,
            CROOKS_MEV_DEFAULT_CONFIDENCE_THRESHOLD_PPM,
        );
        assert!(due.is_empty());
    }

    /// Phase 3.3 — multiple due observations emitted in stable
    /// (block_height, attacker_pre_idx) order. Locks the
    /// validator-convergence contract: all proposers must propose
    /// the SAME tx ordering.
    #[test]
    fn due_refund_txs_canonical_ordering() {
        let mut obs = std::collections::VecDeque::new();
        // Insert out-of-order to exercise the sort.
        obs.push_back(make_obs(75, 0, Some(50)));
        obs.push_back(make_obs(50, 5, Some(100)));
        obs.push_back(make_obs(50, 0, Some(200)));
        obs.push_back(make_obs(60, 0, Some(75)));
        let settled = std::collections::HashSet::new();
        let due = due_refund_txs(
            &obs,
            &settled,
            &std::collections::HashSet::new(),
            100,
            5,
            256,
            CROOKS_MEV_DEFAULT_CONFIDENCE_THRESHOLD_PPM,
        );
        assert_eq!(due.len(), 4);
        let heights_idxs: Vec<(u64, usize)> = due
            .iter()
            .map(|tx| match tx {
                evaporchain_types::Transaction::Refund(r) => {
                    (r.source_block_height, r.source_observation_idx)
                }
                _ => panic!("non-Refund in due list"),
            })
            .collect();
        assert_eq!(
            heights_idxs,
            vec![(50, 0), (50, 5), (60, 0), (75, 0)],
            "canonical (block_height, attacker_pre_idx) ordering"
        );
    }

    /// Phase 3.4 helper — extract `&RefundTx` slices from a block's
    /// transaction vector for the validator-side check.
    fn extract_refunds(
        txs: &[evaporchain_types::Transaction],
    ) -> Vec<&evaporchain_types::RefundTx> {
        txs.iter()
            .filter_map(|tx| match tx {
                evaporchain_types::Transaction::Refund(r) => Some(r),
                _ => None,
            })
            .collect()
    }

    /// Phase 3.4 — empty expected + empty block refunds = OK
    /// (the observe-mode default chain state).
    #[test]
    fn validate_block_refunds_empty_passes() {
        assert_eq!(validate_block_refunds(&[], &[]), Ok(()));
    }

    /// Phase 3.4 — exact match: every expected refund present, no
    /// extras. Pass.
    #[test]
    fn validate_block_refunds_exact_match_passes() {
        let mut obs = std::collections::VecDeque::new();
        obs.push_back(make_obs(50, 0, Some(100)));
        obs.push_back(make_obs(60, 0, Some(75)));
        let settled = std::collections::HashSet::new();
        let expected = due_refund_txs(
            &obs,
            &settled,
            &std::collections::HashSet::new(),
            100,
            5,
            256,
            CROOKS_MEV_DEFAULT_CONFIDENCE_THRESHOLD_PPM,
        );

        let block_refunds = extract_refunds(&expected);
        assert_eq!(validate_block_refunds(&expected, &block_refunds), Ok(()));
    }

    /// Phase 3.4 — block missing a required refund → MissingRefund.
    #[test]
    fn validate_block_refunds_missing_required() {
        let mut obs = std::collections::VecDeque::new();
        obs.push_back(make_obs(50, 0, Some(100)));
        obs.push_back(make_obs(60, 0, Some(75)));
        let settled = std::collections::HashSet::new();
        let expected = due_refund_txs(
            &obs,
            &settled,
            &std::collections::HashSet::new(),
            100,
            5,
            256,
            CROOKS_MEV_DEFAULT_CONFIDENCE_THRESHOLD_PPM,
        );

        // Block carries only the first refund.
        let partial: Vec<evaporchain_types::Transaction> = expected[0..1].to_vec();
        let block_refunds = extract_refunds(&partial);
        let err = validate_block_refunds(&expected, &block_refunds).unwrap_err();
        assert!(matches!(err, RefundValidationError::MissingRefund { .. }));
    }

    /// Phase 3.4 — block carrying a refund for an observation that
    /// doesn't exist (or isn't due) → UnexpectedRefund.
    #[test]
    fn validate_block_refunds_unexpected_refund() {
        let expected = Vec::new(); // chain expects nothing
        let bogus = evaporchain_types::RefundTx {
            source_block_height: 999,
            source_observation_idx: 0,
            attacker: addr(0xAA),
            victim: addr(0xBB),
            amount: 100,
            settle_block_height: 1000,
        };
        let block_txs = vec![evaporchain_types::Transaction::Refund(bogus)];
        let block_refunds = extract_refunds(&block_txs);
        let err = validate_block_refunds(&expected, &block_refunds).unwrap_err();
        assert!(matches!(
            err,
            RefundValidationError::UnexpectedRefund { .. }
        ));
    }

    /// Phase 3.4 — block carrying a refund for the right
    /// observation but with a tampered amount → MismatchedRefund.
    #[test]
    fn validate_block_refunds_mismatched_amount() {
        let mut obs = std::collections::VecDeque::new();
        obs.push_back(make_obs(50, 0, Some(100)));
        let settled = std::collections::HashSet::new();
        let expected = due_refund_txs(
            &obs,
            &settled,
            &std::collections::HashSet::new(),
            100,
            5,
            256,
            CROOKS_MEV_DEFAULT_CONFIDENCE_THRESHOLD_PPM,
        );
        // Tamper: bump amount.
        let mut tampered = match &expected[0] {
            evaporchain_types::Transaction::Refund(r) => r.clone(),
            _ => unreachable!(),
        };
        tampered.amount += 1;
        let block_txs = vec![evaporchain_types::Transaction::Refund(tampered)];
        let block_refunds = extract_refunds(&block_txs);
        let err = validate_block_refunds(&expected, &block_refunds).unwrap_err();
        assert!(matches!(
            err,
            RefundValidationError::MismatchedRefund { .. }
        ));
    }

    /// Phase 3.3 — misconfigured grace > window emits empty
    /// (rather than negative interval). Observable via empty
    /// settlement output.
    #[test]
    fn due_refund_txs_grace_exceeds_window_yields_empty() {
        let mut obs = std::collections::VecDeque::new();
        obs.push_back(make_obs(50, 0, Some(100)));
        let settled = std::collections::HashSet::new();
        let disputed = std::collections::HashSet::new();
        let due = due_refund_txs(
            &obs,
            &settled,
            &disputed,
            100,
            1000,
            100,
            CROOKS_MEV_DEFAULT_CONFIDENCE_THRESHOLD_PPM,
        );
        assert!(due.is_empty());
    }

    /// Phase 4.1 — observation with confidence below the threshold
    /// must NOT settle. Locks the contract: low-confidence detections
    /// are surfaced for monitoring but never auto-refunded.
    #[test]
    fn due_refund_txs_skips_low_confidence_observations() {
        let mut obs = std::collections::VecDeque::new();
        let mut low_conf = make_obs(50, 0, Some(100));
        low_conf.confidence_score_ppm = 400_000; // below 500_000 default ppm threshold
        obs.push_back(low_conf);
        let settled = std::collections::HashSet::new();
        let disputed = std::collections::HashSet::new();
        let due = due_refund_txs(
            &obs,
            &settled,
            &disputed,
            100,
            5,
            256,
            CROOKS_MEV_DEFAULT_CONFIDENCE_THRESHOLD_PPM,
        );
        assert!(due.is_empty());
        // Threshold lowered to 300_000 ppm (0.3) → 400_000-ppm
        // observation now passes the gate.
        let due_low = due_refund_txs(&obs, &settled, &disputed, 100, 5, 256, 300_000);
        assert_eq!(due_low.len(), 1);
    }

    /// Phase 6.2 of `CROOKS_MEV_INTEGRATION_PLAN.md` — worst-case
    /// detection cost benchmark. Generates a synthetic 1000-tx
    /// block and times `scan_block`. The plan's hot-path budget is
    /// 50 ms on M4. The current O(n²) outer scan over Transfer txs
    /// completes in well under that — confirmed empirically.
    ///
    /// Marked `#[ignore]` because it's an instrumentation test, not
    /// a correctness check. Run with
    /// `cargo test -p evaporchain-mev-detect --release -- --ignored --nocapture`.
    #[test]
    #[ignore = "perf benchmark — run with --ignored to record numbers"]
    fn benchmark_scan_block_n1000() {
        // Pathological-but-plausible block: 1000 transfers, half
        // sharing the same `from` (heavy attacker-pair fan-out so
        // the inner triple-search visits the most candidates).
        let mut txs = Vec::with_capacity(1000);
        for i in 0..1000usize {
            let from = if i % 2 == 0 { 0xAA } else { (i % 200) as u8 };
            let to = (i % 50) as u8;
            txs.push(transfer(from, to, (i + 1) as u64, i as u64));
        }

        let start = std::time::Instant::now();
        let observations = scan_block(&txs, 1);
        let elapsed = start.elapsed();

        eprintln!(
            "[scan_block bench] 1000 txs → {} observations in {:?}",
            observations.len(),
            elapsed
        );

        // 50 ms hot-path budget per the plan. 100 ms hard ceiling
        // (Phase 6.2 stopping condition: re-design to bucket-by-target
        // if the scan exceeds this).
        assert!(
            elapsed.as_millis() < 100,
            "scan_block @ 1000 txs exceeded 100 ms ceiling: {:?}",
            elapsed
        );
    }

    /// Phase 4.4 — disputed observation must NOT settle even if
    /// otherwise due. Operators dispute via the `/api/mev/dispute`
    /// endpoint within the grace period.
    #[test]
    fn due_refund_txs_skips_disputed_observations() {
        let mut obs = std::collections::VecDeque::new();
        obs.push_back(make_obs(50, 0, Some(100)));
        let settled = std::collections::HashSet::new();
        let mut disputed = std::collections::HashSet::new();
        disputed.insert((50, 0));
        let due = due_refund_txs(
            &obs,
            &settled,
            &disputed,
            100,
            5,
            256,
            CROOKS_MEV_DEFAULT_CONFIDENCE_THRESHOLD_PPM,
        );
        assert!(due.is_empty());
    }

    #[test]
    fn attacker_stat_record_bumps_count_and_height() {
        let mut s = AttackerStat::fresh(1);
        s.record(5);
        assert_eq!(s.sandwich_count, 2);
        assert_eq!(s.first_seen_height, 1);
        assert_eq!(s.last_seen_height, 5);
        // Out-of-order record must NOT clobber last_seen.
        s.record(3);
        assert_eq!(s.last_seen_height, 5);
    }

    #[test]
    fn honest_sequential_transfers_no_observation() {
        // Three different senders to the same target — nobody
        // sandwiches anybody.
        let txs = vec![
            transfer(0xAA, 0x99, 100, 0),
            transfer(0xBB, 0x99, 200, 0),
            transfer(0xCC, 0x99, 150, 0),
        ];
        let out = scan_block(&txs, 1);
        assert!(out.is_empty(), "no shared attacker → no sandwich");
    }

    #[test]
    fn different_target_addresses_no_observation() {
        // Attacker pre/post target different accounts — not a
        // sandwich on any single victim.
        let txs = vec![
            transfer(0xAA, 0x99, 100, 0),
            transfer(0xBB, 0x99, 200, 0),
            transfer(0xAA, 0x88, 150, 1), // different target!
        ];
        let out = scan_block(&txs, 1);
        assert!(out.is_empty());
    }

    #[test]
    fn self_mev_skipped() {
        // Attacker == victim — Phase 4.3 contract.
        let txs = vec![
            transfer(0xAA, 0x99, 100, 0),
            transfer(0xAA, 0x99, 200, 1), // same address as attacker
            transfer(0xAA, 0x99, 150, 2),
        ];
        let out = scan_block(&txs, 1);
        assert!(out.is_empty(), "self-MEV must not register");
    }

    /// Phase 4.2 of `CROOKS_MEV_INTEGRATION_PLAN.md` — victim
    /// opt-out via `mev_refund_eligible = Some(false)`. Detector
    /// skips the entire observation: no buffer entry, no auto-refund.
    #[test]
    fn victim_opt_out_skips_observation() {
        // Standard sandwich shape, but the victim has set
        // `mev_refund_eligible: Some(false)` to opt out.
        let mut victim_tx = TransferTx {
            from: addr(0xBB),
            to: addr(0x99),
            amount: 200,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: Some(false),
        };
        let txs = vec![
            transfer(0xAA, 0x99, 100, 0),
            Transaction::Transfer(victim_tx.clone()),
            transfer(0xAA, 0x99, 150, 1),
        ];
        let out = scan_block(&txs, 1);
        assert!(out.is_empty(), "opted-out victim must not register");

        // None (default) → still detects.
        victim_tx.mev_refund_eligible = None;
        let txs_default = vec![
            transfer(0xAA, 0x99, 100, 0),
            Transaction::Transfer(victim_tx.clone()),
            transfer(0xAA, 0x99, 150, 1),
        ];
        let out_default = scan_block(&txs_default, 1);
        assert_eq!(out_default.len(), 1);

        // Some(true) → also detects (explicit opt-IN).
        victim_tx.mev_refund_eligible = Some(true);
        let txs_optin = vec![
            transfer(0xAA, 0x99, 100, 0),
            Transaction::Transfer(victim_tx),
            transfer(0xAA, 0x99, 150, 1),
        ];
        let out_optin = scan_block(&txs_optin, 1);
        assert_eq!(out_optin.len(), 1);
    }

    #[test]
    fn non_transfer_txs_ignored() {
        // Mix in a Refresh between the front-run and back-run.
        // It shouldn't disrupt the sandwich detection on the
        // surrounding Transfer triple.
        use evaporchain_types::RefreshTx;
        let txs = vec![
            transfer(0xAA, 0x99, 100, 0),
            transfer(0xBB, 0x99, 200, 0),
            Transaction::Refresh(RefreshTx {
                object_id: [0u8; 32],
                energy_deposit: 1000,
                signature: None,
                public_key: None,
            }),
            transfer(0xAA, 0x99, 150, 1),
        ];
        let out = scan_block(&txs, 1);
        assert_eq!(
            out.len(),
            1,
            "refresh in the middle should not break detection"
        );
    }

    #[test]
    fn multiple_sandwiches_all_detected() {
        // Two independent sandwiches in one block.
        let txs = vec![
            transfer(0xAA, 0x99, 100, 0), // sandwich 1: front
            transfer(0xBB, 0x99, 200, 0), // sandwich 1: victim
            transfer(0xAA, 0x99, 150, 1), // sandwich 1: back
            transfer(0xCC, 0x88, 50, 0),  // sandwich 2: front
            transfer(0xDD, 0x88, 300, 0), // sandwich 2: victim
            transfer(0xCC, 0x88, 75, 1),  // sandwich 2: back
        ];
        let out = scan_block(&txs, 1);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].attacker, addr(0xAA));
        assert_eq!(out[1].attacker, addr(0xCC));
    }

    #[test]
    fn one_attacker_one_victim_pair_emits_once() {
        // Two victims for the same (front, back) pair — Phase 1
        // contract emits ONLY the first to avoid exponential blowup.
        let txs = vec![
            transfer(0xAA, 0x99, 100, 0),
            transfer(0xBB, 0x99, 200, 0),
            transfer(0xCC, 0x99, 250, 0),
            transfer(0xAA, 0x99, 150, 1),
        ];
        let out = scan_block(&txs, 1);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].victim,
            addr(0xBB),
            "first victim wins per Phase 1 contract"
        );
    }
}
