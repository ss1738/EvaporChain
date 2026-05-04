//! Singh-Resonance (Vital-Sign NFTs).
//!
//! Per `research/INVENTION_STACK.md` §A5.3:
//!
//! > λ inversely coupled to engagement (views, on-chain reactions,
//! > transfers, derivative mints). Loved art slows toward immortality;
//! > ignored art accelerates toward zero.
//!
//! > **Pitch:** maps directly to attention-economy critique (Tristan
//! > Harris, Jenny Odell *How to Do Nothing*). Risk: looks like
//! > "Black Mirror but on chain" — needs careful framing as critique.
//!
//! ## What's structurally different from existing NFTs
//!
//! Every other "engagement-coupled" art experiment has been *additive*
//! — likes accumulate forever, scoring a kind of permanent celebrity.
//! Singh-Resonance is **subtractive by default**: tokens decay; the
//! only way to keep them alive is to be witnessed. The math runs both
//! directions:
//!
//! - **Engagement *windows* matter, not cumulative counts.** A
//!   firework of engagement followed by silence still leads to death.
//!   Sustained moderate engagement is what slows λ.
//! - **The chain doesn't store a global like-counter.** It stores a
//!   recent-engagement *energy* that itself decays — a moving average
//!   the chain enforces on attention. Yesterday's likes evaporate just
//!   like the token they're trying to save.
//!
//! V1 ships with **explicit on-chain engagement events** — callers
//! invoke `register_engagement(token_id, weight)` to deposit
//! attention. The weight is a chain-controlled scalar (1 for a view, N
//! for a derivative mint, etc.). No external social graph. Future
//! versions can route engagement through a Lens-equivalent in-stack
//! social graph crate; the math doesn't change.
//!
//! ## The framing
//!
//! Doctrine flags the Black-Mirror risk explicitly. The right framing
//! is **attention-economy critique**: this is not "art that demands
//! likes," it is "art that exposes which art we are actually
//! sustaining with attention vs which we have abandoned." The decay
//! is the witness, not the punishment.
//!
//! ## Module map
//!
//! - [`engagement`] — [`EngagementWindow`] decaying attention-budget;
//!   `register` and `attention_at` accessors. **The whole crate's
//!   novelty lives here.**
//! - [`coupling`] — [`effective_half_life`]: how engagement scales the
//!   token's λ. Closed-form, integer-only.
//! - [`token`] — [`ResonanceToken`], [`TokenError`]; mint, register
//!   engagement, witness, transfer.

pub mod coupling;
pub mod engagement;
pub mod token;

pub use coupling::{effective_half_life, CouplingError, CouplingParams};
pub use engagement::{EngagementWindow, EngagementWindowError};
pub use token::{ResonanceToken, TokenError, TokenId};

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Singh-Resonance attention is **subtractive**:
    /// the EngagementWindow is itself a decaying energy. Yesterday's
    /// engagement evaporates just like the token it's trying to save.
    /// Backwards time is rejected; zero half-life is rejected at
    /// construction; lifetime_total is a non-decaying audit trail."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let mut w = EngagementWindow::new(10, 0).unwrap();

        // Fresh window: zero attention.
        assert_eq!(w.attention_at(0), 0);

        // Register 1000 units at epoch 0.
        w.register(0, 1_000).unwrap();
        assert_eq!(w.attention_at(0), 1_000);
        assert_eq!(w.lifetime_total, 1_000);

        // After one half-life (10 epochs), attention has decayed by half.
        let attn_h1 = w.attention_at(10);
        assert!(attn_h1 < 1_000, "attention must decay over time");
        assert!(attn_h1 <= 500 + 50, "≈ 500 after one half-life");

        // After two half-lives, decayed by ≈ 4×.
        let attn_h2 = w.attention_at(20);
        assert!(attn_h2 < attn_h1, "attention monotonically decreasing");

        // Backwards time → typed error.
        let err = w.register(0, 100); // anchor moved to 0; epoch_now=0 OK
        assert!(err.is_ok());
        // Now anchor is at 0, lifetime_total bumped.
        assert_eq!(w.lifetime_total, 1_100);
        // Try epoch BEFORE the new anchor → reject. First advance anchor.
        w.register(50, 1).unwrap();
        let backwards = w.register(10, 1);
        assert!(matches!(
            backwards,
            Err(EngagementWindowError::BackwardsTime { .. })
        ));

        // Zero half-life rejected at construction.
        assert!(matches!(
            EngagementWindow::new(0, 0),
            Err(EngagementWindowError::ZeroHalfLife)
        ));
    }
}
