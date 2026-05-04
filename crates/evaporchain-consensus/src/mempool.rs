use evaporchain_crypto::signatures::{HybridVerifier, Verifier};
use evaporchain_types::{energy_at_epoch, Transaction};
use std::collections::{HashMap, HashSet, VecDeque};

// Re-export the canonical priority constants from `evaporchain-types`
// (Lane A.3 moved them out of mempool so the execution layer can read
// the same constants without a circular consensus → execution dep).
pub use evaporchain_types::{BASE_INCLUSION_ENERGY, MEV_INCLUSION_HALF_LIFE_BLOCKS};

/// Source of transactions for block proposals — Lane G.1 substrate seam.
///
/// `TendermintConsensus` currently holds a concrete [`Mempool`]. This trait
/// names the abstract contract so alternative implementations (antichain
/// mempool, MEV-aware reorderer, k-of-n shard router) can plug in without
/// changing consensus code. The migration of `TendermintConsensus` to
/// `Box<dyn BlockSource>` is Lane G.2 (separate commit) — this commit just
/// lands the trait + the blanket impl on `Mempool` so other impls can be
/// written against a stable seam.
///
/// Method set is the minimum needed by `tendermint.rs` today:
///
/// - [`submit_priority`](BlockSource::submit_priority): admit a tx with
///   energy-decay priority bookkeeping. The "priority" naming is honoured
///   by the existing `Mempool` impl but the contract is "admit-or-reject."
///   Future impls (e.g. FIFO antichain) may ignore priority and just
///   queue; the bool return = "did we admit it?"
/// - [`len`](BlockSource::len) / [`is_empty`](BlockSource::is_empty):
///   pending count for status RPCs.
/// - [`set_epoch`](BlockSource::set_epoch): advance the TTL window so
///   stale txs evict; called by consensus on every committed block.
/// - [`take_with_priority_sum_and_hints`](BlockSource::take_with_priority_sum_and_hints):
///   the proposal-time draw. Returns `(txs, priority_sum,
///   submit_epoch_hints)` so `create_proposal` can stamp the
///   priority-bonus and the on-the-wire submit-epoch hints (Lane A.2/A.3).
///   Hints are `Vec<u64>` index-parallel to `txs`. Non-priority impls
///   satisfy the contract by returning `(txs, 0, vec![current_block; txs.len()])`
///   so per-tx priority degrades to a uniform 1-block elapsed window.
///
/// `Send + Sync` so consensus engines can hold this behind locks; the
/// existing `Mempool` is already `Send + Sync` by virtue of its fields.
pub trait BlockSource: Send + Sync {
    /// Admit a transaction to the source. Returns `true` if accepted,
    /// `false` if rejected (size cap, duplicate, malformed, etc.).
    fn submit_priority(&mut self, tx: Transaction) -> bool;

    /// Number of pending transactions.
    fn len(&self) -> usize;

    /// True iff `len() == 0`.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Advance the source's epoch counter so age-based eviction and
    /// energy-decay bookkeeping stay current. Called by consensus on
    /// every committed block.
    fn set_epoch(&mut self, epoch: u64);

    /// Draw up to `n` transactions for inclusion in a proposal at
    /// `current_block`. Returns the txs, the aggregate priority sum
    /// for the producer's reward bonus, and per-tx submit-epoch hints
    /// (parallel to `txs`). Hints carry on-the-wire so every follower
    /// can re-derive the priority bonus without consulting its local
    /// mempool — see Lane A.2.
    fn take_with_priority_sum_and_hints(
        &mut self,
        n: usize,
        current_block: u64,
    ) -> (Vec<Transaction>, u64, Vec<u64>);
}

/// Maximum number of transactions in the mempool (DoS protection).
const MAX_MEMPOOL_SIZE: usize = 10_000;

/// Maximum transaction data size in bytes (prevents oversized payloads).
const MAX_TX_SIZE_BYTES: usize = 128 * 1024; // 128 KB

/// Maximum aggregate bytes across all pending mempool transactions
/// (DoS protection — caps total memory regardless of per-tx size).
/// Closes punch-list 5: prior to this, per-tx and per-account caps were
/// enforced but the global byte total was tracked-not-rejected, so an
/// adversary submitting many medium-sized txs could exceed the implicit
/// 10K × 128KB = 1.28GB ceiling. 256 MiB is well above realistic
/// throughput needs for an L1 mempool.
const MAX_MEMPOOL_BYTES: usize = 256 * 1024 * 1024; // 256 MiB

/// Maximum transactions per account in the mempool (anti-spam).
const MAX_TXS_PER_ACCOUNT: usize = 64;

/// Maximum age of a transaction in epochs before it's evicted.
const MAX_TX_AGE_EPOCHS: u64 = 256;

/// Thread-safe pending transaction pool with DoS protection.
///
/// Enforces size limits, per-account caps, TTL eviction, and basic validation.
pub struct Mempool {
    pending: VecDeque<Transaction>,
    /// BLAKE3 hashes of transactions currently in the pool (dedup).
    seen: HashSet<[u8; 32]>,
    /// Total bytes of serialized transactions in the pool (approximate).
    total_bytes: usize,
    /// Maximum pool size.
    max_size: usize,
    /// Transactions rejected due to pool being full.
    rejected_count: u64,
    /// Transactions rejected as duplicates.
    duplicate_count: u64,
    /// Per-account transaction count (anti-spam).
    account_tx_count: HashMap<[u8; 32], usize>,
    /// Epoch when each transaction was submitted (for TTL eviction).
    tx_submit_epoch: HashMap<[u8; 32], u64>,
    /// Current chain epoch (updated on each block commit).
    current_epoch: u64,
    /// Verify signatures before accepting transactions.
    verify_signatures: bool,
    /// Chain ID for signing message domain separation (cross-chain replay protection).
    chain_id: String,
}

