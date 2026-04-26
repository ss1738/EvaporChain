use evaporchain_crypto::signatures::{HybridVerifier, Verifier};
use evaporchain_types::Transaction;
use std::collections::{HashMap, HashSet, VecDeque};

/// Maximum number of transactions in the mempool (DoS protection).
const MAX_MEMPOOL_SIZE: usize = 10_000;

/// Maximum transaction data size in bytes (prevents oversized payloads).
const MAX_TX_SIZE_BYTES: usize = 128 * 1024; // 128 KB

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
        let tx_size = Self::estimate_tx_size(tx);
        if tx_size > MAX_TX_SIZE_BYTES {
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
        if self.verify_signatures {
            if !matches!(tx, Transaction::Unshield(_) | Transaction::PrivateTransfer(_)) {
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
        let mut with_hash: Vec<([u8; 32], Transaction)> = all
            .into_iter()
            .map(|tx| (tx.tx_hash(), tx))
            .collect();
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
        self.total_bytes = self.pending.iter()
            .map(Self::estimate_tx_size)
            .sum();
        taken
    }

    /// Take up to `n` transactions with nonce-aware ordering.
    /// Returns transactions paired with their hashes for callers that need them.
    pub fn take_with_hashes(&mut self, n: usize) -> Vec<([u8; 32], Transaction)> {
        let all: Vec<Transaction> = self.pending.drain(..).collect();
        let mut with_hash: Vec<([u8; 32], Transaction)> = all
            .into_iter()
            .map(|tx| (tx.tx_hash(), tx))
            .collect();
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
        self.total_bytes = self.pending.iter()
            .map(Self::estimate_tx_size)
            .sum();
        result
    }

    /// Take transactions up to a gas limit with nonce-aware ordering.
    pub fn take_with_gas_limit(&mut self, max_txs: usize, gas_limit: u64) -> Vec<Transaction> {
        let all: Vec<Transaction> = self.pending.drain(..).collect();
        let mut with_hash: Vec<([u8; 32], Transaction)> = all
            .into_iter()
            .map(|tx| (tx.tx_hash(), tx))
            .collect();
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
    fn sort_nonce_aware(txs: &mut Vec<([u8; 32], Transaction)>) {
        txs.sort_by(|a, b| {
            let sender_a = a.1.sender().copied().unwrap_or([0xff; 32]);
            let sender_b = b.1.sender().copied().unwrap_or([0xff; 32]);
            sender_a.cmp(&sender_b)
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
                100_000 + 20_000 * ptx.input_nullifiers.len() as u64
                    + 15_000 * ptx.output_commitments.len() as u64
            }
            Transaction::Deferred(dtx) => 75_000 + 5_000 * dtx.guards.len() as u64,
            Transaction::Blob(tx) => 50_000 + 10 * tx.data.len() as u64,
            Transaction::Governance(_) => 25_000,
            Transaction::MultiSig(_) => 50_000,
            Transaction::UserOp(tx) => 30_000 + 16 * tx.call_data.len() as u64,
            Transaction::UpgradeContract(tx) => 100_000 + 200 * tx.new_bytecode.len() as u64,
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
                32 + t.template.len() + t.init_args.len() + 16
                    + t.signature.as_ref().map_or(0, |s| s.len())
                    + t.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::CallContract(t) => {
                32 + 32 + t.method.len() + t.args.len() + 16
                    + t.signature.as_ref().map_or(0, |s| s.len())
                    + t.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::DeployScript(t) => {
                32 + t.source_code.len() + 16
                    + t.signature.as_ref().map_or(0, |s| s.len())
                    + t.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::CallScript(t) => {
                32 + 8 + t.method.len() + t.args.len() + 16
                    + t.signature.as_ref().map_or(0, |s| s.len())
                    + t.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::ValidatorStake(t) => {
                32 + 8 + 8 + 8
                    + t.bls_public_key.as_ref().map_or(0, |k| k.len())
                    + t.signature.as_ref().map_or(0, |s| s.len())
                    + t.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::ValidatorExit(t) => {
                32 + 8 + 8
                    + t.signature.as_ref().map_or(0, |s| s.len())
                    + t.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::ValidatorClaimStake(t) => {
                32 + 8 + 8
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
                32 + tx.data.len() + 8 + 8
                    + tx.signature.as_ref().map_or(0, |s| s.len())
                    + tx.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::Governance(tx) => {
                32 + 8 + 64
                    + tx.signature.as_ref().map_or(0, |s| s.len())
                    + tx.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::MultiSig(tx) => {
                32 + 1 + 8 + tx.signers.len() * 32 + tx.inner_tx_bytes.len()
                    + tx.signatures.len() * 64
            }
            Transaction::UserOp(tx) => {
                32 + 8 + 8 + tx.call_data.len()
                    + tx.signature.as_ref().map_or(0, |s| s.len())
                    + tx.public_key.as_ref().map_or(0, |p| p.len())
            }
            Transaction::UpgradeContract(tx) => {
                32 + 8 + 8 + tx.new_bytecode.len()
                    + tx.signature.as_ref().map_or(0, |s| s.len())
                    + tx.public_key.as_ref().map_or(0, |p| p.len())
            }
        }
    }
}

impl Default for Mempool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_types::TransferTx;

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
            from: [1u8; 32], to: [2u8; 32], amount: 100, nonce: 0,
            signature: None, public_key: None,
        });
        let tx_b = Transaction::Transfer(TransferTx {
            from: [1u8; 32], to: [2u8; 32], amount: 200, nonce: 1,
            signature: None, public_key: None,
        });
        let tx_c = Transaction::Transfer(TransferTx {
            from: [1u8; 32], to: [2u8; 32], amount: 300, nonce: 2,
            signature: None, public_key: None,
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
                from: [1u8; 32], to: [2u8; 32], amount: i * 100, nonce: i,
                signature: None, public_key: None,
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
            from: [1u8; 32], to: [2u8; 32], amount: 100, nonce: 0,
            signature: None, public_key: None,
        });
        let tx2 = Transaction::Transfer(TransferTx {
            from: [1u8; 32], to: [2u8; 32], amount: 100, nonce: 1,
            signature: None, public_key: None,
        });
        assert!(pool.submit(tx1));
        assert!(pool.submit(tx2));
        assert_eq!(pool.len(), 2);
        assert_eq!(pool.duplicate_count(), 0);
    }
}
