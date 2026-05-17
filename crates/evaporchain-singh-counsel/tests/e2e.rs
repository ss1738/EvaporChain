//! End-to-end integration tests for evaporchain-singh-counsel.
//!
//! Non-trivial fixture: Alice's wallet session — 5 intents derived from
//! chat interactions, evaluated at epoch_now=50, confidence_floor=9_000bp.
//!
//!   I1 (Send)    — valid grammar, fresh, high confidence, budget OK → ADMIT
//!   I2 (Stake)   — valid grammar, PAST deadline (deadline=30, now=50) → REJECT
//!   I3 (Vote)    — valid grammar, LOW confidence (6_000bp < 9_000bp floor) → REJECT
//!   I4 (Deploy)  — valid grammar, OVER budget (required=600, budget=500) → REJECT
//!   I5 (Refresh) — chat noise in constraint ("send all my money lol") → REJECT at construction
//!
//! Doctrine claim (INVENTION_STACK.md §A5.x Singh-Counsel):
//! "Grammar-validated intents: chat noise that doesn't parse is rejected at
//! construction, never executed-blindly. Past-deadline and low-confidence
//! intents fail at the admission gate. The chain never executes a free-form
//! string. Anti-replay commitment changes with any field change."
//!
//! INVENTION_STACK §A5: Singh-Counsel (chat-first AI wallet primitive).

use evaporchain_singh_counsel::{Intent, IntentError, IntentId, Verb, KNOWN_VERBS};

// ── Helpers ───────────────────────────────────────────────────────────────

const EPOCH_NOW: u64       = 50;
const CONF_FLOOR: u64      = 9_000; // 90% floor

fn iid(b: u8) -> IntentId {
    IntentId([b; 32])
}

fn holder() -> [u8; 32] {
    [0xAA; 32]
}

fn make_intent(
    seq: u8,
    verb: Verb,
    deadline: u64,
    budget: u64,
    confidence_bp: u64,
) -> Result<Intent, IntentError> {
    Intent::new(
        iid(seq),
        holder(),
        verb,
        vec![0xCC; 32], // 32-byte object (e.g., recipient address)
        b"{}".to_vec(),
        deadline,
        budget,
        confidence_bp,
    )
}

// ── Non-trivial fixture ───────────────────────────────────────────────────

#[test]
fn i1_valid_send_admits() {
    // I1: Send, deadline=100 (fresh), confidence=9_500, budget=500, required=400 → ADMIT.
    let i1 = make_intent(1, Verb::Send, 100, 500, 9_500).unwrap();
    i1.admit(EPOCH_NOW, CONF_FLOOR, 400).unwrap();
}

#[test]
fn i2_past_deadline_rejected_at_admission() {
    // I2: Stake, deadline=30 — past (now=50) → REJECT.
    let i2 = make_intent(2, Verb::Stake, 30, 500, 9_500).unwrap();
    let err = i2.admit(EPOCH_NOW, CONF_FLOOR, 400).unwrap_err();
    assert!(
        matches!(err, IntentError::DeadlinePassed { now: 50, deadline: 30 }),
        "expected DeadlinePassed{{now=50, deadline=30}}, got {err:?}"
    );
}

#[test]
fn i3_low_confidence_rejected_at_admission() {
    // I3: Vote, confidence=6_000bp < 9_000bp floor → REJECT.
    let i3 = make_intent(3, Verb::Vote, 100, 500, 6_000).unwrap();
    let err = i3.admit(EPOCH_NOW, CONF_FLOOR, 400).unwrap_err();
    assert!(
        matches!(err, IntentError::LowConfidence { confidence_bp: 6_000, floor_bp: 9_000 }),
        "expected LowConfidence, got {err:?}"
    );
}

#[test]
fn i4_over_budget_rejected_at_admission() {
    // I4: Deploy, budget=500, required=600 → BudgetExceeded.
    let i4 = make_intent(4, Verb::Deploy, 100, 500, 9_500).unwrap();
    let err = i4.admit(EPOCH_NOW, CONF_FLOOR, 600).unwrap_err();
    assert!(
        matches!(err, IntentError::BudgetExceeded { required: 600, budget: 500 }),
        "expected BudgetExceeded, got {err:?}"
    );
}

