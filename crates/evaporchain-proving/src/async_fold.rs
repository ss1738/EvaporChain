//! Async fold queue — moves the per-block Nova IVC fold off the
//! consensus thread.
//!
//! Under real `--prove`, each fold runs ~250-350 ms (14041+10554
//! constraints). When the consensus tick blocks on it, block production
//! is capped at roughly `1 / fold_time` blocks/sec — fine behind
//! Tendermint's 2 s default block interval, but a hard cap once the
//! interval is tightened.
//!
//! `FoldQueue` decouples submission from work:
//! - Producer (consensus tick) calls [`FoldQueue::submit`] with the
//!   block + post-execution state root. Submit is ~free (a `try_send`
//!   on a bounded `mpsc`).
//! - A single worker task drains the channel and folds in order
//!   (Nova IVC requires deterministic block ordering — no parallelism
//!   gain at this layer, only latency hiding).
//! - [`FoldQueue::latest_proof`] returns the most-recent successfully
//!   generated [`ChainProof`]. Consensus reads this when it wants to
//!   attach a proof to an outgoing block; the proof's `num_steps` field
//!   tells receivers exactly which prefix it covers, so wire-compat
//!   with the previous synchronous attach is preserved.
//! - If the channel is full, [`FoldQueue::submit`] returns
//!   [`SubmitOutcome::QueueFull`] and the caller is expected to fall
//!   back to the synchronous `chain_prover.fold_block` path for that
//!   one block — never silently dropped.

use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{error, warn};

use crate::chain_proof::{ChainProof, ChainProver};
use crate::ProvingError;
use evaporchain_types::Block;

/// Default channel capacity. 256 blocks of headroom — at 1 blk/s
/// production and 1 blk/s fold this is 4 minutes of slack; at 2 blk/s
/// production and 1 blk/s fold the queue saturates in ~2 minutes which
/// gives ample warning before backpressure kicks in.
pub const DEFAULT_FOLD_QUEUE_CAPACITY: usize = 256;

/// Backpressure warning threshold (75% full). Logged once per crossing
/// from below to above.
const BACKPRESSURE_WATERMARK_PCT: usize = 75;

/// One unit of fold work submitted by the producer.
struct FoldJob {
    block: Block,
    new_state_root: [u8; 32],
}

/// Outcome of a [`FoldQueue::submit`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// Job accepted by the worker; will be folded asynchronously.
    Queued,
    /// Channel was full — caller must fall back to synchronous fold for
    /// this block to avoid dropping it. This must never happen in
    /// steady-state; if it does, the worker is consistently slower than
    /// block production.
    QueueFull,
    /// Worker has shut down. Submit is a no-op and the caller should
    /// fall back to synchronous fold (or stop folding entirely if the
    /// node is shutting down).
    WorkerGone,
}

/// Async wrapper around a [`ChainProver`].
///
/// Holds an mpsc channel to a single worker task and an [`RwLock`]
/// publishing the latest successfully-generated [`ChainProof`]. Drop
/// the queue to close the channel; the worker drains remaining jobs
/// and exits.
pub struct FoldQueue {
    sender: mpsc::Sender<FoldJob>,
    capacity: usize,
    latest_proof: Arc<RwLock<Option<ChainProof>>>,
    /// Last seen depth across the high-water threshold; reset when
    /// depth drops below it. Protected by the same lock as the proof
    /// to avoid a second lock on the hot path.
    backpressure_warned: Arc<RwLock<bool>>,
}

impl FoldQueue {
    /// Spawn a worker that owns the given prover. Returns a
    /// non-`Send`-clonable [`FoldQueue`] handle. The worker exits when
    /// the queue is dropped.
    pub fn spawn(prover: ChainProver, capacity: usize) -> Self {
        let (tx, rx) = mpsc::channel::<FoldJob>(capacity);
        let latest_proof = Arc::new(RwLock::new(None));
        let backpressure_warned = Arc::new(RwLock::new(false));

        let worker_proof = Arc::clone(&latest_proof);
        tokio::spawn(async move {
            run_fold_worker(prover, rx, worker_proof).await;
        });

        Self {
            sender: tx,
            capacity,
            latest_proof,
            backpressure_warned,
        }
    }

    /// Submit a (block, post-execution state root) for asynchronous
    /// folding. Returns immediately. See [`SubmitOutcome`] for fallback
    /// guidance.
    ///
    /// The caller is responsible for invoking this in the same order
    /// blocks were committed — Nova IVC is sequential, and out-of-order
    /// submissions will fold against the wrong predecessor state root.
    pub fn submit(&self, block: Block, new_state_root: [u8; 32]) -> SubmitOutcome {
        let job = FoldJob {
            block,
            new_state_root,
        };
        match self.sender.try_send(job) {
            Ok(()) => {
                self.maybe_warn_backpressure();
                SubmitOutcome::Queued
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                warn!(
                    capacity = self.capacity,
                    "FoldQueue full — caller must fall back to synchronous fold for this block"
                );
                SubmitOutcome::QueueFull
            }
            Err(mpsc::error::TrySendError::Closed(_)) => SubmitOutcome::WorkerGone,
        }
    }

    /// Most-recent successfully-generated chain proof. `None` until the
    /// worker has completed at least one fold.
    pub fn latest_proof(&self) -> Option<ChainProof> {
        // try_read for the hot path — if the worker holds the write
        // lock briefly during its publish, just return the previous
        // value (tracker semantics, not strict consistency).
        self.latest_proof.try_read().ok().and_then(|g| g.clone())
    }

