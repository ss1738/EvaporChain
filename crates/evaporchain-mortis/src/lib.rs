//! Mortis — the chain's final-death act.
//!
//! Per `research/INVENTION_STACK.md` Amendment 2 §A2.5:
//!
//! > **Mortis** — when refresh pool falls below ε for N epochs, the
//! > final state root is auto-minted as a single unowned NFT visible
//! > to all light clients forever.
//! >
//! > *"The first blockchain that signs its own death certificate."*
//!
//! ## Why this exists
//!
//! Bitcoin promises immortality and quietly fails. EvaporChain
//! promises mortality and provably succeeds — by writing its own
//! death certificate the moment it can no longer fund its
//! keep-alive (refresh pool < ε for N epochs in a row).
//!
//! The death certificate is unowned, immutable, visible to every
//! light client forever. It pairs with [`evaporchain_tombstone`]'s
//! eulogy trie: the tombstones are the chain's small deaths; mortis
//! is its final one.
//!
//! ## Substrate
//!
//! - [`condition`] — `MortisCondition { refresh_pool_floor,
//!   sustained_epochs }` — chain-set governance parameters.
//! - [`monitor`] — `MortisMonitor` tracks the consecutive-epochs
//!   count below threshold; `tick(epoch, refresh_pool_total)`
//!   advances and `is_triggered()` returns true once the run hits
//!   `sustained_epochs`.
//! - [`certificate`] — `MortisCertificate { final_state_root,
//!   epoch_of_death, eulogy_trie_root, witness }` — the singleton
//!   death-certificate NFT the chain mints on the trigger.

pub mod certificate;
pub mod condition;
pub mod monitor;

pub use certificate::{mint_certificate, CertificateError, MortisCertificate};
pub use condition::MortisCondition;
pub use monitor::{MortisMonitor, TickOutcome};

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Mortis triggers when the refresh pool stays at-
    /// or-below ε for N consecutive epochs. The trigger is latched —
    /// once fired, subsequent ticks are no-ops and the chain has
    /// signed its own death certificate."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let cond = MortisCondition::new(1_000, 3);
        let mut m = MortisMonitor::new(cond);
        let r = m.tick(1, 5_000);
        assert!(matches!(r, TickOutcome::Healthy));
        assert!(!m.is_triggered());

        let _ = m.tick(2, 500);
        let _ = m.tick(3, 500);
        let r = m.tick(4, 500);
        assert!(matches!(r, TickOutcome::JustTriggered));
        assert!(m.is_triggered());

        let r = m.tick(5, 999_999_999);
        assert!(matches!(r, TickOutcome::AlreadyTriggered));
        assert!(m.is_triggered());
    }
}
