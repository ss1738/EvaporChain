//! The Gallery That Forgets.
//!
//! Per `research/INVENTION_STACK.md` §A2.3 — cultural-launch wedge.
//!
//! > One launch artifact, three primitives:
//! >
//! > 1. **Provably-Mortal NFTs ("Mayflies")** — minted with declared
//! >    half-life + ZK death certificate. Wallet shows literal countdown.
//! > 2. **Decay-as-Performance-Art** — gallery contract; artists
//! >    deposit works with chosen half-lives; gallery's visual state
//! >    changes daily. Closing date is *thermodynamic*.
//! > 3. **AI-Decay-Art** — generative pieces taking chain-energy as
//! >    runtime parameter; output literally changes as state evaporates.
//! >    Basinski's Disintegration Loops on-chain.
//!
//! > **The single sentence:**
//! > *"It is the first thing humans have made that is provably going to die."*
//!
//! ## What this crate ships
//!
//! Three modules that compose Singh-Sabi (already shipped) + a new
//! gallery orchestration layer + Mayfly NFTs (declared half-life,
//! ZK-style cryptographic death certificate):
//!
//! - **Mayfly** ([`mayfly`]) — `MayflyToken` is a thin wrapper around
//!   a Singh-Sabi `PatinaToken` whose `ruined_beautiful` floor is
//!   pinned at zero (genuine evaporation, not asymptote — *mayflies
//!   actually die*). Mints emit a `DeathCertificate` (Blake3 commitment
//!   to mint inputs + projected death epoch) at mint time, before the
//!   token has died. The certificate is the on-chain "this NFT
//!   provably will die" guarantee.
//!
//! - **Gallery** ([`gallery`]) — `Gallery` aggregates exhibits, tracks
//!   "thermodynamic closing date" = epoch at which *all* exhibits have
//!   passed their projected death. Adds/removes exhibits, queries the
//!   gallery's overall state at any epoch. The aggregate state changes
//!   from `Open` → `Closing` (≥1 exhibit dead) → `Closed` (all dead).
//!
//! - **AI-Decay-Art** ([`ai_decay`]) — runtime-parameter helper that
//!   surfaces the current energy as a *seed scalar* the off-chain
//!   shader consumes. The chain doesn't render pixels; it publishes a
//!   single `f64` that the artist's GLSL/WGSL shader takes as input.
//!   The art "literally changes" each block because the seed scalar
//!   does.
//!
//! ## Module map
//!
//! - [`mayfly`] — [`MayflyToken`], [`DeathCertificate`].
//! - [`gallery`] — [`Gallery`], [`GalleryStatus`], [`Exhibit`].
//! - [`ai_decay`] — [`runtime_seed`], [`AiSeedSnapshot`].

pub mod ai_decay;
pub mod gallery;
pub mod mayfly;

pub use ai_decay::{runtime_seed, AiSeedSnapshot};
pub use gallery::{Exhibit, ExhibitId, Gallery, GalleryError, GalleryStatus};
pub use mayfly::{DeathCertificate, MayflyError, MayflyToken};

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use evaporchain_types::AccountAddress;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Mayflies actually DIE — the patina floor is
    /// pinned to ZERO (no ruined-beautiful asymptote). Mint emits a
    /// DeathCertificate that anyone can re-verify against the token's
    /// stored fields. Transfer of a dead token rejects; self-transfer
    /// rejects; certificate tampering breaks verification."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let alice: AccountAddress = [0xAAu8; 32];
        let bob: AccountAddress = [0xBBu8; 32];

        // Mint a short-lived mayfly: initial=1024, half_life=10.
        let mut m = MayflyToken::mint([1u8; 32], alice, 1_024, 10, 0).unwrap();
        assert!(m.verify_certificate(), "fresh certificate must verify");

        // At mint epoch: full energy, alive.
        assert_eq!(m.score_at(0), 1_024);
        assert!(!m.is_dead(0));

        // Long after the projected death horizon: actually 0
        // (mayfly floor is zero — not the 15% Singh-Sabi default).
        assert!(m.is_dead(10_000));
        assert_eq!(m.score_at(10_000), 0);

        // Transfer while alive succeeds.
        m.transfer(alice, bob, 0).unwrap();
        assert_eq!(m.owner, bob);

        // Self-transfer rejected.
        assert!(matches!(
            m.transfer(bob, bob, 0),
            Err(MayflyError::SelfTransfer)
        ));

        // Transfer of a dead token rejected.
        let carol: AccountAddress = [0xCCu8; 32];
        assert!(matches!(
            m.transfer(bob, carol, 10_000),
            Err(MayflyError::Died)
        ));

        // Tampered certificate fails verification.
        let mut tampered = m.clone();
        tampered.certificate.commitment[0] ^= 0xFF;
        assert!(!tampered.verify_certificate());
    }
}
