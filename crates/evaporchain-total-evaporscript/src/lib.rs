//! Total-Programming EvaporScript — structural totality checker.
//!
//! ## What this crate is
//!
//! A restricted AST and a static checker that statically rejects
//! every non-total program. Every loop carries a syntactic
//! termination witness:
//!
//! - [`Term::BoundedFor`] is a counted loop. The body is statically
//!   forbidden from mutating the loop variable, so iteration is
//!   bounded by `(to - from) / step` at type level.
//! - [`Term::BoundedWhile`] carries a *named ranking variable*. The
//!   loop condition must syntactically guarantee the ranking
//!   variable is positive, and the body must contain a syntactic
//!   strict-decrement of that ranking variable on every control-
//!   flow path. The chain knows the loop terminates because it
//!   cannot encode a `BoundedWhile` whose ranking variable doesn't
//!   strictly decrease.
//!
//! There is **no general `while`**. There is **no general
//!   recursion**. The only recursion construct is structural:
//!   nested `BoundedFor` / `BoundedWhile` whose ranking is itself
//!   total.
//!
//! ## Why this kills the infinite-loop DoS class
//!
//! Gas metering kills *individual* infinite-loop attacks by
//! running the program until it hits the gas limit. That works,
//! but every contract still has the *type* "may not terminate."
//! Total programming makes "this program terminates" a
//! *type-level* fact: the chain knows before execution that any
//! program that passed the checker will halt. The infinite-loop
//! DoS class doesn't get mitigated, it ceases to be expressible.
//!
//! This is the paradigm-grade claim: EvaporChain is the first L1
//! whose contract VM has structural totality at the language
//! level.
//!
//! ## What this crate does NOT do
//!
//! - It does NOT prove correctness. A program can be total and
//!   wrong. (Total Coq programs can be wrong about their spec
//!   too.)
//! - It does NOT verify the ranking decreases by *value*. It
//!   verifies the body contains a syntactic strict-decrement
//!   instruction `x = x - k` for a positive literal `k`. This is
//!   sound but conservative: an arithmetic program that
//!   terminates by some non-syntactic reason will be rejected.
//!   The escape hatch is to express the program in terms of a
//!   ranking variable that the checker can see.
//! - It does NOT lift legacy EvaporScript (`evaporchain-script`)
//!   parser ASTs into `Term`. A future lift can be added; for V1
//!   the chain code that wants total guarantees writes `Term`s
//!   directly.
//!
//! ## Module map
//!
//! - [`term`] — the restricted [`Term`] / [`Expr`] AST.
//! - [`check`] — [`check_total`] + [`TotalError`] + [`Certificate`].

pub mod check;
pub mod term;

pub use check::{check_total, Certificate, TotalError};
pub use term::{BinOp, Expr, Term, UnaryOp};

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Total-Programming EvaporScript closes the
    /// infinite-loop DoS class as a *type-system* property. Every
    /// program that passes `check_total` carries a Certificate; the
    /// checker rejects (a) `BoundedFor` whose body mutates the loop
    /// variable, and (b) `BoundedWhile` whose body lacks a syntactic
    /// strict-decrement of its ranking variable on every CFG path."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        // (a) Total program: BoundedFor with read-only loop var.
        let total = Term::BoundedFor {
            var: "i".into(),
            from: Expr::Lit(0),
            to: Expr::Lit(10),
            step: 1,
            body: Box::new(Term::Skip),
        };
        assert!(check_total(&total).is_ok(), "trivially-total loop must certify");

        // (b) BoundedFor body mutates the loop variable → rejected.
        let mutating = Term::BoundedFor {
            var: "i".into(),
            from: Expr::Lit(0),
            to: Expr::Lit(10),
            step: 1,
            body: Box::new(Term::Assign {
                target: "i".into(),
                value: Expr::Lit(0),
            }),
        };
        assert!(matches!(
            check_total(&mutating),
            Err(TotalError::BoundedForMutatesLoopVar { .. })
        ));

        // (c) BoundedWhile with proper ranking + strict-decrement → OK.
        let bw_ok = Term::BoundedWhile {
            cond: Expr::Bin {
                op: BinOp::Gt,
                lhs: Box::new(Expr::Var("n".into())),
                rhs: Box::new(Expr::Lit(0)),
            },
            ranking_var: "n".into(),
            body: Box::new(Term::Assign {
                target: "n".into(),
                value: Expr::Bin {
                    op: BinOp::Sub,
                    lhs: Box::new(Expr::Var("n".into())),
                    rhs: Box::new(Expr::Lit(1)),
                },
            }),
        };
        assert!(check_total(&bw_ok).is_ok(), "well-formed BoundedWhile must certify");

        // (d) BoundedWhile body never strict-decrements the ranking
        // variable → rejected (divergence-prone).
        let bw_bad = Term::BoundedWhile {
            cond: Expr::Bin {
                op: BinOp::Gt,
                lhs: Box::new(Expr::Var("n".into())),
                rhs: Box::new(Expr::Lit(0)),
            },
            ranking_var: "n".into(),
            body: Box::new(Term::Skip),
        };
        assert!(check_total(&bw_bad).is_err(), "missing decrement must reject");
    }
}
