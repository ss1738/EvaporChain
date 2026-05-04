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
    /// Placeholder confidence in Phase 1; always 1.0. Phase 4
    /// replaces with a real score so low-confidence events can be
    /// filtered before settlement.
    pub confidence_score: f64,
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

    // Project txs to (idx, from, to, amount) tuples for Transfer
    // variants only — every other tx variant is skipped in Phase 1.
    let transfers: Vec<(usize, AccountAddress, AccountAddress, u64)> = txs
        .iter()
        .enumerate()
        .filter_map(|(i, tx)| match tx {
            Transaction::Transfer(t) => Some((i, t.from, t.to, t.amount)),
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
            let (idx_i, from_i, to_i, amt_i) = transfers[ai];
            let (idx_k, from_k, to_k, amt_k) = transfers[ak];
            if from_i != from_k {
                continue;
            }
            if to_i != to_k {
                // Strict shape: attacker's pre/post both target the
                // same account. Loosened in Phase 6.3.
                continue;
            }
            // Find a victim slot between ai and ak.
            for aj in (ai + 1)..ak {
                let (idx_j, from_j, to_j, _amt_j) = transfers[aj];
                if from_j == from_i {
                    // Self-MEV — skip per Phase 4.3 anti-gaming.
                    continue;
                }
                if to_j != to_i {
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
                    confidence_score: 1.0,
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
        })
    }

    #[test]
    fn empty_block_yields_no_observations() {
        let out = scan_block(&[], 1);
        assert!(out.is_empty());
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
        assert_eq!(obs.confidence_score, 1.0);
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
        assert_eq!(out.len(), 1, "refresh in the middle should not break detection");
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
        assert_eq!(out[0].victim, addr(0xBB), "first victim wins per Phase 1 contract");
    }
}
