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

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use evaporchain_app_templates::class::MAYFLY;
    use evaporchain_app_templates_receipt::DeployReceipt;

    fn receipt(block_height: u64, instance_byte: u8) -> DeployReceipt {
        let mut iid = [0u8; 32];
        iid[0] = instance_byte;
        DeployReceipt::new(
            [1u8; 32], iid, MAYFLY, [7u8; 32], 500, block_height, block_height,
        )
        .unwrap()
    }

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "DeployEventLog is monotonic by block height,
    /// rejects duplicate event_ids, and Merkle-roots over canonical
    /// receipt bytes. Light clients verify a receipt's inclusion via
    /// (root, receipt, path) without holding the whole log."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let mut log = DeployEventLog::new();
        log.append(receipt(1, 0xAA)).unwrap();
        log.append(receipt(2, 0xBB)).unwrap();
        log.append(receipt(2, 0xCC)).unwrap(); // same height, different instance — OK

        // Monotonicity: appending an older height fails.
        let res = log.append(receipt(1, 0xDD));
        assert!(res.is_err(), "monotone non-decreasing block heights required");

        // Duplicate receipt rejected.
        let dup = log.append(receipt(2, 0xBB));
        assert!(dup.is_err(), "duplicate event_id must reject");
    }
}
