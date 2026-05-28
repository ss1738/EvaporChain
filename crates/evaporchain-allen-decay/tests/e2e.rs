//! End-to-end integration tests for evaporchain-allen-decay.
//!
//! Non-trivial fixture: a DeFi protocol lifecycle audit using energy
//! intervals. In EvaporChain, every phase of a contract's existence is
//! bounded by the chain's energy level — not wall-clock time. An external
//! auditor uses `compute_relation` to verify that each phase of the
//! protocol occurs in the intended causal order.
//!
//! Doctrine claim (INVENTION_STACK §4.2 Allen-Decay Opcodes):
//!   "13 interval-relation opcodes in EvaporScript (Allen 1983);
//!   intervals bounded by energy levels. The pair-flip inversion identity
//!   holds: `compute_relation(a,b).inverse() == compute_relation(b,a)`
//!   for every pair. Intervals with `start ≥ end` are rejected."
//!
//! Protocol lifecycle intervals (energy units = cumulative chain energy spent):
//!
//!   GENESIS      [0,           1_000)   — chain boot, before all other phases
//!   WARMUP       [1_000,      50_000)   — meets genesis exactly at 1_000
//!   ACTIVE       [40_000,    900_000)   — overlaps warmup (starts inside warmup's tail)
//!   FLASH_LOAN   [50_000,    200_000)   — during active lending window
//!   GOVERNANCE   [400_000,   700_000)   — voting window (during active)
//!   GOV_WIDER    [400_000,   800_000)   — started-by governance (same start, later end)
//!   GOV_LATER    [500_000,   700_000)   — finishes governance (later start, same end)
//!   GOV_COPY     [400_000,   700_000)   — equals governance exactly
//!   GOV_SUB      [450_000,   650_000)   — during governance
//!   AUDIT        [800_000,   950_000)   — overlapped-by active (active ends at 900k inside audit)
//!   SHUTDOWN     [900_000, 1_100_000)   — met-by active (active ends exactly at shutdown start)
//!   TAIL         [1_100_001, 1_200_000) — after shutdown (1-unit gap guarantees strict After)
//!
//! This fixture exercises all 13 Allen relations across a single coherent scenario.

use evaporchain_allen_decay::{compute_relation, AllenRelation, Interval, IntervalError};

// ── Fixture helpers ───────────────────────────────────────────────────────────

fn iv(s: u64, e: u64) -> Interval {
    Interval::new(s, e).unwrap()
}

fn genesis() -> Interval {
    iv(0, 1_000)
}
fn warmup() -> Interval {
    iv(1_000, 50_000)
}
fn active() -> Interval {
    iv(40_000, 900_000)
}
fn flash_loan() -> Interval {
    iv(50_000, 200_000)
}
fn governance() -> Interval {
    iv(400_000, 700_000)
}
fn gov_wider() -> Interval {
    iv(400_000, 800_000)
}
fn gov_later() -> Interval {
    iv(500_000, 700_000)
}
fn gov_copy() -> Interval {
    iv(400_000, 700_000)
}
fn gov_sub() -> Interval {
    iv(450_000, 650_000)
}
fn audit() -> Interval {
    iv(800_000, 950_000)
}
fn shutdown() -> Interval {
    iv(900_000, 1_100_000)
}
fn tail() -> Interval {
    iv(1_100_001, 1_200_000)
}

// ── All 13 Allen relations from one coherent fixture ─────────────────────────

