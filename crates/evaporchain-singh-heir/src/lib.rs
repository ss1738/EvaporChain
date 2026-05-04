//! Singh-Heir (Patrilithic Tokens) — kin-graph heirloom NFTs.
//!
//! ## What this is
//!
//! Holders declare an **ordered kin list** (preferred heirs in
//! priority order). On certified death of the holder, the token
//! transfers to the highest-ranked living heir who is still
//! alive and has not refused inheritance.
//!
//! Each inheritance hop applies an **inheritance-tax half-life**
//! to the token's energy: `new_energy = old_energy / 2`. Across
//! generations, the heirloom value decays geometrically unless
//! heirs actively reinforce (refresh) the energy. The chain's
//! single-λ binds heirloom longevity to active engagement.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Heirs ordered, deterministic.** The kin list is a Vec
//!    in priority order. Validators agree on the next heir.
//!
//! 2. **Inheritance halves energy.** No exceptions. Heirs
//!    cannot bypass the tax; they can only refresh AFTER they
//!    inherit.
//!
//! 3. **Liveness gate on heirs.** A dead-or-decayed heir is
//!    skipped (the chain holds the certificates). Inheritance
//!    walks down the priority list until it finds a live
//!    candidate. If none live, the token is *escheated*
//!    (returned to the chain's commons pool).
//!
//! ## What this crate does NOT do
//!
//! - Does NOT verify the death certificate. Caller passes a
//!    signed cert; chain's higher layer verifies the m-of-n
//!    threshold attestation.
//! - Does NOT enforce blood / adoption distinctions. The kin
//!    list is opaque labels; the chain's higher layer can
//!    classify externally.
//! - Does NOT model joint inheritance (multiple heirs splitting).
//!    V1 single-heir transfer; V2 multi-share splits.
//!
//! ## Module map
//!
//! - [`token`] — [`HeirloomNft`] state machine.

pub mod token;

pub use token::{HeirloomNft, HeirloomError, TokenId};
