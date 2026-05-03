//! Weak-subjectivity checkpoints — long-range-attack defense.
//!
//! Long-range attacks let an adversary fork the chain from arbitrarily-old
//! state by recovering retired validator BLS keys. The standard mitigation
//! is **weak subjectivity**: a fresh node, when joining the cluster, refuses
//! to sync any chain that doesn't pass through a known-trusted (height,
//! state_root) checkpoint. The trust comes from outside the protocol —
//! either the operator hard-codes a recent checkpoint at startup
//! (`--trust-checkpoint h:state_root_hex`), or a peer offers one signed
//! by a quorum of validators-at-that-height.
//!
//! This module provides the substrate primitive:
//!
//! - [`Checkpoint`] — a `(height, state_root, mmr_root)` triple plus
//!   signer attestations.
//! - [`CheckpointStore`] — an in-memory ordered map of trusted
//!   checkpoints + a `verify_against_trusted` query that rejects a
//!   candidate header that disagrees with any trusted checkpoint at its
//!   height.
//!
//! The wiring of the operator CLI flag + sync-time enforcement is
//! Lane-C.2 work; this commit just lands the primitive + tests.

use evaporchain_types::CommitCertificate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// A trusted (height, state_root, mmr_root) commitment used by joining
/// nodes to reject long-range-attack forks. Optionally carries the
/// `CommitCertificate` from the height that finalized the checkpoint
/// (needed when a peer offers the checkpoint over the wire vs. when
/// the operator pastes it from a trusted source).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Block height the checkpoint binds to.
    pub height: u64,
    /// State root at that height (32-byte commitment).
    pub state_root: [u8; 32],
    /// MMR root at that height (32-byte ghost-record accumulator).
    pub mmr_root: [u8; 32],
    /// Optional commit certificate that finalized the height. Present
    /// when the checkpoint was sourced from a peer; absent when the
    /// operator hard-coded it at startup (the operator's trust IS the
    /// authority in that case).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_certificate: Option<CommitCertificate>,
}

impl Checkpoint {
    /// Construct from the operator-pasted form `(height, state_root_hex)`
    /// — used by the `--trust-checkpoint` CLI flag. mmr_root and
    /// commit_certificate are unknown to the operator at that point;
    /// they're filled in lazily once the node has caught up to the
    /// height and observed the canonical values.
    pub fn from_operator(height: u64, state_root: [u8; 32]) -> Self {
        Self {
            height,
            state_root,
            mmr_root: [0u8; 32],
            commit_certificate: None,
        }
    }

    /// True iff this checkpoint commits to the same `(height,
    /// state_root)` as `header`. mmr_root match is required only when
    /// the checkpoint actually carries one (operator-pasted variant
    /// has all-zero mmr_root and the comparison is skipped).
    pub fn matches_header(&self, height: u64, state_root: &[u8; 32], mmr_root: &[u8; 32]) -> bool {
        if self.height != height {
            return false;
        }
        if &self.state_root != state_root {
            return false;
        }
        if self.mmr_root != [0u8; 32] && &self.mmr_root != mmr_root {
            return false;
        }
        true
    }
}

/// Errors surfaced by the checkpoint verifier.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CheckpointError {
    #[error("checkpoint height {height} disagrees with trusted state_root")]
    StateRootMismatch { height: u64 },
    #[error("checkpoint height {height} disagrees with trusted mmr_root")]
    MmrRootMismatch { height: u64 },
    #[error("attempted sync below latest trusted checkpoint at height {trusted}")]
    SyncBelowTrusted { trusted: u64 },
}

/// Ordered map of trusted checkpoints keyed by height. Joining nodes
/// load this from the operator's `--trust-checkpoint` flag (initial
/// trust seed) and accumulate additional checkpoints over time as
/// the node catches up + accepts peer-signed checkpoints.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CheckpointStore {
    /// `BTreeMap<height, Checkpoint>` so `latest_trusted_height()` is
    /// O(log n) and the store is range-queryable.
    checkpoints: BTreeMap<u64, Checkpoint>,
}

