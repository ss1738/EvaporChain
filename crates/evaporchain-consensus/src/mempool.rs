use evaporchain_types::Transaction;
use std::collections::VecDeque;

/// Maximum number of transactions in the mempool (DoS protection).
const MAX_MEMPOOL_SIZE: usize = 10_000;

/// Maximum transaction data size in bytes (prevents oversized payloads).
const MAX_TX_SIZE_BYTES: usize = 128 * 1024; // 128 KB

/// Thread-safe pending transaction pool with DoS protection.
///
/// Enforces size limits and basic validation before accepting transactions.
pub struct Mempool {
    pending: VecDeque<Transaction>,
    /// Total bytes of serialized transactions in the pool (approximate).
    total_bytes: usize,
    /// Maximum pool size.
    max_size: usize,
    /// Transactions rejected due to pool being full.
    rejected_count: u64,
}

impl Mempool {
    pub fn new() -> Self {
        Self {
            pending: VecDeque::new(),
            total_bytes: 0,
            max_size: MAX_MEMPOOL_SIZE,
            rejected_count: 0,
        }
    }

    /// Create a mempool with a custom size limit.
    pub fn with_max_size(max_size: usize) -> Self {
        Self {
            pending: VecDeque::new(),
            total_bytes: 0,
            max_size,
            rejected_count: 0,
        }
    }

    /// Add a transaction to the pool. Returns false if rejected (pool full or oversized).
    pub fn submit(&mut self, tx: Transaction) -> bool {
        // Reject if pool is full
        if self.pending.len() >= self.max_size {
            self.rejected_count += 1;
            return false;
        }

        // Estimate transaction size
        let tx_size = Self::estimate_tx_size(&tx);
        if tx_size > MAX_TX_SIZE_BYTES {
            self.rejected_count += 1;
            return false;
        }

        self.total_bytes += tx_size;
        self.pending.push_back(tx);
        true
    }

    /// Add a high-priority transaction to the FRONT of the pool.
    /// Used for API-submitted transactions that should be included before demo txs.
    pub fn submit_priority(&mut self, tx: Transaction) -> bool {
        if self.pending.len() >= self.max_size {
            self.rejected_count += 1;
            return false;
        }
        let tx_size = Self::estimate_tx_size(&tx);
        if tx_size > MAX_TX_SIZE_BYTES {
            self.rejected_count += 1;
            return false;
        }
        self.total_bytes += tx_size;
        self.pending.push_front(tx);
        true
    }

    /// Drain all pending transactions for inclusion in the next block.
    pub fn drain(&mut self) -> Vec<Transaction> {
        self.total_bytes = 0;
        self.pending.drain(..).collect()
    }

    /// Take up to `n` transactions from the front of the pool, leaving the rest.
    pub fn take(&mut self, n: usize) -> Vec<Transaction> {
        let take_count = n.min(self.pending.len());
        let taken: Vec<Transaction> = self.pending.drain(..take_count).collect();
        // Recalculate total_bytes for remaining
        self.total_bytes = self.pending.iter()
            .map(|tx| serde_json::to_vec(tx).map(|v| v.len()).unwrap_or(0))
            .sum();
        taken
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

    fn dummy_tx() -> Transaction {
        Transaction::Transfer(TransferTx {
            from: [1u8; 32],
            to: [2u8; 32],
            amount: 100,
            nonce: 0,
            signature: None,
            public_key: None,
        })
    }

    #[test]
    fn test_submit_and_drain() {
        let mut pool = Mempool::new();
        assert!(pool.is_empty());

        assert!(pool.submit(dummy_tx()));
        assert!(pool.submit(dummy_tx()));
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
        assert!(pool.submit(dummy_tx()));
        assert!(pool.submit(dummy_tx()));
        // Third should be rejected
        assert!(!pool.submit(dummy_tx()));
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
}
