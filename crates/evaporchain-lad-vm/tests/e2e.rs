//! End-to-end integration tests for evaporchain-lad-vm.
//!
//! Non-trivial fixture: a multi-resource ticket vending machine.
//!
//! Three resources are issued by the coordinator (epoch 0):
//!
//!   Resource 0 — Linear ticket (one-time-use concert pass)
//!   Resource 1 — Affine coupon  (optional discount, can be abandoned)
//!   Resource 2 — Decaying pass  (day-pass, window=100 epochs)
//!
//! Scenario:
//!   Epoch  5  — Linear ticket redeemed at the gate.
//!   Epoch 10  — Affine coupon explicitly dropped (holder opts out).
//!   Epoch 90  — Decaying pass is still valid; holder checks in.
//!   Epoch 101 — Decaying pass has expired; no transaction needed.
//!   Epoch 200 — Re-run tick_decay to confirm evaporation is idempotent.
//!
//! Adversarial checks woven in:
//!   - Second redeem of the Linear ticket → AlreadyConsumed.
//!   - Drop on the Linear ticket → LinearCannotDrop.
//!   - Use of the Decaying pass after window → Evaporated.
//!
//! Doctrine claim (INVENTION_STACK §4.1 row 12):
//!   "Move resources × decay. 'Use it or evaporate.' Forces liveness
//!   as a type-system property."

use evaporchain_lad_vm::{drop_resource, tick_decay, use_resource, Mode, OpError, Resource};

// ── Helpers ───────────────────────────────────────────────────────────────

fn linear_ticket(created_at: u64) -> Resource<&'static str> {
    Resource::linear("CONCERT-2026-ROW-A-SEAT-7", created_at)
}

fn affine_coupon(created_at: u64) -> Resource<&'static str> {
    Resource::affine("DISCOUNT-15-PCT-OFF-MERCH", created_at)
}

fn decaying_pass(created_at: u64, window: u64) -> Resource<&'static str> {
    Resource::decaying("DAY-PASS-VALID-100-EPOCHS", created_at, window)
}

// ── Happy-path lifecycle ──────────────────────────────────────────────────

#[test]
fn full_lifecycle_three_resource_types() {
    // ── Issue epoch 0 ────────────────────────────────────────────────────

    let ticket = linear_ticket(0);
    let coupon = affine_coupon(0);
    let mut pass = decaying_pass(0, 100);

    assert_eq!(ticket.mode, Mode::Linear);
    assert_eq!(coupon.mode, Mode::Affine);
    assert_eq!(pass.mode, Mode::Decaying);
    assert!(!ticket.consumed);
    assert!(!coupon.consumed);
    assert!(!pass.consumed);

    // ── Epoch 5: redeem the Linear ticket at the gate ─────────────────────

    let (claim, receipt) = use_resource(ticket, 5).unwrap();
    assert_eq!(claim, "CONCERT-2026-ROW-A-SEAT-7");
    assert!(receipt.consumed);

    // ── Epoch 10: holder abandons the Affine coupon ───────────────────────

    drop_resource(coupon).unwrap();

    // ── Epoch 90: Decaying pass is still valid ────────────────────────────

    // tick_decay within window → no change.
    pass = tick_decay(pass, 90);
    assert!(
        !pass.consumed,
        "pass must still be live at epoch 90 (window=100)"
    );

    // ── Epoch 100: pass has naturally expired — no TX sent ────────────────
    // is_evaporated uses >=, so created_at(0) + window(100) = 100 is first expired.

    pass = tick_decay(pass, 100);
    assert!(
        pass.consumed,
        "pass must be auto-consumed at epoch 100 (created_at+window)"
    );

    // ── Epoch 200: idempotent — tick again changes nothing ────────────────

    pass = tick_decay(pass, 200);
    assert!(
        pass.consumed,
        "consumed state is idempotent across further ticks"
    );
}

// ── Adversarial: Linear exactly-once semantics ───────────────────────────

#[test]
fn linear_double_redeem_rejected() {
    let mut ticket = linear_ticket(0);
    ticket.consumed = true; // simulate prior redeem
    let err = use_resource(ticket, 5).unwrap_err();
    assert_eq!(err, OpError::AlreadyConsumed);
}

#[test]
fn linear_drop_rejected_substructural_invariant() {
    let ticket = linear_ticket(0);
    let err = drop_resource(ticket).unwrap_err();
    assert_eq!(
        err,
        OpError::LinearCannotDrop,
        "Linear resources must be consumed — the runtime enforces liveness"
    );
}

// ── Adversarial: Decaying expiry ─────────────────────────────────────────

#[test]
fn decaying_use_after_window_evaporated() {
    let pass = decaying_pass(0, 100);
    let err = use_resource(pass, 200).unwrap_err();
    assert_eq!(
        err,
        OpError::Evaporated,
        "use_resource must reject a resource whose epoch window has closed"
    );
}

#[test]
fn decaying_use_last_valid_epoch_accepted() {
    // is_evaporated uses >= so created_at + window is the FIRST expired epoch.
    // For created_at=0, window=100: epoch 99 is the last valid epoch.
    let pass = decaying_pass(0, 100);
    let (claim, _) = use_resource(pass, 99).unwrap();
    assert_eq!(claim, "DAY-PASS-VALID-100-EPOCHS");
}

#[test]
fn decaying_use_at_created_plus_window_evaporated() {
    // epoch == created_at + window (100) is the first expired epoch.
    let pass = decaying_pass(0, 100);
    let err = use_resource(pass, 100).unwrap_err();
    assert_eq!(err, OpError::Evaporated);
}

// ── Affine semantics ─────────────────────────────────────────────────────

#[test]
fn affine_drop_then_redeem_rejected() {
    let mut coupon = affine_coupon(0);
    // Simulate prior drop by marking consumed.
    coupon.consumed = true;
    let err = use_resource(coupon, 5).unwrap_err();
    assert_eq!(err, OpError::AlreadyConsumed);
}

#[test]
fn affine_redeem_sets_consumed() {
    let coupon = affine_coupon(0);
    let (claim, receipt) = use_resource(coupon, 1).unwrap();
    assert_eq!(claim, "DISCOUNT-15-PCT-OFF-MERCH");
    assert!(receipt.consumed);
}

// ── Tick semantics: Linear never evaporates ───────────────────────────────

#[test]
fn linear_tick_at_max_epoch_does_not_evaporate() {
    // Linear resources have no decay window — they survive any epoch tick.
    let ticket = linear_ticket(0);
    let after = tick_decay(ticket, u64::MAX);
    assert!(
        !after.consumed,
        "Linear resources never auto-expire — they require explicit consumption"
    );
}

// ── Invariant: resource mode is immutable ────────────────────────────────

#[test]
fn mode_is_immutable_after_issue() {
    let r = Resource::decaying("value", 0u64, 50);
    let ticked = tick_decay(r, 10);
    assert_eq!(ticked.mode, Mode::Decaying);
}