#[test]
fn i5_chat_noise_in_constraint_rejected_at_construction() {
    // I5: Refresh — chat noise as constraint fails BEFORE reaching admission.
    // Doctrine: "chat noise that doesn't parse is rejected at construction".
    let err = Intent::new(
        iid(5),
        holder(),
        Verb::Refresh,
        vec![0xCC; 32],
        b"send all my money lol".to_vec(), // not JSON-shaped
        100,
        500,
        9_500,
    )
    .unwrap_err();
    assert!(
        matches!(err, IntentError::BadConstraint(_)),
        "chat-noise constraint must fail at construction, got {err:?}"
    );
}

#[test]
fn session_admit_matrix_all_five_intents() {
    // Full session: exactly 1 admits, 4 rejects (each for a different reason).
    let i1 = make_intent(1, Verb::Send, 100, 500, 9_500).unwrap();
    let i2 = make_intent(2, Verb::Stake, 30, 500, 9_500).unwrap();
    let i3 = make_intent(3, Verb::Vote, 100, 500, 6_000).unwrap();
    let i4 = make_intent(4, Verb::Deploy, 100, 500, 9_500).unwrap();

    assert!(i1.admit(EPOCH_NOW, CONF_FLOOR, 400).is_ok(),    "I1 must ADMIT");
    assert!(i2.admit(EPOCH_NOW, CONF_FLOOR, 400).is_err(),   "I2 must REJECT (deadline)");
    assert!(i3.admit(EPOCH_NOW, CONF_FLOOR, 400).is_err(),   "I3 must REJECT (confidence)");
    assert!(i4.admit(EPOCH_NOW, CONF_FLOOR, 600).is_err(),   "I4 must REJECT (budget)");

    // I5 never makes it past construction — counted as rejected by grammar.
    let i5 = Intent::new(
        iid(5), holder(), Verb::Refresh,
        vec![0xCC; 32], b"not json".to_vec(), 100, 500, 9_500,
    );
    assert!(i5.is_err(), "I5 must REJECT at construction (chat noise)");
}

// ── Doctrine tests ────────────────────────────────────────────────────────

#[test]
fn doctrine_all_known_verbs_are_constructible() {
    // Every verb in KNOWN_VERBS must pass construction.
    for &verb in KNOWN_VERBS {
        let r = make_intent(0x10, verb, 100, 500, 9_500);
        assert!(r.is_ok(), "KNOWN_VERBS entry {verb:?} must be constructible");
    }
}

#[test]
fn doctrine_grammar_gate_fires_before_admission_gate() {
    // Prove ordering: a BadConstraint intent never reaches admit().
    // Construct fails → no admit() call needed.
    let bad = Intent::new(
        iid(0xFF), holder(), Verb::Send,
        vec![0x01], b"not-a-json-shape".to_vec(), 100, 500, 9_500,
    );
    assert!(matches!(bad, Err(IntentError::BadConstraint(_))));
}

#[test]
fn doctrine_anti_replay_commitment_changes_with_every_field() {
    let base = make_intent(0x01, Verb::Send, 100, 500, 9_500).unwrap();
    let h0 = base.commitment();

    let mut a = base.clone();
    a.verb = Verb::Stake;
    assert_ne!(a.commitment(), h0, "verb change must alter commitment");

    let mut b = base.clone();
    b.deadline_epoch = 200;
    assert_ne!(b.commitment(), h0, "deadline change must alter commitment");

    let mut c = base.clone();
    c.energy_budget = 1_000;
    assert_ne!(c.commitment(), h0, "budget change must alter commitment");

    let mut d = base.clone();
    d.confidence_bp = 5_000;
    assert_ne!(d.commitment(), h0, "confidence change must alter commitment");

    let mut e = base.clone();
    e.object = vec![0xDE; 32];
    assert_ne!(e.commitment(), h0, "object change must alter commitment");
}