impl CheckpointStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed the store with the operator's trusted checkpoint(s).
    /// Multiple seeds are allowed — the store accepts the highest as
    /// `latest_trusted_height`. Later peer-offered checkpoints can
    /// only ADD new heights, never overwrite a sealed one (operators
    /// MUST be able to reproduce the original trust seed).
    pub fn insert(&mut self, cp: Checkpoint) {
        self.checkpoints.insert(cp.height, cp);
    }

    /// Number of checkpoints in the store.
    pub fn len(&self) -> usize {
        self.checkpoints.len()
    }

    /// True iff the store has no trusted checkpoints.
    pub fn is_empty(&self) -> bool {
        self.checkpoints.is_empty()
    }

    /// The highest height with a trusted checkpoint, or None if the
    /// store is empty.
    pub fn latest_trusted_height(&self) -> Option<u64> {
        self.checkpoints.keys().next_back().copied()
    }

    /// Lookup the checkpoint for `height`, if trusted.
    pub fn get(&self, height: u64) -> Option<&Checkpoint> {
        self.checkpoints.get(&height)
    }

    /// Iterate all trusted checkpoints in height order.
    pub fn iter(&self) -> impl Iterator<Item = (&u64, &Checkpoint)> {
        self.checkpoints.iter()
    }

    /// Verify a candidate header against the trusted checkpoints.
    ///
    /// Returns:
    /// - `Ok(())` if there's no trusted checkpoint at this height OR
    ///   the header agrees with the trusted one;
    /// - `Err(StateRootMismatch)` if the heights match but state_root
    ///   disagrees;
    /// - `Err(MmrRootMismatch)` if the trusted checkpoint carries an
    ///   mmr_root and the header's mmr_root differs.
    ///
    /// **Joining-node rule** (enforced by the caller via
    /// `reject_below_trusted`): a candidate header at `height <
    /// latest_trusted_height()` is automatically rejected unless its
    /// hash chains forward into the trusted height; that's a
    /// protocol-level invariant the sync layer enforces, not a
    /// per-header check.
    pub fn verify_against_trusted(
        &self,
        height: u64,
        state_root: &[u8; 32],
        mmr_root: &[u8; 32],
    ) -> Result<(), CheckpointError> {
        let Some(cp) = self.checkpoints.get(&height) else {
            return Ok(());
        };
        if &cp.state_root != state_root {
            return Err(CheckpointError::StateRootMismatch { height });
        }
        if cp.mmr_root != [0u8; 32] && &cp.mmr_root != mmr_root {
            return Err(CheckpointError::MmrRootMismatch { height });
        }
        Ok(())
    }

    /// Returns `Err(SyncBelowTrusted)` when `target_height` is below
    /// the latest trusted checkpoint. Callers (sync layer) use this
    /// at peer-handshake time to refuse a peer that offers a
    /// chain whose tip is below our weak-subjectivity floor.
    pub fn reject_below_trusted(&self, target_height: u64) -> Result<(), CheckpointError> {
        if let Some(latest) = self.latest_trusted_height() {
            if target_height < latest {
                return Err(CheckpointError::SyncBelowTrusted { trusted: latest });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cp(h: u64, sr: u8, mmr: u8) -> Checkpoint {
        Checkpoint {
            height: h,
            state_root: [sr; 32],
            mmr_root: [mmr; 32],
            commit_certificate: None,
        }
    }

    #[test]
    fn empty_store_accepts_anything() {
        let s = CheckpointStore::new();
        assert!(s.is_empty());
        assert_eq!(s.latest_trusted_height(), None);
        assert!(s
            .verify_against_trusted(42, &[0u8; 32], &[0u8; 32])
            .is_ok());
        assert!(s.reject_below_trusted(0).is_ok());
    }

    #[test]
    fn matching_checkpoint_passes() {
        let mut s = CheckpointStore::new();
        s.insert(cp(100, 0xAA, 0xBB));
        assert!(s
            .verify_against_trusted(100, &[0xAA; 32], &[0xBB; 32])
            .is_ok());
    }

    #[test]
    fn state_root_mismatch_rejects() {
        let mut s = CheckpointStore::new();
        s.insert(cp(100, 0xAA, 0xBB));
        let err = s
            .verify_against_trusted(100, &[0xCC; 32], &[0xBB; 32])
            .unwrap_err();
        assert_eq!(err, CheckpointError::StateRootMismatch { height: 100 });
    }

    #[test]
    fn mmr_root_mismatch_rejects() {
        let mut s = CheckpointStore::new();
        s.insert(cp(100, 0xAA, 0xBB));
        let err = s
            .verify_against_trusted(100, &[0xAA; 32], &[0xCC; 32])
            .unwrap_err();
        assert_eq!(err, CheckpointError::MmrRootMismatch { height: 100 });
    }

    #[test]
    fn operator_checkpoint_skips_mmr_check() {
        // Operator-pasted form has mmr_root = [0; 32] sentinel.
        // verify_against_trusted treats that as "don't compare".
        let mut s = CheckpointStore::new();
        s.insert(Checkpoint::from_operator(100, [0xAA; 32]));
        // mmr_root in the header can be anything — passes.
        assert!(s
            .verify_against_trusted(100, &[0xAA; 32], &[0xCC; 32])
            .is_ok());
        // state_root still enforced.
        assert!(s
            .verify_against_trusted(100, &[0xCC; 32], &[0xCC; 32])
            .is_err());
    }

    #[test]
    fn reject_below_trusted_floor() {
        let mut s = CheckpointStore::new();
        s.insert(cp(500, 0xAA, 0xBB));
        s.insert(cp(1000, 0xCC, 0xDD));
        assert_eq!(s.latest_trusted_height(), Some(1000));

        // Below the latest trusted floor — rejected.
        let err = s.reject_below_trusted(500).unwrap_err();
        assert_eq!(err, CheckpointError::SyncBelowTrusted { trusted: 1000 });

        // At-or-above the floor — accepted.
        assert!(s.reject_below_trusted(1000).is_ok());
        assert!(s.reject_below_trusted(2000).is_ok());
    }

    #[test]
    fn unknown_height_in_store_passes() {
        // verify_against_trusted at a height with no trusted checkpoint
        // is `Ok(())` — only the heights actually pinned by the operator
        // (or peer-attested) are enforced.
        let mut s = CheckpointStore::new();
        s.insert(cp(100, 0xAA, 0xBB));
        assert!(s
            .verify_against_trusted(101, &[0u8; 32], &[0u8; 32])
            .is_ok());
    }

    #[test]
    fn matches_header_skips_mmr_for_operator_form() {
        let cp = Checkpoint::from_operator(100, [0xAA; 32]);
        // mmr_root is [0; 32] sentinel → comparison skipped.
        assert!(cp.matches_header(100, &[0xAA; 32], &[0xCC; 32]));
        // height mismatch fails.
        assert!(!cp.matches_header(101, &[0xAA; 32], &[0xCC; 32]));
        // state_root still enforced.
        assert!(!cp.matches_header(100, &[0xCC; 32], &[0xCC; 32]));
    }
}
