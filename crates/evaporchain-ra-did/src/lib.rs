//! Reinforced-Attribute Decentralized Identifier (RA-DID).
//!
//! ## What this crate is
//!
//! A DID type where each *attribute* (a typed claim like
//! "age-verified by issuer X", "KYC-tier 2", "professional-
//! credential PE 2024") carries its own energy budget. Attributes
//! whose energy decays below the verifier-supplied floor are
//! structurally non-presentable: even a holder with the right DID
//! key cannot construct a valid presentation that includes a
//! decayed attribute.
//!
//! ## Anti-Sybil property
//!
//! Standard DIDs let an attacker spin up N synthetic identities
//! cheaply (just generate keypairs). Each fresh DID is fully
//! capable from the moment it's minted.
//!
//! RA-DID inverts this: a fresh DID has no attributes. Each
//! attribute requires:
//! 1. An issuer attestation (the chain's higher layer, not in scope here).
//! 2. Energy at attestation time, which decays over the chain's λ.
//!
//! To match the attribute set of a real long-lived DID, an
//! attacker has to either (a) earn every attribute at full
//! attestation cost on every fake identity, or (b) wait for
//! attribute decay to expire — at which point the fake
//! attestations don't match either.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Attributes are energy-tagged independently.** A single
//!    DID can have some attributes live and some decayed; the
//!    presentation gate enforces per-attribute thresholds.
//!
//! 2. **Selective disclosure is structural.** A presentation
//!    discloses a subset of attribute-keys; the gate verifies
//!    each disclosed attribute is above its verifier floor. No
//!    "all or nothing" — attribute-bundling at the application
//!    layer.
//!
//! 3. **Issuer attestation binds attribute hash + energy.** An
//!    attribute's commitment hash includes both the value AND
//!    the issuance energy, so a holder cannot present a
//!    "boosted-energy" copy of an old attestation.
//!
//! ## What this crate does NOT do
//!
//! - It does NOT verify issuer signatures. Caller passes
//!   `issuer_pubkey` as part of the attestation; signature
//!   verification is the chain's transaction layer.
//! - It does NOT model attribute revocation lists. Decay is the
//!   only revocation mechanism in V1.
//! - It does NOT model nested / hierarchical attributes (e.g.,
//!   "KYC-tier-2 implies KYC-tier-1"). V1 is flat.
//!
//! ## Module map
//!
//! - [`attr`] — [`Attribute`] + [`AttributeKey`] +
//!   energy-binding hash.
//! - [`did`] — [`Did`] document + presentation construction.
//! - [`verify`] — [`verify_presentation`] gate.

pub mod attr;
pub mod did;
pub mod verify;

pub use attr::{attribute_hash, Attribute, AttributeKey, ATTRIBUTE_TAG};
pub use did::{Did, DidError, DidId, Presentation};
pub use verify::{verify_presentation, VerifyError};
