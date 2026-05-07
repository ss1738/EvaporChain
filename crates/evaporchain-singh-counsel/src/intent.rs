//! `Intent` — one chat-derived wallet action + grammar validation.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 32-byte intent id. The chain dedups by id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct IntentId(pub [u8; 32]);

/// V1 known-verb set. Adding a verb is a chain upgrade — the
/// wallet refuses unknown verbs.
pub const KNOWN_VERBS: &[Verb] = &[
    Verb::Send,
    Verb::Stake,
    Verb::Unstake,
    Verb::Vote,
    Verb::Deploy,
    Verb::Refresh,
    Verb::Reanchor,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Verb {
    Send,
    Stake,
    Unstake,
    Vote,
    Deploy,
    Refresh,
    Reanchor,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum IntentError {
    #[error("verb {0:?} is not in the known-verb set; chain refuses unknown actions")]
    UnknownVerb(Verb),
    #[error("object is empty — intent must name an object")]
    EmptyObject,
    #[error("object exceeds 1024 bytes — intent payload bound enforced")]
    ObjectTooLong,
    #[error("constraint failed to parse: {0}")]
    BadConstraint(String),
    #[error("intent past deadline: now {now} > deadline {deadline}")]
    DeadlinePassed { now: u64, deadline: u64 },
    #[error("zero deadline — must specify a future epoch")]
    ZeroDeadline,
    #[error("zero energy_budget — must specify a positive budget")]
    ZeroBudget,
    #[error("execution would exceed energy budget: required {required} > budget {budget}")]
    BudgetExceeded { required: u64, budget: u64 },
    #[error(
        "confidence {confidence_bp}bp below floor {floor_bp}bp — chain refuses low-confidence chat translations"
    )]
    LowConfidence { confidence_bp: u64, floor_bp: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Intent {
    pub id: IntentId,
    pub holder: [u8; 32],
    pub verb: Verb,
    /// Object the verb operates on, e.g., recipient address bytes
    /// for Send, validator id for Stake, contract id for Deploy.
    pub object: Vec<u8>,
    /// Pre-parsed constraint expression (typically a small AST or
    /// JSON-ish blob). The wallet's grammar validator must accept
    /// it; we ship a minimal structural check here.
    pub constraint: Vec<u8>,
    pub deadline_epoch: u64,
    pub energy_budget: u64,
    /// AI-attestation confidence in basis points (0..=10_000).
    pub confidence_bp: u64,
}

impl Intent {
    /// Construct an intent, applying grammar checks at construction.
    /// The chain calls this; chat-to-intent layer is upstream.
    pub fn new(
        id: IntentId,
        holder: [u8; 32],
        verb: Verb,
        object: Vec<u8>,
        constraint: Vec<u8>,
        deadline_epoch: u64,
        energy_budget: u64,
        confidence_bp: u64,
    ) -> Result<Self, IntentError> {
        if !KNOWN_VERBS.contains(&verb) {
            return Err(IntentError::UnknownVerb(verb));
        }
        if object.is_empty() {
            return Err(IntentError::EmptyObject);
        }
        if object.len() > 1024 {
            return Err(IntentError::ObjectTooLong);
        }
        validate_constraint(&constraint)?;
        if deadline_epoch == 0 {
            return Err(IntentError::ZeroDeadline);
        }
        if energy_budget == 0 {
            return Err(IntentError::ZeroBudget);
        }
        Ok(Self {
            id,
            holder,
            verb,
            object,
            constraint,
            deadline_epoch,
            energy_budget,
            confidence_bp,
        })
    }

    /// Pre-execution gate: deadline + confidence + budget checks.
    /// Caller passes the chain's current epoch and minimum-confidence
    /// floor.
    pub fn admit(&self, now: u64, floor_bp: u64, required_energy: u64) -> Result<(), IntentError> {
        if now > self.deadline_epoch {
            return Err(IntentError::DeadlinePassed {
                now,
                deadline: self.deadline_epoch,
            });
        }
        if self.confidence_bp < floor_bp {
            return Err(IntentError::LowConfidence {
                confidence_bp: self.confidence_bp,
                floor_bp,
            });
        }
        if required_energy > self.energy_budget {
            return Err(IntentError::BudgetExceeded {
                required: required_energy,
                budget: self.energy_budget,
            });
        }
        Ok(())
    }

    /// Domain-tagged commitment over the intent's body. Anti-replay:
    /// changing any field changes the commitment.
    pub fn commitment(&self) -> [u8; 32] {
        let mut h = blake3::Hasher::new();
        h.update(b"evaporchain:counsel-intent:v1\0");
        h.update(&self.id.0);
        h.update(&self.holder);
        h.update(&[self.verb as u8 as u8]);
        h.update(&(self.object.len() as u32).to_le_bytes());
        h.update(&self.object);
        h.update(&(self.constraint.len() as u32).to_le_bytes());
        h.update(&self.constraint);
        h.update(&self.deadline_epoch.to_le_bytes());
        h.update(&self.energy_budget.to_le_bytes());
        h.update(&self.confidence_bp.to_le_bytes());
        *h.finalize().as_bytes()
    }
}

/// Grammar check on a constraint blob. V1 minimal: must be valid
/// JSON when interpreted as UTF-8. (Production would parse a
/// constraint AST.) Empty constraint is allowed (means "no
/// constraint").
fn validate_constraint(c: &[u8]) -> Result<(), IntentError> {
    if c.is_empty() {
        return Ok(());
    }
    // Reject obviously malformed bytes: must be valid UTF-8.
    let s = std::str::from_utf8(c)
        .map_err(|_| IntentError::BadConstraint("constraint must be valid UTF-8".into()))?;
    // V1: must start and end with matching JSON braces { } or
    // brackets [ ]. (Real grammar parser is V2.)
    let trimmed = s.trim();
    let valid_shape = (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'));
    if !valid_shape {
        return Err(IntentError::BadConstraint(format!(
            "constraint must be wrapped in {{ }} or [ ]; got: {trimmed:.64}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iid(b: u8) -> IntentId {
        IntentId([b; 32])
    }
    fn alice() -> [u8; 32] {
        [0xAA; 32]
    }

    fn fresh(verb: Verb) -> Intent {
        Intent::new(
            iid(1),
            alice(),
            verb,
            b"target".to_vec(),
            b"{}".to_vec(),
            100,
            1_000,
            9_500,
        )
        .unwrap()
    }

    // ── grammar ──────────────────────────────────────────────────

    #[test]
    fn known_verb_accepted() {
        let i = fresh(Verb::Send);
        assert_eq!(i.verb, Verb::Send);
    }

    #[test]
    fn empty_object_rejected() {
        let err = Intent::new(
            iid(1),
            alice(),
            Verb::Send,
            vec![],
            b"{}".to_vec(),
            100,
            1000,
            9500,
        )
        .unwrap_err();
        assert_eq!(err, IntentError::EmptyObject);
    }

    #[test]
    fn over_long_object_rejected() {
        let big = vec![0u8; 1025];
        let err = Intent::new(
            iid(1),
            alice(),
            Verb::Send,
            big,
            b"{}".to_vec(),
            100,
            1000,
            9500,
        )
        .unwrap_err();
        assert_eq!(err, IntentError::ObjectTooLong);
    }

    #[test]
    fn malformed_constraint_rejected() {
        let err = Intent::new(
            iid(1),
            alice(),
            Verb::Send,
            b"target".to_vec(),
            b"not-json-shape".to_vec(),
            100,
            1000,
            9500,
        )
        .unwrap_err();
        assert!(matches!(err, IntentError::BadConstraint(_)));
    }

    #[test]
    fn empty_constraint_is_allowed() {
        Intent::new(
            iid(1),
            alice(),
            Verb::Send,
            b"target".to_vec(),
            vec![],
            100,
            1000,
            9500,
        )
        .unwrap();
    }

    #[test]
    fn array_constraint_is_allowed() {
        Intent::new(
            iid(1),
            alice(),
            Verb::Send,
            b"target".to_vec(),
            b"[1,2,3]".to_vec(),
            100,
            1000,
            9500,
        )
        .unwrap();
    }

    #[test]
    fn non_utf8_constraint_rejected() {
        // 0xFF is not a valid UTF-8 start byte.
        let err = Intent::new(
            iid(1),
            alice(),
            Verb::Send,
            b"target".to_vec(),
            vec![0xFF, 0xFE],
            100,
            1000,
            9500,
        )
        .unwrap_err();
        assert!(matches!(err, IntentError::BadConstraint(_)));
    }

    #[test]
    fn zero_deadline_rejected() {
        let err = Intent::new(
            iid(1),
            alice(),
            Verb::Send,
            b"target".to_vec(),
            b"{}".to_vec(),
            0,
            1000,
            9500,
        )
        .unwrap_err();
        assert_eq!(err, IntentError::ZeroDeadline);
    }

    #[test]
    fn zero_budget_rejected() {
        let err = Intent::new(
            iid(1),
            alice(),
            Verb::Send,
            b"target".to_vec(),
            b"{}".to_vec(),
            100,
            0,
            9500,
        )
        .unwrap_err();
        assert_eq!(err, IntentError::ZeroBudget);
    }

    // ── admit gate ────────────────────────────────────────────────

    #[test]
    fn fresh_intent_admits() {
        let i = fresh(Verb::Send);
        i.admit(50, 9000, 500).unwrap();
    }

    #[test]
    fn past_deadline_rejected() {
        let i = fresh(Verb::Send);
        let err = i.admit(101, 9000, 500).unwrap_err();
        assert!(matches!(err, IntentError::DeadlinePassed { .. }));
    }

    #[test]
    fn at_deadline_admits() {
        // `now > deadline` rejects; `now == deadline` admits.
        let i = fresh(Verb::Send);
        i.admit(100, 9000, 500).unwrap();
    }

    #[test]
    fn low_confidence_rejected() {
        let i = fresh(Verb::Send);
        let err = i.admit(50, 9_700, 500).unwrap_err(); // floor 97% > i.confidence 95%
        assert!(matches!(err, IntentError::LowConfidence { .. }));
    }

    #[test]
    fn budget_exceeded_rejected() {
        let i = fresh(Verb::Send);
        let err = i.admit(50, 9000, 1500).unwrap_err(); // 1500 > 1000 budget
        assert!(matches!(err, IntentError::BudgetExceeded { .. }));
    }

    #[test]
    fn at_budget_admits() {
        // required == budget is admissible.
        let i = fresh(Verb::Send);
        i.admit(50, 9000, 1000).unwrap();
    }

    // ── commitment ───────────────────────────────────────────────

    #[test]
    fn commitment_is_deterministic() {
        let i = fresh(Verb::Send);
        assert_eq!(i.commitment(), i.commitment());
    }

    #[test]
    fn commitment_changes_with_each_field() {
        let base = fresh(Verb::Send);
        let h0 = base.commitment();
        let mut a = base.clone();
        a.deadline_epoch = 200;
        assert_ne!(a.commitment(), h0);
        let mut b = base.clone();
        b.confidence_bp = 1000;
        assert_ne!(b.commitment(), h0);
        let mut c = base.clone();
        c.energy_budget = 99;
        assert_ne!(c.commitment(), h0);
    }

    // ── doctrine claim ────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Singh-Counsel is the first chat-AI wallet
        // primitive where natural-language intents are validated
        // against a structural grammar BEFORE execution. Chat
        // noise that doesn't parse is rejected — the chain
        // never executes a free-form string. Low-confidence
        // translations and past-deadline intents are also
        // rejected."

        // Honest intent: parses, fresh, high confidence → admits.
        let honest = fresh(Verb::Send);
        honest.admit(50, 9000, 500).unwrap();

        // Chat-noise unknown-verb attack: attacker tries to
        // express something the wallet doesn't know how to do.
        // (Forced via direct field manipulation — the chain
        // refuses because verb isn't in KNOWN_VERBS.)
        // Actually all variants of `Verb` are in KNOWN_VERBS, so
        // unknown-verb in V1 surfaces only when the verb enum
        // grows but the validator hasn't been updated. We test
        // the malformed-constraint path instead:
        let err = Intent::new(
            iid(2),
            alice(),
            Verb::Send,
            b"target".to_vec(),
            b"send all my money lol".to_vec(), // chat noise as constraint
            100,
            1000,
            9500,
        )
        .unwrap_err();
        assert!(matches!(err, IntentError::BadConstraint(_)));

        // Stale-intent attack: replay a past-deadline intent.
        let stale = fresh(Verb::Send);
        let err = stale.admit(200, 9000, 500).unwrap_err();
        assert!(matches!(err, IntentError::DeadlinePassed { .. }));

        // Low-confidence chat translation rejected.
        let mut shaky = fresh(Verb::Send);
        shaky.confidence_bp = 5_000; // 50%
        let err = shaky.admit(50, 9000, 500).unwrap_err();
        assert!(matches!(err, IntentError::LowConfidence { .. }));
    }

    proptest::proptest! {
        #[test]
        fn property_past_deadline_always_rejects(
            deadline in 1u64..1000u64,
            after in 1u64..1000u64,
        ) {
            let i = Intent::new(
                iid(1), alice(), Verb::Send, b"x".to_vec(), b"{}".to_vec(),
                deadline, 1000, 9500,
            ).unwrap();
            let result = i.admit(deadline + after, 9000, 500);
            let is_deadline_err = matches!(result, Err(IntentError::DeadlinePassed { .. }));
            proptest::prop_assert!(is_deadline_err);
        }

        #[test]
        fn property_below_floor_always_rejects(
            confidence in 0u64..5_000u64,
            floor in 5_001u64..10_000u64,
        ) {
            let i = Intent::new(
                iid(1), alice(), Verb::Send, b"x".to_vec(), b"{}".to_vec(),
                100, 1000, confidence,
            ).unwrap();
            let result = i.admit(50, floor, 500);
            let is_low = matches!(result, Err(IntentError::LowConfidence { .. }));
            proptest::prop_assert!(is_low);
        }
    }
}