#[test]
fn all_13_allen_relations_exercised_in_defi_lifecycle() {
    // 1. Before — genesis fully precedes flash_loan (gap: [1_000, 50_000)).
    assert_eq!(
        compute_relation(genesis(), flash_loan()),
        AllenRelation::Before,
        "genesis [0,1k) must be Before flash_loan [50k,200k)"
    );

    // 2. Meets — genesis ends exactly where warmup begins.
    assert_eq!(
        compute_relation(genesis(), warmup()),
        AllenRelation::Meets,
        "genesis [0,1k) must Meet warmup [1k,50k)"
    );

    // 3. Overlaps — warmup starts, active starts inside warmup, warmup ends inside active.
    assert_eq!(
        compute_relation(warmup(), active()),
        AllenRelation::Overlaps,
        "warmup [1k,50k) must Overlap active [40k,900k)"
    );

    // 4. Starts — governance and gov_wider share the same start; governance ends first.
    assert_eq!(
        compute_relation(governance(), gov_wider()),
        AllenRelation::Starts,
        "governance [400k,700k) must Start gov_wider [400k,800k)"
    );

    // 5. During — flash_loan is strictly inside active (both endpoints).
    assert_eq!(
        compute_relation(flash_loan(), active()),
        AllenRelation::During,
        "flash_loan [50k,200k) must be During active [40k,900k)"
    );

    // 6. Finishes — gov_later starts after governance but ends at the same point.
    assert_eq!(
        compute_relation(gov_later(), governance()),
        AllenRelation::Finishes,
        "gov_later [500k,700k) must Finish governance [400k,700k)"
    );

    // 7. Equals — governance and gov_copy are identical intervals.
    assert_eq!(
        compute_relation(governance(), gov_copy()),
        AllenRelation::Equals,
        "governance must Equal gov_copy (identical intervals)"
    );

    // 8. FinishedBy — governance contains gov_later and they share the same end.
    assert_eq!(
        compute_relation(governance(), gov_later()),
        AllenRelation::FinishedBy,
        "governance [400k,700k) must be FinishedBy gov_later [500k,700k)"
    );

    // 9. Contains — active fully contains governance (strict containment on both sides).
    assert_eq!(
        compute_relation(active(), governance()),
        AllenRelation::Contains,
        "active [40k,900k) must Contain governance [400k,700k)"
    );

    // 10. StartedBy — gov_wider starts at same point as governance; gov_wider ends later.
    assert_eq!(
        compute_relation(gov_wider(), governance()),
        AllenRelation::StartedBy,
        "gov_wider [400k,800k) must be StartedBy governance [400k,700k)"
    );

    // 11. OverlappedBy — audit starts inside active (at 800k) and extends past active's end (900k).
    assert_eq!(
        compute_relation(audit(), active()),
        AllenRelation::OverlappedBy,
        "audit [800k,950k) must be OverlappedBy active [40k,900k)"
    );

    // 12. MetBy — shutdown starts exactly where active ends (both at 900_000).
    assert_eq!(
        compute_relation(shutdown(), active()),
        AllenRelation::MetBy,
        "shutdown [900k,1100k) must be MetBy active (active ends exactly at shutdown start)"
    );

    // 13. After — tail starts one unit past shutdown's end.
    assert_eq!(
        compute_relation(tail(), shutdown()),
        AllenRelation::After,
        "tail [1_100_001,1_200_000) must be After shutdown [900k,1100k)"
    );
}

// ── Pair-flip inversion identity ─────────────────────────────────────────────

#[test]
fn pair_flip_inversion_holds_for_every_lifecycle_pair() {
    // For any (A, B): compute_relation(B, A) == compute_relation(A, B).inverse()
    let intervals = [
        genesis(),
        warmup(),
        active(),
        flash_loan(),
        governance(),
        gov_wider(),
        gov_later(),
        audit(),
        shutdown(),
        tail(),
    ];
    for &a in &intervals {
        for &b in &intervals {
            let r_ab = compute_relation(a, b);
            let r_ba = compute_relation(b, a);
            assert_eq!(
                r_ab.inverse(),
                r_ba,
                "inverse identity failed for a={a:?}, b={b:?}: R(a,b)={r_ab:?}, R(b,a)={r_ba:?}"
            );
        }
    }
}

#[test]
fn double_inverse_is_identity_for_all_13_relations() {
    let all_relations = [
        AllenRelation::Before,
        AllenRelation::Meets,
        AllenRelation::Overlaps,
        AllenRelation::Starts,
        AllenRelation::During,
        AllenRelation::Finishes,
        AllenRelation::Equals,
        AllenRelation::FinishedBy,
        AllenRelation::Contains,
        AllenRelation::StartedBy,
        AllenRelation::OverlappedBy,
        AllenRelation::MetBy,
        AllenRelation::After,
    ];
    for r in all_relations {
        assert_eq!(
            r.inverse().inverse(),
            r,
            "double-inverse must be identity for {r:?}"
        );
    }
}

// ── Symmetric pairs (r and its inverse are distinct unless Equals) ────────────

#[test]
fn symmetric_inverse_pairs_are_consistent() {
    // 6 symmetric pairs + Equals (self-inverse).
    let pairs = [
        (AllenRelation::Before, AllenRelation::After),
        (AllenRelation::Meets, AllenRelation::MetBy),
        (AllenRelation::Overlaps, AllenRelation::OverlappedBy),
        (AllenRelation::Starts, AllenRelation::StartedBy),
        (AllenRelation::During, AllenRelation::Contains),
        (AllenRelation::Finishes, AllenRelation::FinishedBy),
    ];
    for (r, inv) in pairs {
        assert_eq!(r.inverse(), inv, "{r:?}.inverse() must be {inv:?}");
        assert_eq!(inv.inverse(), r, "{inv:?}.inverse() must be {r:?}");
        assert_ne!(r, inv, "{r:?} must not equal its own inverse");
    }
    assert_eq!(
        AllenRelation::Equals.inverse(),
        AllenRelation::Equals,
        "Equals must be its own inverse (the only self-inverse relation)"
    );
}

// ── Interval construction and duration ───────────────────────────────────────