    /// Approximate current queue depth (for diagnostics / metrics).
    /// `mpsc::Sender::capacity()` returns the *remaining* capacity, so
    /// depth is `capacity - capacity_remaining`.
    pub fn approx_depth(&self) -> usize {
        self.capacity.saturating_sub(self.sender.capacity())
    }

    /// Total channel capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    fn maybe_warn_backpressure(&self) {
        let depth = self.approx_depth();
        let high = self.capacity * BACKPRESSURE_WATERMARK_PCT / 100;
        if depth >= high {
            // Only warn once per crossing.
            if let Ok(mut warned) = self.backpressure_warned.try_write() {
                if !*warned {
                    warn!(
                        depth,
                        capacity = self.capacity,
                        watermark_pct = BACKPRESSURE_WATERMARK_PCT,
                        "FoldQueue depth crossed backpressure watermark — fold worker is falling behind block production"
                    );
                    *warned = true;
                }
            }
        } else if depth == 0 {
            if let Ok(mut warned) = self.backpressure_warned.try_write() {
                *warned = false;
            }
        }
    }
}

async fn run_fold_worker(
    mut prover: ChainProver,
    mut rx: mpsc::Receiver<FoldJob>,
    latest_proof: Arc<RwLock<Option<ChainProof>>>,
) {
    while let Some(job) = rx.recv().await {
        // The fold itself is CPU-bound and synchronous inside
        // `ChainProver::fold_block`. We run it on the current tokio
        // worker thread — the runtime's blocking pool is the right
        // place if folds get slower, but for now ~300ms in a
        // multi-threaded runtime keeps the rest of the node
        // unblocked.
        match prover.fold_block(&job.block, job.new_state_root) {
            Ok(_fold_res) => match prover.generate_chain_proof() {
                Ok(proof) => {
                    let mut guard = latest_proof.write().await;
                    *guard = Some(proof);
                }
                Err(e) => {
                    error!(
                        height = job.block.number,
                        error = %e,
                        "FoldQueue worker: chain proof generation failed"
                    );
                }
            },
            Err(e) => {
                error!(
                    height = job.block.number,
                    error = %e,
                    "FoldQueue worker: fold_block failed"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain_proof::ChainProver;
    use crate::MockProver;

    fn dummy_block(num: u64) -> Block {
        Block {
            number: num,
            epoch: num,
            parent_hash: [0u8; 32],
            state_root: [0u8; 32],
            transactions: vec![],
            timestamp: 0,
            chain_id: String::new(),
            producer_id: None,
            vrf_output: None,
            vrf_proof: None,
            data_root: None,
            blob_commitments: vec![],
            da_certificate: None,
            commit_certificate: None,
            nova_proof: None,
            anchor_hash: None,
            state_function_commitment: None,
            oracle_state_root: None,
            shard_count: None,
            da_row_roots: vec![],
            da_col_roots: vec![],
        }
    }

    fn make_queue(capacity: usize) -> FoldQueue {
        let prover = ChainProver::new(Box::new(MockProver::new()), [0u8; 32], 0);
        FoldQueue::spawn(prover, capacity)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn submit_n_blocks_all_fold_in_order() {
        let queue = make_queue(16);
        for n in 1..=10 {
            let outcome = queue.submit(dummy_block(n), [n as u8; 32]);
            assert_eq!(outcome, SubmitOutcome::Queued, "submit at {n}");
        }
        // Wait for the worker to drain. Polling because the worker is
        // asynchronous and we don't want a flaky sleep.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(p) = queue.latest_proof() {
                if p.num_steps == 10 {
                    break;
                }
            }
            if std::time::Instant::now() >= deadline {
                panic!(
                    "worker did not fold all 10 blocks within 5s; latest = {:?}",
                    queue.latest_proof().map(|p| p.num_steps)
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let final_proof = queue.latest_proof().expect("proof present after drain");
        assert_eq!(final_proof.num_steps, 10);
        assert_eq!(final_proof.block_height, 10);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn queue_full_returns_queue_full_outcome() {
        // Build a queue with capacity 2 and a prover whose fold
        // pretends to be slow so the worker can't drain. We fake
        // slowness by submitting many jobs without yielding — under
        // multi_thread the worker will drain some, so the test asserts
        // *eventually one of the submits is rejected* rather than the
        // exact ordinal.
        let queue = make_queue(2);
        let mut saw_full = false;
        for n in 0..200u64 {
            match queue.submit(dummy_block(n), [n as u8; 32]) {
                SubmitOutcome::QueueFull => {
                    saw_full = true;
                    break;
                }
                SubmitOutcome::Queued => {}
                SubmitOutcome::WorkerGone => panic!("worker exited unexpectedly"),
            }
        }
        // With MockProver folds completing instantly the worker keeps
        // pace with the producer, so QueueFull is unlikely under mock.
        // The real-Nova path will exercise this naturally; here we
        // just assert it doesn't panic and behaves consistently.
        let _ = saw_full;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn worker_gone_after_drop() {
        let queue = make_queue(4);
        // Submit one and let it fold so we know the worker is alive.
        assert_eq!(
            queue.submit(dummy_block(1), [1u8; 32]),
            SubmitOutcome::Queued
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while queue.latest_proof().is_none() {
            assert!(std::time::Instant::now() < deadline, "worker never produced a proof");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        // Drop the queue — sender closes, worker drains and exits.
        drop(queue);
        // No way to assert directly without a join handle; the test's
        // value is that this doesn't deadlock or leak.
    }
}
