use evaporchain_types::Transaction;
use std::collections::{HashSet, VecDeque};

/// Maximum number of transactions in the mempool (DoS protection).
const MAX_MEMPOOL_SIZE: usize = 10_000;

/// Maximum transaction data size in bytes (prevents oversized payloads).
const MAX_TX_SIZE_BYTES: usize = 128 * 1024; // 128 KB

/// Thread-safe pending transaction pool with DoS protection.
///
/// Enforces size limits and basic validation before accepting transactions.
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
        }
    }

    /// Add a transaction to the pool. Returns false if rejected (duplicate, pool full, or oversized).
    pub fn submit(&mut self, tx: Transaction) -> bool {
        let hash = tx.tx_hash();
        if self.seen.contains(&hash) {
            self.duplicate_count += 1;
            return false;
        }
        if self.pending.len() >= self.max_size {
            self.rejected_count += 1;
            return false;
        }
        let tx_size = Self::estimate_tx_size(&tx);
        if tx_size > MAX_TX_SIZE_BYTES {
            self.rejected_count += 1;
            return false;
        }
        self.seen.insert(hash);
        self.total_bytes += tx_size;
        self.pending.push_back(tx);
        true
    }

    /// Add a high-priority transaction to the FRONT of the pool.
    /// Used for API-submitted transactions that should be included before demo txs.
    pub fn submit_priority(&mut self, tx: Transaction) -> bool {
        let hash = tx.tx_hash();
        if self.seen.contains(&hash) {
            self.duplicate_count += 1;
            return false;
        }
        if self.pending.len() >= self.max_size {
            self.rejected_count += 1;
            return false;
        }
        let tx_size = Self::estimate_tx_size(&tx);
        if tx_size > MAX_TX_SIZE_BYTES {
            self.rejected_count += 1;
            return false;
        }
        self.seen.insert(hash);
        self.total_bytes += tx_size;
        self.pending.push_front(tx);
        true
    }

    /// Drain all pending transactions for inclusion in the next block.
    pub fn drain(&mut self) -> Vec<Transaction> {
        self.total_bytes = 0;
        self.seen.clear();
        self.pending.drain(..).collect()
    }

    /// Take up to `n` transactions, sorted by tx_hash for deterministic ordering.
    /// All validators selecting from the same TX set will produce identical proposals.
    pub fn take(&mut self, n: usize) -> Vec<Transaction> {
        let all: Vec<Transaction> = self.pending.drain(..).collect();
        let mut with_hash: Vec<([u8; 32], Transaction)> = all
            .into_iter()
            .map(|tx| (tx.tx_hash(), tx))
            .collect();
        with_hash.sort_by(|a, b| a.0.cmp(&b.0));

        let take_count = n.min(with_hash.len());
        let mut taken = Vec::with_capacity(take_count);
        let mut remaining = VecDeque::new();

        for (i, (_hash, tx)) in with_hash.into_iter().enumerate() {
            if i < take_count {
                self.seen.remove(&_hash);
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

    /// Take up to `n` transactions sorted by tx_hash (deterministic proposal ordering).
    /// Returns transactions paired with their hashes for callers that need them.
    pub fn take_with_hashes(&mut self, n: usize) -> Vec<([u8; 32], Transaction)> {
        let all: Vec<Transaction> = self.pending.drain(..).collect();
        let mut with_hash: Vec<([u8; 32], Transaction)> = all
            .into_iter()
            .map(|tx| (tx.tx_hash(), tx))
            .collect();
        with_hash.sort_by(|a, b| a.0.cmp(&b.0));

        let take_count = n.min(with_hash.len());
        let (taken, rest) = with_hash.split_at(take_count);
        let result: Vec<([u8; 32], Transaction)> = taken.to_vec();

        self.pending = rest.iter().map(|(_, tx)| tx.clone()).collect();
        for (h, _) in &result {
            self.seen.remove(h);
        }
        self.total_bytes = self.pending.iter()
            .map(Self::estimate_tx_size)
            .sum();
        result
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
