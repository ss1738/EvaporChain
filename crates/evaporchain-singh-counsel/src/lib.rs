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

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Singh-Counsel never executes free-form chat:
    /// every intent is grammar-validated at construction (verb in
    /// known set, object non-empty + bounded, constraint parseable)
    /// AND admission-gated at execution (deadline, confidence floor,
    /// energy budget). Anti-replay commitment changes when any
    /// field changes."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let id = IntentId([1u8; 32]);
        let holder = [2u8; 32];

        // Honest intent: valid Send, non-empty object, parseable
        // constraint, future deadline, positive budget, high
        // confidence.
        let intent = Intent::new(
            id,
            holder,
            Verb::Send,
            vec![3u8; 32],          // recipient address
            b"{}".to_vec(),         // minimal valid constraint
            1_000,                  // deadline_epoch
            500,                    // energy_budget
            9_500,                  // 95% confidence
        )
        .unwrap();

        // Admission at now=10 with floor=5000, required=400 → OK.
        intent.admit(10, 5_000, 400).unwrap();

        // Past deadline rejected.
        assert!(matches!(
            intent.admit(2_000, 5_000, 400),
            Err(IntentError::DeadlinePassed { .. })
        ));
        // Below confidence floor rejected.
        assert!(matches!(
            intent.admit(10, 9_999, 400),
            Err(IntentError::LowConfidence { .. })
        ));
        // Over budget rejected.
        assert!(matches!(
            intent.admit(10, 5_000, 600),
            Err(IntentError::BudgetExceeded { .. })
        ));

        // Construction guards.
        // Empty object → rejected.
        assert!(matches!(
            Intent::new(id, holder, Verb::Send, vec![], b"{}".to_vec(), 1, 1, 1),
            Err(IntentError::EmptyObject)
        ));
        // Object too long (>1024) → rejected.
        assert!(matches!(
            Intent::new(id, holder, Verb::Send, vec![0u8; 2_000], b"{}".to_vec(), 1, 1, 1),
            Err(IntentError::ObjectTooLong)
        ));
        // Zero deadline → rejected.
        assert!(matches!(
            Intent::new(id, holder, Verb::Send, vec![1], b"{}".to_vec(), 0, 1, 1),
            Err(IntentError::ZeroDeadline)
        ));

        // Anti-replay: changing the verb changes the commitment.
        let mut other = intent.clone();
        other.verb = Verb::Stake;
        assert_ne!(intent.commitment(), other.commitment());

        // KNOWN_VERBS coverage: every variant in the constant.
        assert!(KNOWN_VERBS.contains(&Verb::Send));
        assert!(KNOWN_VERBS.contains(&Verb::Stake));
        assert!(KNOWN_VERBS.contains(&Verb::Reanchor));
    }
}
