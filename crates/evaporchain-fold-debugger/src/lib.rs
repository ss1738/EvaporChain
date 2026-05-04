//! Folded-State Debugger.
//!
//! ## What this is
//!
//! A rewind tool that reconstructs the chain's state at any past
//! height. Instead of block-by-block replay (O(N) for N blocks),
//! it walks a *fold tree* of energy-folded snapshots (Nova-style):
//! the snapshot at height `2^k · m` carries enough state to fold
//! the next `2^k` blocks of input. Replay depth: O(log N) folds.
//!
//! ## Fold tree structure
//!
//! Snapshots are committed at heights `0, 1, 2, 4, 8, …, 2^k, …`
//! (and at the chain's current head). To rewind to height `h`:
//! 1. Find the largest snapshot ≤ h.
//! 2. Apply log₂(h - snapshot_height) further fold steps.
//! 3. The terminal state = fold(snapshot, replay_steps).
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Determinism.** Same height + same fold tree ⇒ byte-
//!    identical reconstructed state. Validators agree on
//!    debugger output if they agree on the fold tree.
//!
//! 2. **Snapshots are content-addressed.** Snapshot id =
//!    BLAKE3(domain_tag || height || state_root). Tampering with
//!    a snapshot changes its id, so a debugger walking a
//!    tampered tree silently produces an incorrect state — the
//!    chain compares the reconstructed state's hash against the
//!    canonical root and surfaces the divergence.
//!
//! 3. **Rewind to a height beyond the head fails closed.** The
//!    debugger refuses to extrapolate; it only RE-derives past
//!    state, never invents future state.
//!
//! ## What this crate does NOT do
//!
//! - Does NOT *generate* the fold-step proofs. The chain's
//!   proving layer (e.g., evaporchain-lambda-fold) does that.
//!   This crate consumes folds.
//! - Does NOT verify Nova circuits. Caller passes
//!   pre-validated fold steps.
//! - Does NOT model branching reorgs. V1 assumes a single
//!   canonical chain.
//!
//! ## Module map
//!
//! - [`snapshot`] — [`Snapshot`] + content-addressed id.
//! - [`fold_tree`] — [`FoldTree`] of snapshots + replay step
//!   sequences.
//! - [`rewind`] — [`Debugger`] driver: rewind-to-height API.

pub mod fold_tree;
pub mod rewind;
pub mod snapshot;

pub use fold_tree::{FoldStep, FoldTree, FoldTreeError};
pub use rewind::{rewind_to, Debugger, RewindError};
pub use snapshot::{snapshot_id, Snapshot, SNAPSHOT_TAG};
