//! Singh Skill Half-Life Market (SHLM).
//!
//! Per `research/INVENTION_STACK.md` §A5.2:
//!
//! > Skill credentials decay at skill-specific rates (Python λ ≈ 18mo,
//! > COBOL λ ≈ 8yr, prompt-engineering λ ≈ 4mo). Holders refresh via
//! > micro-assessments. Employers post bounties for fresh-skill tokens
//! > above decay threshold.
//!
//! > **Pitch:** *"Your Python skills now expire — and employers can see
//! > the timestamp."* Rides AI-displacement wave perfectly.
//! > $50B B2B TAM (recruiter market).
//!
//! ## What's the actual mechanism?
//!
//! 1. **Skill Class** — a registered taxonomy entry (e.g. "Python", "COBOL",
//!    "prompt-engineering"). Each class has its own λ (decay rate) baked
//!    in at registration. The λ is what makes SHLM structurally distinct
//!    from existing skill-credential systems: a Python cert from 2018
//!    decays faster than a COBOL cert from 2010 *because the underlying
//!    skill itself decays faster*.
//! 2. **Credential** — issued by an attestor (the equivalent of "exam
//!    board" or "previous employer") to a holder, with a `level` score
//!    and an `attested_at_epoch`. Freshness derives mechanically from
//!    `(class.lambda, attested_at, current_epoch)` via the standard
//!    half-life rule. **No central registry; no expiry transactions.**
//! 3. **Refresh** — holders can re-attest via a micro-assessment that
//!    bumps `attested_at_epoch` forward and optionally updates `level`.
//!    The assessment itself isn't on-chain (it's the attestor's
//!    business); only the resulting refreshed credential is.
//! 4. **Bounty market** — employers post bounties on `(class, min_freshness,
//!    min_level)`. Holders bid, clearing through SDDC on `(salary_offer,
//!    skill_lambda_tolerance)`. The two-axis interpretation:
//!       - SDDC's `max_price` ⇔ employer's salary cap.
//!       - SDDC's `lambda_tolerance` ⇔ how decayed-but-still-acceptable
//!         the credential can be (employers desperate for COBOL accept
//!         a more-decayed token; FAANG-tier Python employers demand
//!         very-fresh tokens).
//!
//! ## Why "biggest commercial wedge"
//!
//! Existing credential systems (LinkedIn, Coursera certificates, MOOC
//! badges, professional certifications) are *static* — once issued they
//! never expire. The pretense is that a Python cert from 2017 is
//! equivalent to one from 2025. SHLM is the first system where
//! *credentials decay at a rate that reflects how the skill itself
//! decays*. Recruiters get a portable freshness signal; holders get a
//! reason to keep re-attesting; assessment platforms get repeat
//! revenue per skill per holder per N epochs.
//!
//! ## Module map
//!
//! - [`skill_class`] — [`SkillClass`], [`SkillClassId`]; the registry entry.
//! - [`credential`] — [`Credential`]; issued + refreshable token.
//! - [`freshness`] — [`freshness_score`]: pure-function freshness derivation.
//! - [`bounty`] — [`Bounty`] employer posting + matching against credentials.
//! - [`market`] — wraps SDDC for clearing employer-bounty bids.

pub mod bounty;
pub mod credential;
pub mod freshness;
pub mod market;
pub mod skill_class;

pub use bounty::{Bounty, BountyError, BountyId};
pub use credential::{Credential, CredentialError, CredentialId};
pub use freshness::{freshness_score, FreshnessLevel};
pub use market::{post_bounty, settle_bounty, MarketError};
pub use skill_class::{SkillClass, SkillClassError, SkillClassId};