#[test]
fn doctrine_deadline_boundary_now_eq_deadline_admits() {
    // Gate: `now > deadline` rejects; `now == deadline` admits.
    let i = make_intent(0x02, Verb::Reanchor, EPOCH_NOW, 500, 9_500).unwrap();
    i.admit(EPOCH_NOW, CONF_FLOOR, 400).unwrap();
    // One epoch later: rejected.
    assert!(matches!(
        i.admit(EPOCH_NOW + 1, CONF_FLOOR, 400),
        Err(IntentError::DeadlinePassed { .. })
    ));
}

#[test]
fn doctrine_confidence_boundary_at_floor_admits() {
    // confidence_bp == floor_bp admits; one below rejects.
    let i = make_intent(0x03, Verb::Send, 100, 500, CONF_FLOOR).unwrap();
    i.admit(EPOCH_NOW, CONF_FLOOR, 400).unwrap();

    let low = make_intent(0x04, Verb::Send, 100, 500, CONF_FLOOR - 1).unwrap();
    assert!(matches!(
        low.admit(EPOCH_NOW, CONF_FLOOR, 400),
        Err(IntentError::LowConfidence { .. })
    ));
}

#[test]
fn doctrine_budget_boundary_at_budget_admits() {
    // required == budget admits; one over rejects.
    let i = make_intent(0x05, Verb::Send, 100, 500, 9_500).unwrap();
    i.admit(EPOCH_NOW, CONF_FLOOR, 500).unwrap();

    assert!(matches!(
        i.admit(EPOCH_NOW, CONF_FLOOR, 501),
        Err(IntentError::BudgetExceeded { .. })
    ));
}

#[test]
fn doctrine_commitment_is_deterministic() {
    let i = make_intent(0x06, Verb::Send, 100, 500, 9_500).unwrap();
    assert_eq!(i.commitment(), i.commitment());
}

// ── Adversarial tests ─────────────────────────────────────────────────────

#[test]
fn adversarial_empty_object_rejected() {
    let err = Intent::new(iid(1), holder(), Verb::Send, vec![], b"{}".to_vec(), 100, 500, 9_500)
        .unwrap_err();
    assert_eq!(err, IntentError::EmptyObject);
}

#[test]
fn adversarial_object_exceeding_1024_bytes_rejected() {
    let err = Intent::new(
        iid(1), holder(), Verb::Send, vec![0u8; 1025], b"{}".to_vec(), 100, 500, 9_500,
    )
    .unwrap_err();
    assert_eq!(err, IntentError::ObjectTooLong);
}

#[test]
fn adversarial_object_at_1024_bytes_accepted() {
    // 1024 bytes is exactly at the limit — allowed.
    Intent::new(
        iid(1), holder(), Verb::Send, vec![0u8; 1024], b"{}".to_vec(), 100, 500, 9_500,
    )
    .unwrap();
}

#[test]
fn adversarial_zero_deadline_rejected() {
    let err = Intent::new(iid(1), holder(), Verb::Send, vec![1], b"{}".to_vec(), 0, 500, 9_500)
        .unwrap_err();
    assert_eq!(err, IntentError::ZeroDeadline);
}

#[test]
fn adversarial_zero_budget_rejected() {
    let err = Intent::new(iid(1), holder(), Verb::Send, vec![1], b"{}".to_vec(), 100, 0, 9_500)
        .unwrap_err();
    assert_eq!(err, IntentError::ZeroBudget);
}

#[test]
fn adversarial_non_utf8_constraint_rejected() {
    let err = Intent::new(
        iid(1), holder(), Verb::Send, vec![1], vec![0xFF, 0xFE], 100, 500, 9_500,
    )
    .unwrap_err();
    assert!(matches!(err, IntentError::BadConstraint(_)));
}

#[test]
fn adversarial_array_constraint_accepted() {
    // Array-shaped JSON is valid: [ ... ]
    Intent::new(
        iid(1), holder(), Verb::Send, vec![1], b"[1,2,3]".to_vec(), 100, 500, 9_500,
    )
    .unwrap();
}

#[test]
fn adversarial_empty_constraint_accepted() {
    // No constraint is allowed (means "no restriction").
    Intent::new(
        iid(1), holder(), Verb::Send, vec![1], vec![], 100, 500, 9_500,
    )
    .unwrap();
}
