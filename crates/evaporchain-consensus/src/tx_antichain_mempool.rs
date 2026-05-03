//! Tx-level antichain mempool — Lane I.1, first concrete `BlockSource` impl
//! per Layer 4 of `DOCTRINE_PUNCH_LIST.md`.
//!
//! Slots into the seam defined in [`crate::mempool::BlockSource`] (Lane G.1).
//! Replaces the FIFO-style drain of [`crate::mempool::Mempool`] with a
//! proposal-time **antichain draw**: select transactions that are
//! mutually independent in the dependency partial order, so the
//! resulting block can be executed in maximum parallelism by Block-STM
//! without conflicts.
//!
//! ## What "antichain" means here
//!
//! Per `evaporchain-antichain-mempool` (block-level antichain) and
//! INVENTION_STACK.md §4.1 row 2, an *antichain* is a set whose elements
//! are pairwise *incomparable* in some partial order. Two transactions
//! are comparable iff they MUST be sequenced — for V1, that means they
//! share a sender (same-sender txs have a nonce ordering that the
//! executor enforces). Two txs from different senders are concurrent —
//! the chain can include both in either order.
//!
//! At proposal time the producer:
//!
//! 1. walks pending txs in descending priority order;
//! 2. greedily admits a tx iff its sender hasn't been seen in the
//!    growing antichain (otherwise the new tx is comparable to one
//!    already-included);
//! 3. stops at `n` admitted or when no more conflict-free txs remain.
//!
//! The output is a maximal antichain (within the priority cap) — the
//! highest-priority subset of the pending pool that can be executed in
//! parallel without conflict.
//!
//! ## V1 conflict heuristic — same-sender only
//!
//! For V1, "comparable" = "same sender." This captures the dominant
//! source of conflicts (sequential nonces from one account) and is
//! cheap to evaluate. Future versions (Lane I.2+) can refine the
//! heuristic to:
//!
//! - account read/write set overlap (full state-conflict graph);
//! - cross-shard message dependencies;
//! - contract storage-key collisions for `CallContract` txs.
//!
//! V1 ships the seam + the simple heuristic; the algorithm shape stays
//! the same for richer heuristics.
//!
//! ## Doctrine link
//!
//! INVENTION_STACK.md §4.1 row 2 — "Mempool *is* the partial order;
//! producer extends maximal antichains whose total energy clears a
//! threshold." This crate is the tx-level realisation; the
//! `evaporchain-antichain-mempool` crate is the block-level one
//! (LightCone DAG). Both shapes ship in V1.

use crate::mempool::{
    BlockSource, BASE_INCLUSION_ENERGY, MEV_INCLUSION_HALF_LIFE_BLOCKS,
};
use evaporchain_types::{energy_at_epoch, AccountAddress, Transaction};
use std::collections::HashSet;

/// Internal pending entry — pairs a transaction with its submit epoch
/// so priority-bonus computation matches the canonical [`Mempool`]
/// (Lane A.2 hint pipeline).
#[derive(Debug, Clone)]
struct PendingTx {
    tx: Transaction,
    submit_epoch: u64,
}

/// Antichain-aware tx mempool. Proposal-time draws return an antichain
/// (mutually independent txs) so Block-STM can run them in parallel.
///
/// Layout: `pending` is a `Vec<PendingTx>` kept in submission order. We
/// re-sort at draw time by descending priority, which keeps `submit`
/// O(1) and pays the O(n log n) cost only when a proposal is actually
/// being built. For typical mempool sizes this is the right tradeoff.
///
/// `current_epoch` is the chain's epoch counter, advanced via
/// [`BlockSource::set_epoch`]. It feeds the energy-decay priority so
/// late-arriving txs inherit the canonical [`BASE_INCLUSION_ENERGY`]
/// half-life rule.
#[derive(Debug, Default)]
pub struct TxAntichainMempool {
    pending: Vec<PendingTx>,
    current_epoch: u64,
}

impl TxAntichainMempool {
    /// Empty mempool at epoch 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// Recompute a tx's priority at `current_block`. Mirrors the formula
    /// [`crate::mempool::Mempool::take_with_priority`] uses internally:
    /// every pending tx is "born" with [`BASE_INCLUSION_ENERGY`] and
    /// decays per [`MEV_INCLUSION_HALF_LIFE_BLOCKS`] thereafter.
    fn priority_for(&self, p: &PendingTx, current_block: u64) -> u64 {
        let elapsed = current_block.saturating_sub(p.submit_epoch);
        energy_at_epoch(
            BASE_INCLUSION_ENERGY,
            MEV_INCLUSION_HALF_LIFE_BLOCKS,
            elapsed,
        )
    }
}

