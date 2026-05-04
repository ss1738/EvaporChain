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
pub use testament::{
    MemorialMarker, Testament, TestamentError, TestamentId, TestamentStatus,
};
pub use vault::{SealedVault, VaultError};