#[test]
fn interval_construction_and_duration_correct() {
    let i = iv(100, 400);
    assert_eq!(i.start, 100);
    assert_eq!(i.end, 400);
    assert_eq!(i.duration(), 300);

    // Minimum valid interval: width = 1.
    let unit = Interval::new(999, 1_000).unwrap();
    assert_eq!(unit.duration(), 1);
}

#[test]
fn adversarial_zero_width_interval_rejected() {
    let err = Interval::new(500, 500).unwrap_err();
    assert!(
        matches!(
            err,
            IntervalError::EmptyOrInverted {
                start: 500,
                end: 500
            }
        ),
        "zero-width interval must be rejected as EmptyOrInverted"
    );
}

#[test]
fn adversarial_inverted_interval_rejected() {
    let err = Interval::new(900_000, 400_000).unwrap_err();
    assert!(
        matches!(
            err,
            IntervalError::EmptyOrInverted {
                start: 900_000,
                end: 400_000
            }
        ),
        "inverted interval [900k,400k) must be rejected as EmptyOrInverted"
    );
}

// ── Boundary and edge cases ───────────────────────────────────────────────────

#[test]
fn unit_gap_before_is_not_meets() {
    // a ends at 999, b starts at 1_000 — gap of 1 → Before (not Meets).
    let a = iv(0, 999);
    let b = iv(1_000, 2_000);
    assert_eq!(
        compute_relation(a, b),
        AllenRelation::Before,
        "a 1-unit gap must produce Before, not Meets"
    );
    assert_eq!(compute_relation(b, a), AllenRelation::After);
}

#[test]
fn relation_compute_is_deterministic() {
    // Same inputs must produce identical outputs on repeated calls.
    for _ in 0..100 {
        assert_eq!(
            compute_relation(active(), governance()),
            AllenRelation::Contains
        );
        assert_eq!(
            compute_relation(governance(), active()),
            AllenRelation::During
        );
    }
}

// ── Full lifecycle chain transitivity ────────────────────────────────────────

#[test]
fn lifecycle_phase_chain_matches_intended_causal_order() {
    // Auditor verifies the intended phase ordering:
    // genesis → warmup → (active starts overlapping) → ... → shutdown → tail
    //
    // Expected chain:
    //   genesis   Meets    warmup       (warmup picks up immediately)
    //   genesis   Before   active       (active starts much later)
    //   warmup    Overlaps active       (warmup's tail overlaps active's start)
    //   active    Contains flash_loan   (flash loan is strictly inside active)
    //   active    Contains governance   (governance is strictly inside active)
    //   active    Overlaps audit        (active ends at 900k, inside audit's [800k,950k))
    //   active    Meets    shutdown     (active.end = shutdown.start = 900k)
    //   shutdown  Before   tail         (tail starts after shutdown ends + 1-unit gap)
    //
    assert_eq!(compute_relation(genesis(), warmup()), AllenRelation::Meets);
    assert_eq!(compute_relation(genesis(), active()), AllenRelation::Before);
    assert_eq!(
        compute_relation(warmup(), active()),
        AllenRelation::Overlaps
    );
    assert_eq!(
        compute_relation(active(), flash_loan()),
        AllenRelation::Contains
    );
    assert_eq!(
        compute_relation(active(), governance()),
        AllenRelation::Contains
    );
    assert_eq!(compute_relation(active(), audit()), AllenRelation::Overlaps);
    assert_eq!(compute_relation(active(), shutdown()), AllenRelation::Meets);
    assert_eq!(compute_relation(shutdown(), tail()), AllenRelation::Before);
}

// ── Governance sub-window relations ──────────────────────────────────────────

#[test]
fn governance_sub_window_relations_all_correct() {
    // gov_sub [450k,650k) is strictly inside governance [400k,700k).
    assert_eq!(
        compute_relation(gov_sub(), governance()),
        AllenRelation::During,
        "gov_sub must be During governance"
    );
    assert_eq!(
        compute_relation(governance(), gov_sub()),
        AllenRelation::Contains,
        "governance must Contain gov_sub"
    );

    // gov_wider [400k,800k) starts with governance but ends later.
    assert_eq!(
        compute_relation(gov_wider(), governance()),
        AllenRelation::StartedBy
    );
    assert_eq!(
        compute_relation(governance(), gov_wider()),
        AllenRelation::Starts
    );

    // gov_later [500k,700k) shares governance's end but starts later.
    assert_eq!(
        compute_relation(gov_later(), governance()),
        AllenRelation::Finishes
    );
    assert_eq!(
        compute_relation(governance(), gov_later()),
        AllenRelation::FinishedBy
    );

    // gov_copy is identical to governance.
    assert_eq!(
        compute_relation(gov_copy(), governance()),
        AllenRelation::Equals
    );
    assert_eq!(
        compute_relation(governance(), gov_copy()),
        AllenRelation::Equals
    );
}
