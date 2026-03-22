use evaporchain_types::Transaction;
use std::collections::VecDeque;

/// Thread-safe pending transaction pool.
///
/// In a real implementation this would use `Arc<Mutex<...>>` or a lock-free
/// queue and validate transactions before accepting them. For the single-node
/// devnet, a simple `VecDeque` suffices.
pub struct Mempool {
    pending: VecDeque<Transaction>,
}

impl Mempool {
    pub fn new() -> Self {
        Self {
            pending: VecDeque::new(),
        }
    }

    /// Add a transaction to the pool.
    pub fn submit(&mut self, tx: Transaction) {
        self.pending.push_back(tx);
    }

    /// Drain all pending transactions for inclusion in the next block.
    pub fn drain(&mut self) -> Vec<Transaction> {
        self.pending.drain(..).collect()
    }

    /// Number of pending transactions.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Whether the mempool is empty.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
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

        pool.submit(dummy_tx());
        pool.submit(dummy_tx());
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
}
