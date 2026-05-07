//! Singh-Posthuma (Sealed Testaments).
//!
//! Per `research/INVENTION_STACK.md` §A5.3:
//!
//! > Mint commits encrypted payload. Decryption key held by
//! > threshold-secret-sharing committee. Decay suspended while issuer
//! > is verifiably alive. On confirmed death, committee reveals key
//! > → payload becomes public → λ-decay begins on the now-public
//! > NFT → fades to permanent on-chain marker.
//!
//! > **Cultural lineage:** Catholic confessional seal; Pessoa's
//! > trunk; Kafka's Brod betrayal; Joan Didion's *Year of Magical
//! > Thinking*.
//!
//! > **Pitch:** *"the first NFT that's a deathbed confession."*
//! > Highest mainstream-press potency of the NFT set. *New Yorker*-grade.
//!
//! ## Three structural decisions
//!
//! 1. **Decay is *suspended* while alive, not just paused.** The
//!    chain doesn't decay testaments at all until a death certificate
//!    is honored. The half-life clock starts the moment the death is
//!    certified, not the mint epoch. This is what distinguishes a
//!    *testament* from a "scheduled-reveal" capsule — it lives as
//!    long as the issuer does.
//!
//! 2. **The payload is opaque on-chain.** Only the BLAKE3 hash of the
//!    ciphertext + the threshold-attestation committee + the public
//!    decryption-key commitment go on chain. The actual encrypted
//!    blob lives off-chain (IPFS / a custodial CDN). Validators agree
//!    on the *commitment*, never on the contents.
//!
//! 3. **Death is multisig-attested in V1.** The committee is a list
//!    of validator addresses; an `M`-of-`N` threshold of them sign a
//!    `DeathCertificate` (over the testament id + death epoch + a
//!    chain-supplied nonce). The certificate is verified
//!    pure-function; once accepted, the testament transitions
//!    `Sealed → Revealed` and the decay clock starts. Future versions
//!    can plug in a real death-oracle without changing the lifecycle.
//!
//! ## After reveal: graceful fade to a permanent marker
//!
//! Once revealed, the testament's `λ` (half-life) governs how long
//! the public ciphertext-pointer stays "fresh" in the chain's
//! retention layer. After the half-life elapses, the ciphertext
//! commitment fades to a `MemorialMarker` — 32 bytes of metadata that
//! permanently attest "a testament existed, was issued by X, was
//! revealed at Y, and has been read." The marker stays on chain
//! forever; the readable form decays.
//!
//! Doctrine framing: a confession seal that opens once, is read by
//! whoever was meant to read it, and fades. The chain holds the
//! memory of the confession ever having existed; the words themselves
//! are not eternal.
//!
//! ## Module map
//!
//! - [`vault`] — [`SealedVault`] payload commitment + threshold
//!   committee + public-key commitment.
//! - [`certificate`] — [`DeathCertificate`] threshold-attestation;
//!   pure-function verifier.
//! - [`testament`] — [`Testament`] lifecycle: Sealed → Revealed →
//!   Memorial. Issuer-locked; reveal idempotent only in the sense
//!   that re-revealing a Revealed/Memorial testament errors.

pub mod certificate;
pub mod testament;
pub mod vault;

pub use certificate::{verify_certificate, Attestation, CertificateError, DeathCertificate};
pub use testament::{MemorialMarker, Testament, TestamentError, TestamentId, TestamentStatus};
pub use vault::{SealedVault, VaultError};

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use evaporchain_types::AccountAddress;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Singh-Posthuma testaments suspend decay while
    /// Sealed (issuer presumed alive). Visible energy stays at the
    /// initial value indefinitely. After a verified DeathCertificate,
    /// status flips Sealed → Revealed and the half-life clock starts
    /// from `cert.death_epoch` — not from mint, not from now.
    /// Re-revealing a Revealed testament fails closed."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let issuer: AccountAddress = [1u8; 32];
        let committee: Vec<AccountAddress> = (10u8..15u8).map(|b| [b; 32]).collect();

        let vault = SealedVault::new(
            [0xAA; 32], // ciphertext_hash
            1024,       // ciphertext_len
            3,          // 3-of-5 threshold
            committee, [0xBB; 32], // pubkey_commitment
        )
        .unwrap();
        assert_eq!(vault.committee_size(), 5);

        let mut t = Testament::seal([7u8; 32], issuer, vault, 100, 1_000, 0).unwrap();

        // Sealed: decay is suspended — visible energy stays at initial
        // even after many half-lives elapse.
        assert!(t.is_sealed());
        assert_eq!(t.visible_energy_at(0), 1_000);
        assert_eq!(t.visible_energy_at(10_000), 1_000);

        // Pre-reveal fade rejected.
        assert!(t.fade_to_memorial(10_000).is_err());

        // Construction guards on the vault.
        assert!(matches!(
            SealedVault::new([0u8; 32], 1, 1, vec![[1u8; 32]], [0xBB; 32]),
            Err(VaultError::EmptyCiphertextHash)
        ));
        assert!(matches!(
            SealedVault::new([0xAA; 32], 0, 1, vec![[1u8; 32]], [0xBB; 32]),
            Err(VaultError::ZeroCiphertextLen)
        ));
        assert!(matches!(
            SealedVault::new([0xAA; 32], 1, 5, vec![[1u8; 32]], [0xBB; 32]),
            Err(VaultError::ThresholdAboveCommittee { .. })
        ));

        // Testament construction guards.
        let v2 = SealedVault::new([0xAA; 32], 1, 1, vec![[1u8; 32]], [0xBB; 32]).unwrap();
        assert!(matches!(
            Testament::seal([0u8; 32], issuer, v2.clone(), 0, 1_000, 0),
            Err(TestamentError::ZeroHalfLife)
        ));
        assert!(matches!(
            Testament::seal([0u8; 32], issuer, v2, 100, 0, 0),
            Err(TestamentError::ZeroInitialEnergy)
        ));

        // Sanity: a sealed testament is neither Revealed nor Memorial.
        assert!(!t.is_revealed());
        assert!(!t.is_memorial());
    }
}