impl Mempool {
    pub fn new() -> Self {
        Self {
            pending: VecDeque::new(),
            seen: HashSet::new(),
            total_bytes: 0,
            max_size: MAX_MEMPOOL_SIZE,
            rejected_count: 0,
            duplicate_count: 0,
            account_tx_count: HashMap::new(),
            tx_submit_epoch: HashMap::new(),
            current_epoch: 0,
            verify_signatures: false,
            chain_id: String::new(),
        }
    }

    /// Create a mempool with a custom size limit.
    pub fn with_max_size(max_size: usize) -> Self {
        Self {
            pending: VecDeque::new(),
            seen: HashSet::new(),
            total_bytes: 0,
            max_size,
            rejected_count: 0,
            duplicate_count: 0,
            account_tx_count: HashMap::new(),
            tx_submit_epoch: HashMap::new(),
            current_epoch: 0,
            verify_signatures: false,
            chain_id: String::new(),
        }
    }

    /// Set the chain ID for signing message domain separation.
    pub fn set_chain_id(&mut self, chain_id: String) {
        self.chain_id = chain_id;
    }

    /// Enable signature verification on transaction submission.
    pub fn enable_sig_verification(&mut self) {
        self.verify_signatures = true;
    }

    /// Add a transaction to the pool. Returns false if rejected (duplicate, pool full, oversized,
    /// or per-account limit exceeded).
    pub fn submit(&mut self, tx: Transaction) -> bool {
        if !self.validate_submission(&tx) {
            return false;
        }
        let hash = tx.tx_hash();
        self.track_account_add(&tx);
        self.tx_submit_epoch.insert(hash, self.current_epoch);
        self.seen.insert(hash);
        self.total_bytes += Self::estimate_tx_size(&tx);
        self.pending.push_back(tx);
        true
    }

    /// Add a high-priority transaction to the FRONT of the pool.
    /// Used for API-submitted transactions that should be included before demo txs.
    pub fn submit_priority(&mut self, tx: Transaction) -> bool {
        if !self.validate_submission(&tx) {
            return false;
        }
        let hash = tx.tx_hash();
        self.track_account_add(&tx);
        self.tx_submit_epoch.insert(hash, self.current_epoch);
        self.seen.insert(hash);
        self.total_bytes += Self::estimate_tx_size(&tx);
        self.pending.push_front(tx);
        true
    }

    fn validate_submission(&mut self, tx: &Transaction) -> bool {
        let hash = tx.tx_hash();
        if self.seen.contains(&hash) {
            self.duplicate_count += 1;
            return false;
        }
        if self.pending.len() >= self.max_size {
            self.rejected_count += 1;
            return false;
        }
        // Phase 4.3 (2026-05-03) — NMT namespace-0 reject at admission.
        // Audit K-13: ns=0 is in active production use by the
        // tendermint proposal builder for "core transactions"
        // (non-Blob txs framed under ns=0). User-submitted BlobTx
        // with namespace_id=0 collides with that frame and would
        // forge a system-namespace blob. Execution already rejects
        // at `lib.rs:2597`, but admission-side rejection
        // additionally prevents adversaries from getting such txs
        // accepted into the pool / proposed into a block. The walk-
        // back at `evaporchain-da/src/namespace.rs:44-52` notes
        // exactly this: "If user-submitted BlobTx with namespace_id=0
        // needs to be rejected, gate it at mempool admission, not at
        // NMT construction."
        if let Transaction::Blob(blob) = tx {
            if blob.namespace_id == 0 {
                self.rejected_count += 1;
                return false;
            }
        }
        let tx_size = Self::estimate_tx_size(tx);
        if tx_size > MAX_TX_SIZE_BYTES {
            self.rejected_count += 1;
            return false;
        }
        // Punch-list 5: global byte-cap admission check. Reject when the
        // pool's total serialized bytes plus this tx would exceed the
        // configured cap. Prevents an adversary from filling the mempool
        // with sub-128KB txs that individually pass the per-tx check but
        // collectively blow past the implicit memory ceiling.
        if self.total_bytes.saturating_add(tx_size) > MAX_MEMPOOL_BYTES {
            self.rejected_count += 1;
            return false;
        }
        if let Some(sender) = tx.sender() {
            let count = self.account_tx_count.get(sender).copied().unwrap_or(0);
            if count >= MAX_TXS_PER_ACCOUNT {
                self.rejected_count += 1;
                return false;
            }
        }
        if self.verify_signatures
            && !matches!(
                tx,
                Transaction::Unshield(_) | Transaction::PrivateTransfer(_)
            )
        {
            if let (Some(sig), Some(pk)) = (tx.signature(), tx.public_key()) {
                let msg = tx.signing_message(&self.chain_id);
                if !HybridVerifier::verify(&msg, sig, pk) {
                    self.rejected_count += 1;
                    return false;
                }
            } else if tx.signature().is_none() && tx.sender().is_some() {
                self.rejected_count += 1;
                return false;
            }
        }
        true
    }

    fn track_account_add(&mut self, tx: &Transaction) {
        if let Some(sender) = tx.sender() {
            *self.account_tx_count.entry(*sender).or_insert(0) += 1;
        }
    }