impl BlockSource for TxAntichainMempool {
    fn submit_priority(&mut self, tx: Transaction) -> bool {
        // Priority bookkeeping: stamp the tx's submit epoch so
        // proposal-time priority decays match the canonical Mempool.
        self.pending.push(PendingTx {
            tx,
            submit_epoch: self.current_epoch,
        });
        true
    }

    fn len(&self) -> usize {
        self.pending.len()
    }

    fn set_epoch(&mut self, epoch: u64) {
        self.current_epoch = epoch;
    }

    fn take_with_priority_sum_and_hints(
        &mut self,
        n: usize,
        current_block: u64,
    ) -> (Vec<Transaction>, u64, Vec<u64>) {
        if n == 0 || self.pending.is_empty() {
            return (Vec::new(), 0, Vec::new());
        }

        // Sort pending by descending priority + tie-break by submit_epoch
        // (older first — preserves FIFO-like fairness within same-priority
        // peers). Collect refs alongside indices so we can drain the
        // chosen entries from `self.pending` after the antichain walk.
        let mut indexed: Vec<(usize, u64)> = self
            .pending
            .iter()
            .enumerate()
            .map(|(i, p)| (i, self.priority_for(p, current_block)))
            .collect();
        indexed.sort_by(|a, b| {
            b.1.cmp(&a.1).then_with(|| {
                self.pending[a.0]
                    .submit_epoch
                    .cmp(&self.pending[b.0].submit_epoch)
            })
        });

        // Greedy antichain: walk highest-priority first, admit iff the
        // tx's sender hasn't been seen yet in the growing antichain.
        // Sender == None (e.g. Refresh tx) is non-conflicting by
        // construction — admit unconditionally up to the cap.
        let mut chosen_indices: Vec<usize> = Vec::with_capacity(n.min(indexed.len()));
        let mut seen_senders: HashSet<AccountAddress> = HashSet::new();
        let mut priority_sum: u64 = 0;
        let mut hints: Vec<u64> = Vec::with_capacity(n.min(indexed.len()));

        for (i, prio) in indexed {
            if chosen_indices.len() >= n {
                break;
            }
            let p = &self.pending[i];
            match p.tx.sender() {
                Some(addr) => {
                    if seen_senders.contains(addr) {
                        continue;
                    }
                    seen_senders.insert(*addr);
                }
                None => {
                    // No sender → no conflict possible. Admit.
                }
            }
            chosen_indices.push(i);
            priority_sum = priority_sum.saturating_add(prio);
            hints.push(p.submit_epoch);
        }

        // Drain chosen entries from `self.pending`. Sort indices
        // descending so `swap_remove` doesn't invalidate later indices.
        let mut chosen_indices_sorted = chosen_indices.clone();
        chosen_indices_sorted.sort_unstable_by(|a, b| b.cmp(a));
        let mut by_index: std::collections::HashMap<usize, Transaction> =
            std::collections::HashMap::new();
        for i in chosen_indices_sorted {
            let p = self.pending.swap_remove(i);
            by_index.insert(i, p.tx);
        }

        // Reassemble in the priority-descending order we computed
        // (chosen_indices is in admission order = priority-descending).
        let txs: Vec<Transaction> = chosen_indices
            .iter()
            .map(|i| {
                by_index
                    .remove(i)
                    .expect("chosen index must map to a drained tx")
            })
            .collect();

        (txs, priority_sum, hints)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_types::{RefreshTx, Transaction, TransferTx};

    fn transfer(from: u8, nonce: u64) -> Transaction {
        Transaction::Transfer(TransferTx {
            from: [from; 32],
            to: [99u8; 32],
            amount: 100,
            nonce,
            signature: None,
            public_key: None,
        })
    }

    #[test]
    fn empty_pool_returns_empty_triple() {
        let mut pool = TxAntichainMempool::new();
        let (txs, sum, hints) = pool.take_with_priority_sum_and_hints(10, 5);
        assert!(txs.is_empty());
        assert_eq!(sum, 0);
        assert!(hints.is_empty());
    }

    #[test]
    fn single_tx_admits_and_drains() {
        let mut pool = TxAntichainMempool::new();
        pool.set_epoch(5);
        assert!(pool.submit_priority(transfer(1, 0)));
        assert_eq!(pool.len(), 1);

        let (txs, sum, hints) = pool.take_with_priority_sum_and_hints(10, 5);
        assert_eq!(txs.len(), 1);
        // submit_epoch=5, current_block=5 → elapsed=0 → priority =
        // BASE_INCLUSION_ENERGY = 1_000_000.
        assert_eq!(sum, BASE_INCLUSION_ENERGY);
        assert_eq!(hints, vec![5]);
        assert!(pool.is_empty());
    }

    #[test]
    fn two_txs_same_sender_only_one_in_antichain() {
        // The defining test: two txs from the SAME sender are
        // comparable (sequential nonces), so the antichain admits
        // exactly one. Without the antichain rule, FIFO would admit
        // both — and Block-STM would have to serialise them anyway.
        let mut pool = TxAntichainMempool::new();
        pool.set_epoch(5);
        pool.submit_priority(transfer(1, 0));
        pool.submit_priority(transfer(1, 1));
        assert_eq!(pool.len(), 2);

        let (txs, _sum, hints) = pool.take_with_priority_sum_and_hints(10, 5);
        assert_eq!(txs.len(), 1, "antichain must drop conflicting tx");
        assert_eq!(hints.len(), 1);
        // The other tx remains in the pool for the next proposal.
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn two_txs_different_senders_both_admitted() {
        let mut pool = TxAntichainMempool::new();
        pool.set_epoch(5);
        pool.submit_priority(transfer(1, 0));
        pool.submit_priority(transfer(2, 0));

        let (txs, sum, hints) = pool.take_with_priority_sum_and_hints(10, 5);
        assert_eq!(txs.len(), 2, "different senders are incomparable");
        // Both at submit_epoch=5, current=5 → both at full priority.
        assert_eq!(sum, BASE_INCLUSION_ENERGY * 2);
        assert_eq!(hints.len(), 2);
        assert!(pool.is_empty());
    }

    #[test]
    fn n_cap_is_honoured() {
        let mut pool = TxAntichainMempool::new();
        pool.set_epoch(5);
        for sender in 1u8..=5 {
            pool.submit_priority(transfer(sender, 0));
        }
        assert_eq!(pool.len(), 5);

        let (txs, _, hints) = pool.take_with_priority_sum_and_hints(3, 5);
        assert_eq!(txs.len(), 3);
        assert_eq!(hints.len(), 3);
        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn priority_descending_order_preserved() {
        // Older txs decay faster → newer txs have higher priority.
        // The antichain draw must surface higher-priority first.
        let mut pool = TxAntichainMempool::new();
        // Older tx (sender 1) — submit at epoch 0 — will decay heavily.
        pool.set_epoch(0);
        pool.submit_priority(transfer(1, 0));
        // Newer tx (sender 2) — submit at epoch 100 — full priority.
        pool.set_epoch(100);
        pool.submit_priority(transfer(2, 0));

        let (txs, _, _hints) = pool.take_with_priority_sum_and_hints(10, 100);
        assert_eq!(txs.len(), 2);
        // First-out should be the newer (higher-priority) sender 2.
        assert_eq!(txs[0].sender(), Some(&[2u8; 32]));
        assert_eq!(txs[1].sender(), Some(&[1u8; 32]));
    }

    #[test]
    fn refresh_tx_no_sender_admits_unconditionally() {
        // Refresh txs have no sender → can never conflict. They should
        // admit alongside any other tx without affecting the antichain
        // sender-set.
        let mut pool = TxAntichainMempool::new();
        pool.set_epoch(5);
        pool.submit_priority(transfer(1, 0));
        pool.submit_priority(Transaction::Refresh(RefreshTx {
            object_id: [42u8; 32],
            energy_deposit: 1,
            signature: None,
            public_key: None,
        }));
        pool.submit_priority(transfer(1, 1)); // same-sender as first → conflict

        let (txs, _, _) = pool.take_with_priority_sum_and_hints(10, 5);
        // Expected: transfer(1,0) + Refresh(...). transfer(1,1) drops
        // because its sender [1; 32] is already seen.
        assert_eq!(txs.len(), 2);
    }

    #[test]
    fn block_source_dyn_dispatch() {
        // Lock in the seam: behind `&mut dyn BlockSource`, behaviour
        // matches concrete calls. This is what proves G.1 + I.1
        // composition works at the consensus seam.
        let mut pool = TxAntichainMempool::new();
        let bs: &mut dyn BlockSource = &mut pool;
        bs.set_epoch(5);
        assert!(bs.submit_priority(transfer(1, 0)));
        assert!(bs.submit_priority(transfer(2, 0)));
        let (txs, _, hints) = bs.take_with_priority_sum_and_hints(10, 5);
        assert_eq!(txs.len(), 2);
        assert_eq!(hints.len(), 2);
        assert!(bs.is_empty());
    }
}
