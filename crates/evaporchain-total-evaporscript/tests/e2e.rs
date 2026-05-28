//! End-to-end integration tests for total-evaporscript.
//!
//! Non-trivial fixture: a sealed-auction settlement program. N bids arrive
//! (bounded for-loop over bid array index 0..N). For each bid, a nested
//! bounded-while computes the platform fee via successive subtraction
//! (integer Euclidean step). The outer loop accumulates total fees into an
//! accumulator variable. The whole program must certify as total.
//!
//! Adversarial fixture: an attacker injects a direct mutation of the outer
//! loop variable inside the fee-computation nested body — checker must catch
//! it across the nesting boundary.
//!
//! INVENTION_STACK §4.2: Total-Programming EvaporScript substrate.

use evaporchain_total_evaporscript::{check_total, BinOp, Expr, Term, TotalError};

// ── Helpers ───────────────────────────────────────────────────────────────

fn lit(n: i64) -> Expr {
    Expr::Lit(n)
}
fn var(name: &str) -> Expr {
    Expr::Var(name.into())
}
fn bin(op: BinOp, l: Expr, r: Expr) -> Expr {
    Expr::Bin {
        op,
        lhs: Box::new(l),
        rhs: Box::new(r),
    }
}
fn assign(name: &str, val: Expr) -> Term {
    Term::Assign {
        target: name.into(),
        value: val,
    }
}
fn dec(name: &str, k: i64) -> Term {
    assign(name, bin(BinOp::Sub, var(name), lit(k)))
}

// ── Non-trivial fixture ───────────────────────────────────────────────────

/// Build the sealed-auction settlement program.
///
/// Logical semantics (not executed, only structurally certified):
///
/// ```text
/// acc = 0
/// for bid_idx in 0..N {
///     rem = FEE_DIVISOR      // fee = 10% of bid value (integer approx)
///     while rem > 0 {
///         rem = rem - STEP
///     }
///     acc = acc + fee_unit   // conceptual accumulation
/// }
/// ```
///
/// The concrete `Term` tree has:
/// - Outer `BoundedFor` over `bid_idx` (0..N, step 1, body does NOT mutate
///   `bid_idx` — acc and rem are the only targets).
/// - Inner `BoundedWhile` ranking on `rem`, decrementing by `STEP`.
/// - Accumulator update on `acc` inside the outer loop after the while.
fn auction_settlement_program(n: i64, fee_divisor: i64, step: i64) -> Term {
    let inner_while = Term::BoundedWhile {
        cond: bin(BinOp::Gt, var("rem"), lit(0)),
        ranking_var: "rem".into(),
        body: Box::new(dec("rem", step)),
    };

    let outer_body = Term::Seq(vec![
        assign("rem", lit(fee_divisor)),
        inner_while,
        assign("acc", bin(BinOp::Add, var("acc"), lit(1))),
    ]);

    Term::Seq(vec![
        assign("acc", lit(0)),
        Term::BoundedFor {
            var: "bid_idx".into(),
            from: lit(0),
            to: lit(n),
            step: 1,
            body: Box::new(outer_body),
        },
    ])
}

// ── Tests ────────────────────────────────────────────────────────────────

#[test]
fn auction_settlement_certifies_as_total() {
    let prog = auction_settlement_program(10, 100, 5);
    let cert = check_total(&prog).expect("auction settlement must be certifiable as total");
    // Certificate exists (non-empty — unit type witnesses totality)
    let _ = cert;
}

#[test]
fn auction_settlement_various_sizes_all_certify() {
    // The checker is size-agnostic: total regardless of N (we certify
    // structure, not values).
    for n in [0, 1, 5, 100, 1000] {
        let prog = auction_settlement_program(n, 50, 1);
        check_total(&prog).unwrap_or_else(|e| panic!("n={n}: unexpected rejection: {e:?}"));
    }
}

#[test]
fn nested_loop_adversarial_outer_var_mutation_rejected() {
    // Attacker: inside the inner while body, try to assign to `bid_idx`
    // (the outer BoundedFor's loop variable). This must be caught even
    // though the mutation is inside a nested BoundedWhile.
    let malicious_inner_while = Term::BoundedWhile {
        cond: bin(BinOp::Gt, var("rem"), lit(0)),
        ranking_var: "rem".into(),
        body: Box::new(Term::Seq(vec![
            dec("rem", 1),
            // Adversarial: mutate the outer loop var
            assign("bid_idx", lit(0)),
        ])),
    };

    let outer_body = Term::Seq(vec![assign("rem", lit(10)), malicious_inner_while]);

    let prog = Term::BoundedFor {
        var: "bid_idx".into(),
        from: lit(0),
        to: lit(10),
        step: 1,
        body: Box::new(outer_body),
    };

    let err = check_total(&prog).expect_err("must reject outer-var mutation in nested body");
    assert!(
        matches!(err, TotalError::BoundedForMutatesLoopVar { .. }),
        "expected BoundedForMutatesLoopVar, got {err:?}"
    );
}

#[test]
fn adversarial_ranking_var_not_decremented_in_nested_else_rejected() {
    // BoundedWhile body: one branch decrements `rem`, the else branch
    // only increments an unrelated var. Missing decrement on the else
    // path means the checker must reject (divergence possible).
    let t = Term::BoundedWhile {
        cond: bin(BinOp::Gt, var("rem"), lit(0)),
        ranking_var: "rem".into(),
        body: Box::new(Term::If {
            cond: bin(BinOp::Gt, var("x"), lit(5)),
            then_body: Box::new(dec("rem", 1)),
            else_body: Box::new(assign("x", bin(BinOp::Add, var("x"), lit(1)))),
        }),
    };
    let err = check_total(&t).expect_err("missing decrement on else path must reject");
    assert!(
        matches!(
            err,
            TotalError::BoundedWhileNeverDecrements { .. }
                | TotalError::BoundedWhileConditionDoesNotBoundRanking { .. }
                | TotalError::BoundedWhileMissingDecrementOnPath { .. }
        ),
        "got {err:?}"
    );
}

#[test]
fn certified_nested_if_both_branches_decrement_passes() {
    // Both branches decrement `rem`, just by different amounts.
    // Checker must certify it.
    let t = Term::BoundedWhile {
        cond: bin(BinOp::Gt, var("rem"), lit(0)),
        ranking_var: "rem".into(),
        body: Box::new(Term::If {
            cond: bin(BinOp::Gt, var("x"), lit(5)),
            then_body: Box::new(dec("rem", 2)),
            else_body: Box::new(dec("rem", 1)),
        }),
    };
    check_total(&t).expect("both-branch decrement must certify");
}

#[test]
fn deeply_nested_total_program_certifies() {
    // Three levels of nesting: For → While → For → Seq.
    // Each level is well-formed. Must certify.
    let innermost_for = Term::BoundedFor {
        var: "k".into(),
        from: lit(0),
        to: lit(3),
        step: 1,
        body: Box::new(assign("z", bin(BinOp::Add, var("z"), var("k")))),
    };

    let mid_while = Term::BoundedWhile {
        cond: bin(BinOp::Gt, var("r"), lit(0)),
        ranking_var: "r".into(),
        body: Box::new(Term::Seq(vec![dec("r", 1), innermost_for])),
    };

    let outer_for = Term::BoundedFor {
        var: "i".into(),
        from: lit(0),
        to: lit(5),
        step: 1,
        body: Box::new(Term::Seq(vec![assign("r", lit(4)), mid_while])),
    };

    check_total(&outer_for).expect("deeply nested total program must certify");
}