    fn track_account_remove(&mut self, tx: &Transaction) {
        if let Some(sender) = tx.sender() {
            if let Some(count) = self.account_tx_count.get_mut(sender) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    self.account_tx_count.remove(sender);
                }
            }
        }
    }

    /// Update the current epoch and evict expired transactions.
    pub fn set_epoch(&mut self, epoch: u64) {
        self.current_epoch = epoch;
        self.evict_expired();
    }

    /// Remove transactions older than MAX_TX_AGE_EPOCHS.
    fn evict_expired(&mut self) {
        if self.current_epoch < MAX_TX_AGE_EPOCHS {
            return;
        }
        let cutoff = self.current_epoch - MAX_TX_AGE_EPOCHS;
        let mut evicted = Vec::new();
        self.pending.retain(|tx| {
            let hash = tx.tx_hash();
            let submit_epoch = self.tx_submit_epoch.get(&hash).copied().unwrap_or(0);
            if submit_epoch < cutoff {
                evicted.push((hash, tx.clone()));
                false
            } else {
                true
            }
        });
        for (hash, tx) in &evicted {
            self.seen.remove(hash);
            self.tx_submit_epoch.remove(hash);
            self.track_account_remove(tx);
        }
        self.total_bytes = self.pending.iter().map(Self::estimate_tx_size).sum();
    }

    /// Drain all pending transactions for inclusion in the next block.
    pub fn drain(&mut self) -> Vec<Transaction> {
        self.total_bytes = 0;
        self.seen.clear();
        self.account_tx_count.clear();
        self.tx_submit_epoch.clear();
        self.pending.drain(..).collect()
    }

    /// Take up to `n` transactions with nonce-aware ordering.
    /// Groups by sender, sorts by nonce within each group, then interleaves
    /// by sender hash for determinism across validators.
    pub fn take(&mut self, n: usize) -> Vec<Transaction> {
        let all: Vec<Transaction> = self.pending.drain(..).collect();
        let mut with_hash: Vec<([u8; 32], Transaction)> =
            all.into_iter().map(|tx| (tx.tx_hash(), tx)).collect();
        Self::sort_nonce_aware(&mut with_hash);

        let take_count = n.min(with_hash.len());
        let mut taken = Vec::with_capacity(take_count);
        let mut remaining = VecDeque::new();

        for (i, (h, tx)) in with_hash.into_iter().enumerate() {
            if i < take_count {
                self.seen.remove(&h);
                self.tx_submit_epoch.remove(&h);
                self.track_account_remove(&tx);
                taken.push(tx);
            } else {
                remaining.push_back(tx);
            }
        }

        self.pending = remaining;
        self.total_bytes = self.pending.iter().map(Self::estimate_tx_size).sum();
        taken
    }

    /// Take up to `n` transactions with nonce-aware ordering.
    /// Returns transactions paired with their hashes for callers that need them.
    /// Take up to `n` transactions ordered by **energy-stamped inclusion
    /// priority** (Phase 2 of the MEV-resistance proposal in
    /// `research/proposals/energy-stamped-mev-resistance.md`).
    ///
    /// Each tx's priority is `energy_at_epoch(BASE_INCLUSION_ENERGY,
    /// MEV_INCLUSION_HALF_LIFE_BLOCKS, current_block - submit_epoch)`.
    /// This means:
    /// - Old txs naturally fall down the queue (their priority decays).
    /// - The proposer's reward incentive is to include high-priority
    ///   txs FAST. Holding a tx to insert your own first costs
    ///   `(1 - 0.5^(1/half_life))` of that tx's reward weight (~16% per
    ///   block at half_life=4) — making sandwich/frontrun attacks
    ///   economically unprofitable when gross < decay cost.
    ///
    /// Tie-breaking falls back to the existing nonce-aware ordering so
    /// per-sender nonce sequences stay valid.
    ///
    /// Honest validators get the same proposal regardless of whether
    /// they call `take` or `take_with_priority` because all txs are
    /// included before the next block; the ordering only matters when
    /// `pending.len() > n` (back-pressure scenario) or when reward
    /// weighting kicks in.
    pub fn take_with_priority(&mut self, n: usize, current_block: u64) -> Vec<Transaction> {
        self.take_with_priority_and_sum(n, current_block).0
    }

    /// As [`take_with_priority`], but ALSO returns the cumulative inclusion
    /// priority of the txs returned. Phase-1.5 MEV-resistance proposer-reward
    /// weighting (research/proposals/energy-stamped-mev-resistance.md):
    /// the proposer's reward bonus is proportional to `Σ priority_at_inclusion(tx)`.
    /// The sum is computed once during the same sort-and-take pass — no extra
    /// state required from the caller — and is `u64::saturating_add` accumulated
    /// so a flood of high-priority txs can never overflow.
    pub fn take_with_priority_and_sum(
        &mut self,
        n: usize,
        current_block: u64,
    ) -> (Vec<Transaction>, u64) {
        let (txs, sum, _hints) = self.take_with_priority_sum_and_hints(n, current_block);
        (txs, sum)
    }

    /// As [`take_with_priority_and_sum`] but ALSO returns a parallel
    /// `Vec<u64>` of per-tx submit-epoch hints — one entry per returned
    /// tx, equal to the epoch the mempool recorded when the tx was
    /// submitted (or `current_block` for txs whose submit_epoch was
    /// already evicted by an earlier take).
    ///
    /// Phase-1/Lane-A.2 MEV-resistance wire-format: the proposer
    /// stamps these onto `Block.submit_epoch_hints` so every follower
    /// validator computes the SAME priority for each tx (currently
    /// only the proposer can — followers don't have the original
    /// submit time). Once hints are on the wire, the priority bonus
    /// can auto-fire from `execute_block` consistently across the
    /// cluster (Lane A.3).
    pub fn take_with_priority_sum_and_hints(
        &mut self,
        n: usize,
        current_block: u64,
    ) -> (Vec<Transaction>, u64, Vec<u64>) {
        let all: Vec<Transaction> = self.pending.drain(..).collect();
        let mut with_meta: Vec<(u64, u64, [u8; 32], Transaction)> = all
            .into_iter()
            .map(|tx| {
                let hash = tx.tx_hash();
                let submit = self
                    .tx_submit_epoch
                    .get(&hash)
                    .copied()
                    .unwrap_or(current_block);
                let elapsed = current_block.saturating_sub(submit);
                let priority = energy_at_epoch(
                    BASE_INCLUSION_ENERGY,
                    MEV_INCLUSION_HALF_LIFE_BLOCKS,
                    elapsed,
                );
                (priority, submit, hash, tx)
            })
            .collect();
        // Sort by (priority desc, sender, nonce, hash) — priority dominates,
        // nonce-aware tie-break preserves per-sender sequencing.
        with_meta.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| {
                    let sa = a.3.sender().copied().unwrap_or([0xff; 32]);
                    let sb = b.3.sender().copied().unwrap_or([0xff; 32]);
                    sa.cmp(&sb)
                })
                .then_with(|| {
                    let na = a.3.nonce().unwrap_or(0);
                    let nb = b.3.nonce().unwrap_or(0);
                    na.cmp(&nb)
                })
                .then_with(|| a.2.cmp(&b.2))
        });

        let take_count = n.min(with_meta.len());
        let mut taken = Vec::with_capacity(take_count);
        let mut hints = Vec::with_capacity(take_count);
        let mut remaining = VecDeque::new();
        let mut priority_sum: u64 = 0;
        for (i, (priority, submit, h, tx)) in with_meta.into_iter().enumerate() {
            if i < take_count {
                self.seen.remove(&h);
                self.tx_submit_epoch.remove(&h);
                self.track_account_remove(&tx);
                taken.push(tx);
                hints.push(submit);
                priority_sum = priority_sum.saturating_add(priority);
            } else {
                remaining.push_back(tx);
            }
        }
        self.pending = remaining;
        self.total_bytes = self.pending.iter().map(Self::estimate_tx_size).sum();
        (taken, priority_sum, hints)
    }

    pub fn take_with_hashes(&mut self, n: usize) -> Vec<([u8; 32], Transaction)> {
        let all: Vec<Transaction> = self.pending.drain(..).collect();
        let mut with_hash: Vec<([u8; 32], Transaction)> =
            all.into_iter().map(|tx| (tx.tx_hash(), tx)).collect();
        Self::sort_nonce_aware(&mut with_hash);

        let take_count = n.min(with_hash.len());
        let (taken, rest) = with_hash.split_at(take_count);
        let result: Vec<([u8; 32], Transaction)> = taken.to_vec();

        self.pending = rest.iter().map(|(_, tx)| tx.clone()).collect();
        for (h, tx) in &result {
            self.seen.remove(h);
            self.tx_submit_epoch.remove(h);
            self.track_account_remove(tx);
        }
        self.total_bytes = self.pending.iter().map(Self::estimate_tx_size).sum();
        result
    }

    /// Take transactions up to a gas limit with nonce-aware ordering.
    pub fn take_with_gas_limit(&mut self, max_txs: usize, gas_limit: u64) -> Vec<Transaction> {
        let all: Vec<Transaction> = self.pending.drain(..).collect();
        let mut with_hash: Vec<([u8; 32], Transaction)> =
            all.into_iter().map(|tx| (tx.tx_hash(), tx)).collect();
        Self::sort_nonce_aware(&mut with_hash);

        let mut taken = Vec::new();
        let mut remaining = VecDeque::new();
        let mut gas_used = 0u64;

        for (hash, tx) in with_hash {
            let tx_gas = Self::estimate_tx_gas(&tx);
            if taken.len() < max_txs && gas_used.saturating_add(tx_gas) <= gas_limit {
                self.seen.remove(&hash);
                self.tx_submit_epoch.remove(&hash);
                self.track_account_remove(&tx);
                gas_used += tx_gas;
                taken.push(tx);
            } else {
                remaining.push_back(tx);
            }
        }

        self.pending = remaining;
        self.total_bytes = self.pending.iter().map(Self::estimate_tx_size).sum();
        taken
    }

    /// Sort transactions by (sender_hash, nonce, tx_hash) for deterministic
    /// nonce-respecting ordering. Ensures lower nonces execute first per account.
    fn sort_nonce_aware(txs: &mut [([u8; 32], Transaction)]) {
        txs.sort_by(|a, b| {
            let sender_a = a.1.sender().copied().unwrap_or([0xff; 32]);
            let sender_b = b.1.sender().copied().unwrap_or([0xff; 32]);
            sender_a
                .cmp(&sender_b)
                .then_with(|| {
                    let nonce_a = a.1.nonce().unwrap_or(0);
                    let nonce_b = b.1.nonce().unwrap_or(0);
                    nonce_a.cmp(&nonce_b)
                })
                .then_with(|| a.0.cmp(&b.0))
        });
    }

    fn estimate_tx_gas(tx: &Transaction) -> u64 {
        match tx {
            Transaction::Transfer(_) => 21_000,
            Transaction::CreateObject(t) => 50_000 + 200 * t.data.len() as u64,
            Transaction::Refresh(_) => 30_000,
            Transaction::DeployContract(_) => 100_000,
            Transaction::CallContract(_) => 40_000,
            Transaction::DeployScript(_) => 150_000,
            Transaction::CallScript(_) => 50_000,
            Transaction::ValidatorStake(_) => 50_000,
            Transaction::ValidatorExit(_) => 30_000,
            Transaction::ValidatorClaimStake(_) => 30_000,
            Transaction::Shield(_) => 60_000,
            Transaction::Unshield(_) => 80_000,
            Transaction::PrivateTransfer(ptx) => {
                100_000
                    + 20_000 * ptx.input_nullifiers.len() as u64
                    + 15_000 * ptx.output_commitments.len() as u64
            }
            Transaction::Deferred(dtx) => 75_000 + 5_000 * dtx.guards.len() as u64,
            Transaction::Blob(tx) => 50_000 + 10 * tx.data.len() as u64,
            Transaction::Governance(_) => 25_000,
            Transaction::MultiSig(_) => 50_000,
            Transaction::UserOp(tx) => 30_000 + 16 * tx.call_data.len() as u64,
            Transaction::UpgradeContract(tx) => 100_000 + 200 * tx.new_bytecode.len() as u64,
            Transaction::Delegate(_) => 40_000,
            Transaction::Undelegate(_) => 40_000,
            Transaction::RotateValidatorKey(_) => 80_000,
            Transaction::ClaimDelegation(_) => 30_000,
            // Refund is protocol-issued; transfer-equivalent gas.
            Transaction::Refund(_) => 21_000,
        }
    }

    /// Number of pending transactions.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Whether the mempool is empty.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Read-only view of pending transactions (for dedup checks).
    pub fn pending(&self) -> &VecDeque<Transaction> {
        &self.pending
    }

    /// Approximate total bytes in the mempool.
    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    /// Number of transactions rejected due to limits.
    pub fn rejected_count(&self) -> u64 {
        self.rejected_count
    }

    /// Number of transactions rejected as duplicates.
    pub fn duplicate_count(&self) -> u64 {
        self.duplicate_count
    }

    /// Check if a transaction hash is already in the mempool.
    pub fn contains_hash(&self, hash: &[u8; 32]) -> bool {
        self.seen.contains(hash)
    }

    /// Estimate the serialized size of a transaction.
    fn estimate_tx_size(tx: &Transaction) -> usize {
        match tx {
            Transaction::Transfer(t) => {
                64 + 8 + 8 // addresses + amount + nonce
                    + t.signature.as_ref().map_or(0, |s| s.len())
                    + t.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::CreateObject(t) => {
                32 + 32 + 8 + 8 + t.data.len() // creator + id + energy + hl + data
                    + t.signature.as_ref().map_or(0, |s| s.len())
                    + t.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::Refresh(t) => {
                32 + 8 // object_id + energy
                    + t.signature.as_ref().map_or(0, |s| s.len())
                    + t.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::DeployContract(t) => {
                32 + t.template.len()
                    + t.init_args.len()
                    + 16
                    + t.signature.as_ref().map_or(0, |s| s.len())
                    + t.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::CallContract(t) => {
                32 + 32
                    + t.method.len()
                    + t.args.len()
                    + 16
                    + t.signature.as_ref().map_or(0, |s| s.len())
                    + t.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::DeployScript(t) => {
                32 + t.source_code.len()
                    + 16
                    + t.signature.as_ref().map_or(0, |s| s.len())
                    + t.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::CallScript(t) => {
                32 + 8
                    + t.method.len()
                    + t.args.len()
                    + 16
                    + t.signature.as_ref().map_or(0, |s| s.len())
                    + t.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::ValidatorStake(t) => {
                32 + 8
                    + 8
                    + 8
                    + t.bls_public_key.as_ref().map_or(0, |k| k.len())
                    + t.signature.as_ref().map_or(0, |s| s.len())
                    + t.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::ValidatorExit(t) => {
                32 + 8
                    + 8
                    + t.signature.as_ref().map_or(0, |s| s.len())
                    + t.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::ValidatorClaimStake(t) => {
                32 + 8
                    + 8
                    + t.signature.as_ref().map_or(0, |s| s.len())
                    + t.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::Shield(t) => {
                32 + 8 + 8 + 32 + 32 + 8 // from + amount + nonce + owner_hash + blinding + half_life
                    + t.signature.as_ref().map_or(0, |s| s.len())
                    + t.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::Unshield(t) => {
                32 + 8 + 32 + 32 // to + amount + anchor + balance_binding
                    + t.input_nullifiers.len() * 32
                    + t.change_commitments.len() * 32
            }
            Transaction::PrivateTransfer(t) => {
                32 + 32 + 8 // anchor + balance_binding + fee
                    + t.input_nullifiers.len() * 32
                    + t.output_commitments.len() * 32
            }
            Transaction::Deferred(dtx) => {
                32 + 8 + 8 + dtx.guards.len() * 50 + dtx.inner_tx_bytes.len()
            }
            Transaction::Blob(tx) => {
                32 + tx.data.len()
                    + 8
                    + 8
                    + tx.signature.as_ref().map_or(0, |s| s.len())
                    + tx.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::Governance(tx) => {
                32 + 8
                    + 64
                    + tx.signature.as_ref().map_or(0, |s| s.len())
                    + tx.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::MultiSig(tx) => {
                32 + 1
                    + 8
                    + tx.signers.len() * 32
                    + tx.inner_tx_bytes.len()
                    + tx.signatures.len() * 64
            }
            Transaction::UserOp(tx) => {
                32 + 8
                    + 8
                    + tx.call_data.len()
                    + tx.signature.as_ref().map_or(0, |s| s.len())
                    + tx.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::UpgradeContract(tx) => {
                32 + 8
                    + 8
                    + tx.new_bytecode.len()
                    + tx.signature.as_ref().map_or(0, |s| s.len())
                    + tx.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::Delegate(tx) => {
                32 + 8
                    + 8
                    + 8
                    + tx.signature.as_ref().map_or(0, |s| s.len())
                    + tx.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::Undelegate(tx) => {
                32 + 8
                    + 8
                    + 8
                    + tx.signature.as_ref().map_or(0, |s| s.len())
                    + tx.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::RotateValidatorKey(tx) => {
                32 + 8
                    + 8
                    + 8
                    + tx.new_bls_public_key.len()
                    + tx.bls_pop_old.len()
                    + tx.bls_pop_new.len()
                    + tx.signature.as_ref().map_or(0, |s| s.len())
                    + tx.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::ClaimDelegation(tx) => {
                32 + 8
                    + 8
                    + tx.signature.as_ref().map_or(0, |s| s.len())
                    + tx.public_key.as_ref().map_or(0, |p| p.len())
            }
            // Refund: attacker (32) + victim (32) + amount (8) + reason hash (32).
            Transaction::Refund(_) => 32 + 32 + 8 + 32,
        }
    }
}

impl Default for Mempool {
    fn default() -> Self {
        Self::new()
    }
}

/// Lane I.5 helper: same-sender antichain projection over a
/// `(candidates, hints)` slice. Used by the proposal-time draw site at
/// `tendermint.rs::create_proposal` when the `block_source_mode`
/// governance key is `"antichain"`. Extracted as a free function so it
/// can be proptest'd in isolation.
///
/// Returns `(kept_txs, kept_hints, dropped_txs)`. Inputs MUST be
/// index-parallel (hints[i] is the submit-epoch for txs[i]); panics in
/// debug builds if they're not. The function preserves priority order
/// (kept[0] is the highest-priority kept tx) because the input is
/// already in priority-desc order from the FIFO draw.
///
/// Conflict heuristic: V1 same-sender = comparable. Mirrors
/// `crate::tx_antichain_mempool::TxAntichainMempool` exactly.
///
/// Public-ish (`pub(crate)`) so the consensus crate can use it without
/// re-exposing it as part of the public Mempool API.
pub(crate) fn antichain_project(
    candidates: Vec<Transaction>,
    hints: Vec<u64>,
) -> (Vec<Transaction>, Vec<u64>, Vec<Transaction>) {
    debug_assert_eq!(
        candidates.len(),
        hints.len(),
        "antichain_project: candidates and hints must be index-parallel"
    );
    let mut seen: HashSet<[u8; 32]> = HashSet::new();
    let mut kept_txs: Vec<Transaction> = Vec::with_capacity(candidates.len());
    let mut kept_hints: Vec<u64> = Vec::with_capacity(hints.len());
    let mut dropped: Vec<Transaction> = Vec::new();
    for (tx, hint) in candidates.into_iter().zip(hints.into_iter()) {
        match tx.sender() {
            Some(addr) => {
                if seen.insert(*addr) {
                    kept_txs.push(tx);
                    kept_hints.push(hint);
                } else {
                    dropped.push(tx);
                }
            }
            None => {
                kept_txs.push(tx);
                kept_hints.push(hint);
            }
        }
    }
    (kept_txs, kept_hints, dropped)
}

/// Lane G.1 blanket impl. `Mempool` already exposes every method the
/// trait names — this is a pure delegation layer so callers can swap in
/// alternative `BlockSource` impls without code churn. No behaviour
/// change. The migration of `TendermintConsensus.mempool` from
/// `Mempool` to `Box<dyn BlockSource>` is Lane G.2.
impl BlockSource for Mempool {
    fn submit_priority(&mut self, tx: Transaction) -> bool {
        Mempool::submit_priority(self, tx)
    }

    fn len(&self) -> usize {
        Mempool::len(self)
    }

    fn set_epoch(&mut self, epoch: u64) {
        Mempool::set_epoch(self, epoch)
    }

    fn take_with_priority_sum_and_hints(
        &mut self,
        n: usize,
        current_block: u64,
    ) -> (Vec<Transaction>, u64, Vec<u64>) {
        Mempool::take_with_priority_sum_and_hints(self, n, current_block)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_types::{BlobTx, TransferTx};

    fn dummy_tx_with_nonce(nonce: u64) -> Transaction {
        Transaction::Transfer(TransferTx {
            from: [1u8; 32],
            to: [2u8; 32],
            amount: 100,
            nonce,
            signature: None,
            public_key: None,
        })
    }

    fn dummy_tx() -> Transaction {
        dummy_tx_with_nonce(0)
    }

    #[test]
    fn test_submit_and_drain() {
        let mut pool = Mempool::new();
        assert!(pool.is_empty());

        assert!(pool.submit(dummy_tx_with_nonce(0)));
        assert!(pool.submit(dummy_tx_with_nonce(1)));
        assert_eq!(pool.len(), 2);

        let txs = pool.drain();
        assert_eq!(txs.len(), 2);
        assert!(pool.is_empty());
    }

    #[test]
    fn test_drain_empty() {
        let mut pool = Mempool::new();
        let txs = pool.drain();
        assert!(txs.is_empty());
    }

    #[test]
    fn test_max_size_rejection() {
        let mut pool = Mempool::with_max_size(2);
        assert!(pool.submit(dummy_tx_with_nonce(0)));
        assert!(pool.submit(dummy_tx_with_nonce(1)));
        // Third should be rejected (pool full)
        assert!(!pool.submit(dummy_tx_with_nonce(2)));
        assert_eq!(pool.len(), 2);
        assert_eq!(pool.rejected_count(), 1);
    }

    #[test]
    fn test_total_bytes_tracking() {
        let mut pool = Mempool::new();
        pool.submit(dummy_tx());
        assert!(pool.total_bytes() > 0);

        pool.drain();
        assert_eq!(pool.total_bytes(), 0);
    }

    #[test]
    fn test_duplicate_rejected() {
        let mut pool = Mempool::new();
        let tx = dummy_tx();
        assert!(pool.submit(tx.clone()));
        assert!(!pool.submit(tx.clone()));
        assert_eq!(pool.len(), 1);
        assert_eq!(pool.duplicate_count(), 1);
    }

    #[test]
    fn test_duplicate_priority_rejected() {
        let mut pool = Mempool::new();
        let tx = dummy_tx();
        assert!(pool.submit(tx.clone()));
        assert!(!pool.submit_priority(tx.clone()));
        assert_eq!(pool.len(), 1);
        assert_eq!(pool.duplicate_count(), 1);
    }

    #[test]
    fn test_drain_clears_seen_set() {
        let mut pool = Mempool::new();
        let tx = dummy_tx();
        pool.submit(tx.clone());
        pool.drain();
        // After drain, same TX can be re-submitted
        assert!(pool.submit(tx));
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn test_take_deterministic_order() {
        let mut pool = Mempool::new();
        let tx_a = Transaction::Transfer(TransferTx {
            from: [1u8; 32],
            to: [2u8; 32],
            amount: 100,
            nonce: 0,
            signature: None,
            public_key: None,
        });
        let tx_b = Transaction::Transfer(TransferTx {
            from: [1u8; 32],
            to: [2u8; 32],
            amount: 200,
            nonce: 1,
            signature: None,
            public_key: None,
        });
        let tx_c = Transaction::Transfer(TransferTx {
            from: [1u8; 32],
            to: [2u8; 32],
            amount: 300,
            nonce: 2,
            signature: None,
            public_key: None,
        });

        // Submit in one order
        let mut pool1 = Mempool::new();
        pool1.submit(tx_a.clone());
        pool1.submit(tx_b.clone());
        pool1.submit(tx_c.clone());

        // Submit in reverse order
        let mut pool2 = Mempool::new();
        pool2.submit(tx_c.clone());
        pool2.submit(tx_b.clone());
        pool2.submit(tx_a.clone());

        let taken1 = pool1.take(3);
        let taken2 = pool2.take(3);

        // Both pools should produce identical ordering (sorted by tx_hash)
        assert_eq!(taken1.len(), taken2.len());
        for (a, b) in taken1.iter().zip(taken2.iter()) {
            assert_eq!(a.tx_hash(), b.tx_hash());
        }
    }

    #[test]
    fn test_take_partial_leaves_remainder() {
        let mut pool = Mempool::new();
        for i in 0..5u64 {
            pool.submit(Transaction::Transfer(TransferTx {
                from: [1u8; 32],
                to: [2u8; 32],
                amount: i * 100,
                nonce: i,
                signature: None,
                public_key: None,
            }));
        }
        let taken = pool.take(3);
        assert_eq!(taken.len(), 3);
        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn test_contains_hash() {
        let mut pool = Mempool::new();
        let tx = dummy_tx();
        let hash = tx.tx_hash();
        assert!(!pool.contains_hash(&hash));
        pool.submit(tx);
        assert!(pool.contains_hash(&hash));
    }

    #[test]
    fn test_different_nonces_not_duplicate() {
        let mut pool = Mempool::new();
        let tx1 = Transaction::Transfer(TransferTx {
            from: [1u8; 32],
            to: [2u8; 32],
            amount: 100,
            nonce: 0,
            signature: None,
            public_key: None,
        });
        let tx2 = Transaction::Transfer(TransferTx {
            from: [1u8; 32],
            to: [2u8; 32],
            amount: 100,
            nonce: 1,
            signature: None,
            public_key: None,
        });
        assert!(pool.submit(tx1));
        assert!(pool.submit(tx2));
        assert_eq!(pool.len(), 2);
        assert_eq!(pool.duplicate_count(), 0);
    }

    /// Punch-list 5: a tx whose size would push the pool past the global
    /// byte cap is rejected, even when the per-tx and per-account caps
    /// would otherwise admit it.
    #[test]
    fn test_global_byte_cap_rejects_when_pool_would_overflow() {
        // Construct a Blob tx whose body alone is just under the per-tx
        // limit. Submitting enough of these should fill the global cap
        // long before the 10K-tx count cap.
        let mut pool = Mempool::new();
        // Each blob: 100 KB of payload — well under MAX_TX_SIZE_BYTES (128 KB)
        // but means we can fit roughly MAX_MEMPOOL_BYTES / 100KB ≈ 2621 txs.
        // We don't actually need to reach that count — just verify the cap
        // is enforced. To keep the test fast we use a much smaller artificial
        // mempool by writing many txs and asserting rejected_count > 0
        // long before MAX_MEMPOOL_SIZE.
        //
        // To make the test deterministic and fast, we craft a single
        // borderline-large blob and confirm the second one is rejected.
        // We size the blob so two of them exceed the global cap.

        // We can't easily mutate the const, so instead we exercise the
        // contract directly: total_bytes counter advances and the
        // saturating_add check fires. Use a blob just over half the cap.
        let half_cap = (super::MAX_MEMPOOL_BYTES / 2) + 1024;
        let payload_size = half_cap.saturating_sub(64); // 64-byte fudge for tx framing
        let blob1 = Transaction::Blob(BlobTx {
            namespace_id: 1,
            data: vec![0xABu8; payload_size],
            submitter: [1u8; 32],
            nonce: 0,
            signature: None,
            public_key: None,
        });
        // Each blob exceeds MAX_TX_SIZE_BYTES (128 KiB) so the per-tx cap
        // would reject it. This test instead targets the global cap path,
        // so we'd need to reduce the blob size below MAX_TX_SIZE_BYTES and
        // submit many. Switch strategy: use small blobs and many submits.
        let _ = blob1; // unused — keep the example for documentation

        // Submit ~3000 blobs of just-under-128KB until the global cap fires.
        let mut submitted = 0usize;
        let mut rejected_after = false;
        let blob_payload = 100 * 1024; // 100 KB payload
        for i in 0..u32::MAX {
            let tx = Transaction::Blob(BlobTx {
                namespace_id: 1,
                data: vec![0u8; blob_payload],
                submitter: [(i & 0xff) as u8; 32], // spread across senders to dodge per-account cap
                nonce: i as u64,
                signature: None,
                public_key: None,
            });
            if pool.submit(tx) {
                submitted += 1;
                if submitted >= super::MAX_MEMPOOL_SIZE {
                    panic!("byte cap should fire before tx-count cap");
                }
            } else {
                rejected_after = true;
                break;
            }
        }
        assert!(rejected_after, "byte cap should reject a tx eventually");
        assert!(
            pool.total_bytes() <= super::MAX_MEMPOOL_BYTES,
            "total_bytes ({}) must stay within MAX_MEMPOOL_BYTES ({})",
            pool.total_bytes(),
            super::MAX_MEMPOOL_BYTES
        );
        assert!(
            pool.rejected_count() >= 1,
            "rejected_count must reflect the byte-cap rejection"
        );
    }

    // ─── MEV-resistance: take_with_priority (Task #34) ────────────────

    #[test]
    fn test_take_with_priority_old_txs_lose_to_new() {
        let mut pool = Mempool::new();
        // Submit tx_a at "block 0" (current_epoch=0).
        pool.set_epoch(0);
        let tx_a = dummy_tx_with_nonce(0);
        let tx_a_hash = tx_a.tx_hash();
        assert!(pool.submit(tx_a));
        // Several blocks pass; submit tx_b at "block 12" — well past
        // multiple half-life windows for tx_a.
        pool.set_epoch(12);
        let tx_b = dummy_tx_with_nonce(1);
        let tx_b_hash = tx_b.tx_hash();
        assert!(pool.submit(tx_b));
        // Drain at block 12. tx_b has full priority (fresh), tx_a has
        // decayed by 12 / 4 = 3 half-lives → ~12.5% remaining.
        let drained = pool.take_with_priority(2, 12);
        assert_eq!(drained.len(), 2);
        // tx_b sorts first because its priority is higher.
        assert_eq!(drained[0].tx_hash(), tx_b_hash);
        assert_eq!(drained[1].tx_hash(), tx_a_hash);
    }

    #[test]
    fn test_take_with_priority_fresh_txs_keep_input_order_for_same_sender() {
        let mut pool = Mempool::new();
        pool.set_epoch(0);
        let tx0 = dummy_tx_with_nonce(0);
        let tx1 = dummy_tx_with_nonce(1);
        assert!(pool.submit(tx0.clone()));
        assert!(pool.submit(tx1.clone()));
        // Both submitted at the same epoch → identical priority. Tie-break
        // by sender then nonce → nonce 0 ahead of nonce 1.
        let drained = pool.take_with_priority(2, 0);
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].nonce(), Some(0));
        assert_eq!(drained[1].nonce(), Some(1));
    }

    use proptest::prelude::*;

    proptest! {
        /// Lane I.5 invariant proof: `antichain_project` always
        /// produces a kept set with no duplicate senders, and dropped +
        /// kept = input. Locks the consumer-side helper called by
        /// `tendermint.rs::create_proposal` when `block_source_mode`
        /// = `"antichain"`.
        #[test]
        fn antichain_project_invariants(
            // Up to 32 candidates; senders ∈ [0, 8) so collisions
            // are likely.
            inputs in proptest::collection::vec((0u8..8, 0u64..50), 0..32),
        ) {
            let candidates: Vec<Transaction> = inputs
                .iter()
                .map(|(s, _)| dummy_tx_with_nonce_and_sender(*s, 0))
                .collect();
            let hints: Vec<u64> = inputs.iter().map(|(_, h)| *h).collect();
            let pre_len = candidates.len();
            let pre_hints = hints.clone();
            let pre_txs = candidates.clone();

            let (kept, kept_hints, dropped) =
                antichain_project(candidates, hints);

            // Property 1: no duplicate senders in the kept set.
            let mut seen: std::collections::HashSet<[u8; 32]> =
                std::collections::HashSet::new();
            for tx in &kept {
                if let Some(addr) = tx.sender() {
                    prop_assert!(
                        seen.insert(*addr),
                        "antichain_project must not return same-sender duplicates"
                    );
                }
            }

            // Property 2: kept_hints index-parallel to kept.
            prop_assert_eq!(kept.len(), kept_hints.len());

            // Property 3: kept + dropped = input total (nothing lost).
            prop_assert_eq!(kept.len() + dropped.len(), pre_len);

            // Property 4: kept hints are a subsequence of input hints
            // (priority order preserved by the FIFO-preserving filter).
            let kept_hints_iter = kept_hints.iter();
            let mut input_iter = pre_hints.iter();
            for kh in kept_hints_iter {
                let mut found = false;
                while let Some(ih) = input_iter.next() {
                    if ih == kh {
                        found = true;
                        break;
                    }
                }
                prop_assert!(
                    found,
                    "kept hints must appear in input order — projection \
                     should not reorder"
                );
            }

            // Property 5: every dropped tx has the same sender as some
            // earlier-input tx that's in the kept set (i.e. drops are
            // always due to a real same-sender conflict, not arbitrary).
            let kept_senders: std::collections::HashSet<[u8; 32]> = kept
                .iter()
                .filter_map(|t| t.sender().copied())
                .collect();
            for d in &dropped {
                if let Some(addr) = d.sender() {
                    prop_assert!(
                        kept_senders.contains(addr),
                        "dropped tx must conflict with a kept tx of the same sender"
                    );
                }
            }
            // Silence unused-result warning for the cloned input vec.
            let _ = pre_txs;
        }
    }

    fn dummy_tx_with_nonce_and_sender(sender: u8, nonce: u64) -> Transaction {
        Transaction::Transfer(TransferTx {
            from: [sender; 32],
            to: [99u8; 32],
            amount: 100,
            nonce,
            signature: None,
            public_key: None,
        })
    }

    #[test]
    fn block_source_trait_delegates_to_mempool() {
        // Lane G.1: the BlockSource trait must dispatch to the same
        // bytes-equal behaviour as direct Mempool calls. Hold a Mempool
        // behind `&mut dyn BlockSource` and verify all four trait
        // methods produce results identical to calling the inherent
        // methods. This locks in the substrate seam: any future impl
        // (antichain, MEV-aware) can be swapped behind the same dyn
        // pointer without consensus code changes.
        let mut pool = Mempool::new();
        // Trait dispatch handle.
        let bs: &mut dyn BlockSource = &mut pool;

        // is_empty default impl, derived from len().
        assert!(bs.is_empty());
        assert_eq!(bs.len(), 0);

        bs.set_epoch(5);
        let admitted = bs.submit_priority(dummy_tx_with_nonce(1));
        assert!(admitted);
        assert_eq!(bs.len(), 1);
        assert!(!bs.is_empty());

        // Drawing for a proposal returns the same triple shape as the
        // concrete method.
        let (txs, sum, hints) = bs.take_with_priority_sum_and_hints(10, 5);
        assert_eq!(txs.len(), 1);
        assert_eq!(hints.len(), txs.len());
        // Submitted at epoch 5 with current_block 5 → elapsed=0 → max
        // priority = BASE_INCLUSION_ENERGY (per Lane A.2 spec).
        assert!(sum >= 1);
        // Hint is the recorded submit epoch (5).
        assert_eq!(hints[0], 5);
    }
}
