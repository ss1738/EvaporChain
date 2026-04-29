//! Information-Bottleneck Validators — A4.3.1.
//!
//! # Doctrine
//!
//! From INVENTION_STACK.md A4.3.1 (PROMISING):
//! > β = λ. Validators act as IB compressors: they compress chain state X
//! > into a bottleneck Z that retains maximum mutual information with future
//! > state Y.  The Lagrange multiplier β controlling compression vs relevance
//! > is tied to the chain's single λ.
//!
//! # Tishby-Pereira-Bialek 1999
//!
//! The IB objective (to maximise) is:
//!
//! ```text
//! L[p(Z|X)] = I(Z; Y) − β × I(Z; X)
//! ```
//!
//! - `I(Z; X)` = bits the bottleneck Z retains about the input X (compression
//!   cost).
//! - `I(Z; Y)` = bits Z retains about the output/future Y (relevance).
//! - `β` = Lagrange multiplier; small β = coarse compression; large β = fine.
//!
//! # Chain instantiation
//!
//! - **X** = a validator's local view of current chain state (recent account
//!   energy histogram, represented as a `StateSignature` — discretised into
//!   `N_BINS` buckets).
//! - **Y** = the *committed* chain root hash at the next checkpoint (the
//!   target a validator is trying to predict / agree on).
//! - **Z** = the validator's *vote* — a compressed representation.  We use a
//!   1-bit vote (`commit` / `abstain`) as the simplest bottleneck.
//! - **β** = `lambda_mb / 1_000_000` (λ in millibits → fraction).  As λ
//!   decreases the validator set becomes more selective (higher compression).
//!
//! # Practical computation
//!
//! For a 1-bit Z, the IB objective simplifies to choosing the vote that
//! maximises the mutual information gain.  We approximate this as:
//!
//! ```text
//! vote = commit  iff  I_gain(state_sig) > β_threshold
//! ```
//!
//! where `I_gain` is the per-validator *relevance score* computed from
//! how closely the validator's state signature matches the prior distribution
//! of state signatures that historically led to consensus.

pub mod ib;
pub mod signature;

pub use ib::{IbParams, IbVote, ib_vote};
pub use signature::{StateSignature, N_BINS};
