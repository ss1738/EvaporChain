//! Singh-Counsel — chat-first AI wallet primitive.
//!
//! ## What this is
//!
//! A wallet primitive where user actions are declared as
//! **intents**: structured `(verb, object, constraint)` triples
//! derived from natural-language conversations. Each intent
//! carries:
//!
//! - `deadline_epoch` — past which the intent is auto-abandoned.
//! - `energy_budget` — total energy the chain may spend
//!   executing this intent.
//! - `min_confidence_bp` — the chain refuses to execute an
//!   intent whose AI-attestation confidence is below floor.
//!
//! The chain validates each intent against an **intent
//! grammar** (verb in known set, object well-formed, constraint
//! parseable) BEFORE executing. Chat noise that doesn't parse is
//! rejected, never executed-blindly.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Grammar-validated intents.** Unknown verbs, malformed
//!    objects, and unparseable constraints are rejected with
//!    `BadIntent`. The wallet never executes a free-form string.
//!
//! 2. **Deadline + budget structurally enforced.** An intent
//!    past its deadline is auto-abandoned without execution;
//!    an intent whose execution-side computation would exceed
//!    its budget is rejected up-front.
//!
//! 3. **Confidence floor.** AI-derived intents carry an
//!    attestation confidence (basis points). Below floor →
//!    rejected. The chain doesn't trust low-confidence
//!    chat-to-intent translations.
//!
//! ## What this crate does NOT do
//!
//! - Does NOT run the LLM. Caller passes the parsed intent +
//!   confidence. Chain validates the structure.
//! - Does NOT execute the actual wallet operation. Hand-off to
//!   the chain's transaction layer with the validated intent.
//! - Does NOT model multi-turn dialogue. V1 is one intent
//!   per submission.
//!
//! ## Module map
//!
//! - [`intent`] — [`Intent`] + [`Verb`] + grammar validation.

pub mod intent;

pub use intent::{Intent, IntentError, IntentId, Verb, KNOWN_VERBS};
