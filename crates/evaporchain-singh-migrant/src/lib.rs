//! Singh-Migrant (Wanderwrits) — NFTs that die if held still.
//!
//! Per `research/INVENTION_STACK.md` §A5.3:
//!
//! > Each NFT has *resting threshold* (~30 days). Energy decays
//! > normally; transfer to a *new* wallet refunds a fraction. Stay
//! > still past threshold → λ doubles; past 60 days → quadruples.
//! > Must keep moving through novel hands or it evaporates.
//!
//! > **Pitch:** *"the NFT that dies if you keep it."*
//!
//! ## Three structural decisions
//!
//! 1. **Effective half-life multiplier scales with rest age.** Below
//!    `resting_threshold_epochs`, decay runs at the base half-life
//!    (multiplier 1×). Past the threshold, multiplier becomes 0.5×
//!    (twice as fast = half the half-life). Past
//!    `2 * resting_threshold_epochs`, multiplier becomes 0.25×
//!    (quadruple speed). The token's apparent decay accelerates as it
//!    sits — so "stay still" really does kill it faster.
//!
//! 2. **Transfer to a NOVEL wallet is required for refund.** Not just
//!    "any transfer." The token tracks its `visited_wallets` set
//!    on-chain. Transfer to an address already in the set is a *legal
//!    move* but yields zero refund. The kula-ring metaphor holds: the
//!    object circulates; it doesn't bounce.
//!
//! 3. **Refund is a fraction of *current* energy, not initial.** A
//!    near-dead token gets a small refund; a fresh one gets a big
//!    one. This kills the trivial farm-and-relay attack: holding it
//!    until it's nearly dead and then circulating doesn't restore
//!    much.
//!
//! Cultural lineage: Trobriand kula ring (Malinowski 1922), chain
//! letters, Olympic torch, Marcel Mauss *The Gift* (1925).
//!
//! ## Module map
//!
//! - [`decay`] — [`effective_half_life`]: piecewise multiplier;
//!   [`current_energy`]: energy at `epoch_now` given `(initial,
//!   half_life, rested_at, threshold)`.
//! - [`refund`] — [`refund_amount`]: fraction-of-current refund on
//!   novel-wallet transfer.
//! - [`token`] — [`MigrantToken`], [`TokenId`]; mint, transfer,
//!   witness.

pub mod decay;
pub mod refund;
pub mod token;

pub use decay::{current_energy, effective_half_life, DecayError};
pub use refund::{refund_amount, RefundError, REFUND_FRACTION_PCT};
pub use token::{MigrantToken, TokenError, TokenId, TransferOutcome};
