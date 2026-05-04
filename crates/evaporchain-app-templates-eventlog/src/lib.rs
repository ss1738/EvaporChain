//! Append-only deploy-event log.
//!
//! Wraps [`DeployReceipt`][1] events into a monotonic, block-ordered
//! log. The chain appends a receipt as soon as a deploy finalises;
//! indexers stream receipts via [`since`][2] / [`range`][3]; light
//! clients verify individual receipts against the log's Merkle
//! root.
//!
//! ## Append discipline
//!
//! Three structural invariants enforced at append time:
//!
//! 1. **Block heights are monotone non-decreasing.** A receipt for
//!    height H+1 cannot be appended before a receipt for height H.
//!    Within a single height, multiple receipts (multiple deploys
//!    in one block) are allowed and ordered by their `event_id`
//!    (BLAKE3 of canonical bytes — validator-deterministic).
//! 2. **No duplicate event_ids.** Each receipt's BLAKE3 commit is
//!    unique by construction (different deploys differ in nonce
//!    or class or block_height); duplicates indicate a chain bug
//!    and the log refuses them.
//! 3. **Merkle root is over canonical bytes, not the receipt
//!    struct.** Light clients can verify a receipt without
//!    deserialising the whole log — they need just the receipt's
//!    canonical bytes and a Merkle path.
//!
//! ## What this crate does NOT do
//!
//! - It does NOT persist to disk. Pure in-memory data structure;
//!   the chain wraps it with whatever durability layer it uses
//!   (RocksDB, snapshot files).
//! - It does NOT gossip events to peers. The chain's network layer
//!   handles that; this crate is purely the in-process log.
//! - It does NOT prune. Pruning policy lives one layer up — some
//!   chains keep all receipts forever, others prune past a window.
//!
//! ## Module map
//!
//! - [`log`] — [`DeployEventLog`] append-only structure +
//!   [`AppendError`] + range queries.
//! - [`merkle`] — [`merkle_root`] computation + verification helper.
//!
//! [1]: evaporchain-app-templates-receipt::DeployReceipt
//! [2]: log::DeployEventLog::since
//! [3]: log::DeployEventLog::range

pub mod log;
pub mod merkle;

pub use log::{AppendError, DeployEventLog};
pub use merkle::{merkle_root, verify_inclusion};
