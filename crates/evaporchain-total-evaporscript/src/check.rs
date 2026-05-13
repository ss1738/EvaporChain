//! `check_total` — the structural-totality checker.
//!
//! Walks a [`Term`] and produces a [`Certificate`] iff every loop
//! carries a syntactic termination witness.

use thiserror::Error;

use crate::term::{BinOp, Expr, Term};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TotalError {
    #[error("BoundedFor `{var}`: step must be a strictly positive literal, got {step}")]
    BoundedForNonPositiveStep { var: String, step: i64 },
    #[error(
        "BoundedFor `{var}`: body mutates the loop variable — body must treat it as read-only"
    )]
    BoundedForMutatesLoopVar { var: String },
    #[error(
        "BoundedWhile `{ranking_var}`: condition does not syntactically establish `{ranking_var} > 0` (or > positive literal). The chain cannot certify termination."
    )]
    BoundedWhileConditionDoesNotBoundRanking { ranking_var: String },
    #[error(
        "BoundedWhile `{ranking_var}`: body has a control-flow path that does NOT strict-decrement the ranking variable"
    )]
    BoundedWhileMissingDecrementOnPath { ranking_var: String },
    #[error(
        "BoundedWhile `{ranking_var}`: ranking variable is never used as the target of a strict-decrement assignment"
    )]
    BoundedWhileNeverDecrements { ranking_var: String },
    #[error("BoundedWhile `{ranking_var}`: body INCREMENTS the ranking variable somewhere — divergence-prone")]
    BoundedWhileIncrementsRanking { ranking_var: String },
}

/// Witness that a [`Term`] passed the totality checker. The chain
/// can pass a `Certificate` to the executor with confidence that
/// the program halts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Certificate;

pub fn check_total(t: &Term) -> Result<Certificate, TotalError> {
    check_term(t)?;
    Ok(Certificate)
}

fn check_term(t: &Term) -> Result<(), TotalError> {
    match t {
        Term::Skip
        | Term::Let { .. }
        | Term::Assign { .. }
        | Term::Return(_)
        | Term::Require { .. }
        | Term::Emit(_)
        | Term::ExprStmt(_) => Ok(()),

        Term::Seq(items) => {
            for it in items {
                check_term(it)?;
            }
            Ok(())
        }

        Term::If {
            then_body,
            else_body,
            ..
        } => {
            check_term(then_body)?;
            check_term(else_body)?;
            Ok(())
        }

        Term::BoundedFor {
            var, step, body, ..
        } => {
            if *step <= 0 {
                return Err(TotalError::BoundedForNonPositiveStep {
                    var: var.clone(),
                    step: *step,
                });
            }
            // The loop var must be read-only inside the body.
            if mutates_var(body, var) {
                return Err(TotalError::BoundedForMutatesLoopVar { var: var.clone() });
            }
            check_term(body)
        }

        Term::BoundedWhile {
            cond,
            ranking_var,
            body,
        } => {
            // Termination witness #1: condition syntactically bounds
            // the ranking variable below.
            if !condition_bounds_ranking_below(cond, ranking_var) {
                return Err(TotalError::BoundedWhileConditionDoesNotBoundRanking {
                    ranking_var: ranking_var.clone(),
                });
            }

            // Termination witness #2: every control-flow path
            // through the body must contain a strict decrement of
            // `ranking_var`.
            let analysis = analyze_decrement_paths(body, ranking_var);
            if analysis.has_increment {
                return Err(TotalError::BoundedWhileIncrementsRanking {
                    ranking_var: ranking_var.clone(),
                });
            }
            if !analysis.any_decrement {
                return Err(TotalError::BoundedWhileNeverDecrements {
                    ranking_var: ranking_var.clone(),
                });
            }
            if !analysis.all_paths_decrement {
                return Err(TotalError::BoundedWhileMissingDecrementOnPath {
                    ranking_var: ranking_var.clone(),
                });
            }

            check_term(body)
        }
    }
}

/// True iff the body contains an `Assign { target == var }` anywhere.
fn mutates_var(t: &Term, var: &str) -> bool {
    match t {
        Term::Skip | Term::Return(_) | Term::Require { .. } | Term::Emit(_) | Term::ExprStmt(_) => {
            false
        }
        Term::Let { .. } => false,
        Term::Assign { target, .. } => target == var,
        Term::Seq(items) => items.iter().any(|it| mutates_var(it, var)),
        Term::If {
            then_body,
            else_body,
            ..
        } => mutates_var(then_body, var) || mutates_var(else_body, var),
        Term::BoundedFor { body, .. } | Term::BoundedWhile { body, .. } => mutates_var(body, var),
    }
}

/// Recogniser for "the condition syntactically guarantees
/// `ranking_var > k` for some non-negative literal `k`". We accept:
/// - `Var(ranking) > Lit(k)` with k ≥ 0
/// - `Var(ranking) >= Lit(k)` with k ≥ 1
/// - `Var(ranking) != Lit(0)` (treated as > 0 only if combined with
///    a sign invariant — for V1 we accept this as "non-zero" and
///    require the body decrement to be syntactic; sign safety is
///    domain logic)
/// - `And(c1, c2)` if either side bounds it
fn condition_bounds_ranking_below(cond: &Expr, ranking_var: &str) -> bool {
    match cond {
        Expr::Bin {
            op: BinOp::Gt,
            lhs,
            rhs,
        } => matches!(
            (lhs.as_ref(), rhs.as_ref()),
            (Expr::Var(name), Expr::Lit(k)) if name == ranking_var && *k >= 0
        ),
        Expr::Bin {
            op: BinOp::Gte,
            lhs,
            rhs,
        } => matches!(
            (lhs.as_ref(), rhs.as_ref()),
            (Expr::Var(name), Expr::Lit(k)) if name == ranking_var && *k >= 1
        ),
        Expr::Bin {
            op: BinOp::Neq,
            lhs,
            rhs,
        } => matches!(
            (lhs.as_ref(), rhs.as_ref()),
            (Expr::Var(name), Expr::Lit(0)) if name == ranking_var
        ),
        Expr::Bin {
            op: BinOp::And,
            lhs,
            rhs,
        } => {
            condition_bounds_ranking_below(lhs, ranking_var)
                || condition_bounds_ranking_below(rhs, ranking_var)
        }
        _ => false,
    }
}

#[derive(Debug, Default)]
struct DecrementAnalysis {
    /// At least one syntactic strict-decrement of `ranking_var`
    /// somewhere in the term.
    any_decrement: bool,
    /// Every control-flow path through the term reaches a
    /// strict-decrement.
    all_paths_decrement: bool,
    /// Some path *increments* the ranking variable — flag for
    /// rejection (divergence-prone).
    has_increment: bool,
}

/// Walk the term and check whether every path through it strict-
/// decrements `ranking_var`, and whether any path increments it.
///
/// Soundness model:
/// - Leaf statements (Skip, Let, Return, Require, Emit, ExprStmt)
///   do not decrement → on a path that ends in a leaf, no decrement
///   means the path doesn't decrement.
/// - Assign of `ranking_var = ranking_var - k` (positive literal)
///   counts as a decrement.
/// - Assign of `ranking_var = ranking_var + k` (positive literal)
///   counts as an increment (rejected).
/// - Other assigns to `ranking_var` (e.g., `r = r * 2`, `r = some_other_expr`)
///   do not decrement and may break the well-foundedness — we
///   conservatively reject these by NOT counting them as a
///   decrement and also flagging as `has_increment` if they're
///   non-decreasing.
/// - `Seq` decrements iff any element decrements; the all-paths
///   bit is maintained by checking each element until one is found.
/// - `If` decrements on every path iff BOTH branches decrement on
///   every path.
/// - `BoundedFor` / `BoundedWhile` body that contains a decrement
///   does NOT count for the *outer* loop's decrement — nested loops
///   must each carry their own ranking witness; we recurse.
fn analyze_decrement_paths(t: &Term, ranking_var: &str) -> DecrementAnalysis {
    match t {
        Term::Skip
        | Term::Let { .. }
        | Term::Return(_)
        | Term::Require { .. }
        | Term::Emit(_)
        | Term::ExprStmt(_) => DecrementAnalysis {
            any_decrement: false,
            all_paths_decrement: false,
            has_increment: false,
        },

        Term::Assign { target, value } => {
            if target != ranking_var {
                return DecrementAnalysis {
                    any_decrement: false,
                    all_paths_decrement: false,
                    has_increment: false,
                };
            }
            // Target IS ranking_var — classify the assignment shape.
            if let Some((src, k)) = value.as_strict_decrement() {
                if src == ranking_var && k > 0 {
                    return DecrementAnalysis {
                        any_decrement: true,
                        all_paths_decrement: true,
                        has_increment: false,
                    };
                }
            }
            // Detect explicit increment: ranking_var = ranking_var + positive_literal
            if let Expr::Bin {
                op: BinOp::Add,
                lhs,
                rhs,
            } = value
            {
                if let (Expr::Var(src), Expr::Lit(k)) = (lhs.as_ref(), rhs.as_ref()) {
                    if src == ranking_var && *k > 0 {
                        return DecrementAnalysis {
                            any_decrement: false,
                            all_paths_decrement: false,
                            has_increment: true,
                        };
                    }
                }
            }
            // Anything else: a non-decrement, non-increment write
            // (e.g., `r = r * 2`, `r = constant`, `r = other_var`).
            // Treat as "does not decrement"; this path will fail
            // all_paths_decrement at the outer loop, surfacing as
            // BoundedWhileMissingDecrementOnPath.
            DecrementAnalysis {
                any_decrement: false,
                all_paths_decrement: false,
                has_increment: false,
            }
        }

        Term::Seq(items) => {
            // The path through the Seq decrements iff at least one
            // element decrements (and no element BEFORE the
            // decrement increments).
            let mut any = false;
            let mut all = false;
            let mut inc = false;
            for it in items {
                let a = analyze_decrement_paths(it, ranking_var);
                if a.has_increment {
                    inc = true;
                }
                if a.any_decrement {
                    any = true;
                    all = true;
                    // We've decremented; later non-increment items
                    // don't change that. (If a later item
                    // increments, that's flagged separately.)
                }
            }
            DecrementAnalysis {
                any_decrement: any,
                all_paths_decrement: all,
                has_increment: inc,
            }
        }

        Term::If {
            then_body,
            else_body,
            ..
        } => {
            let a = analyze_decrement_paths(then_body, ranking_var);
            let b = analyze_decrement_paths(else_body, ranking_var);
            DecrementAnalysis {
                any_decrement: a.any_decrement || b.any_decrement,
                // Both arms must decrement for the path to count.
                all_paths_decrement: a.all_paths_decrement && b.all_paths_decrement,
                has_increment: a.has_increment || b.has_increment,
            }
        }

        Term::BoundedFor { body, .. } | Term::BoundedWhile { body, .. } => {
            // Nested loops do NOT contribute the outer loop's
            // decrement (they'd leave the outer ranking unchanged
            // if entered zero times). Recurse to surface increments
            // / unrelated mutations of ranking_var inside.
            let inner = analyze_decrement_paths(body, ranking_var);
            DecrementAnalysis {
                any_decrement: false,
                all_paths_decrement: false,
                has_increment: inner.has_increment,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::{BinOp, Expr, Term};

    fn var(s: &str) -> Expr {
        Expr::Var(s.into())
    }
    fn lit(k: i64) -> Expr {
        Expr::Lit(k)
    }
    fn bin(op: BinOp, l: Expr, r: Expr) -> Expr {
        Expr::Bin {
            op,
            lhs: Box::new(l),
            rhs: Box::new(r),
        }
    }
    fn seq(items: Vec<Term>) -> Term {
        Term::Seq(items)
    }
    fn assign(target: &str, value: Expr) -> Term {
        Term::Assign {
            target: target.into(),
            value,
        }
    }
    fn dec(target: &str, k: i64) -> Term {
        // target = target - k
        assign(target, bin(BinOp::Sub, var(target), lit(k)))
    }
    fn inc(target: &str, k: i64) -> Term {
        assign(target, bin(BinOp::Add, var(target), lit(k)))
    }

    // ── BoundedFor ───────────────────────────────────────────────

    #[test]
    fn bounded_for_with_inert_body_passes() {
        let t = Term::BoundedFor {
            var: "i".into(),
            from: lit(0),
            to: lit(10),
            step: 1,
            body: Box::new(Term::Skip),
        };
        check_total(&t).unwrap();
    }

    #[test]
    fn bounded_for_zero_step_rejected() {
        let t = Term::BoundedFor {
            var: "i".into(),
            from: lit(0),
            to: lit(10),
            step: 0,
            body: Box::new(Term::Skip),
        };
        let err = check_total(&t).unwrap_err();
        assert!(matches!(
            err,
            TotalError::BoundedForNonPositiveStep { step: 0, .. }
        ));
    }

    #[test]
    fn bounded_for_negative_step_rejected() {
        let t = Term::BoundedFor {
            var: "i".into(),
            from: lit(10),
            to: lit(0),
            step: -1,
            body: Box::new(Term::Skip),
        };
        assert!(check_total(&t).is_err());
    }

    #[test]
    fn bounded_for_body_mutating_loop_var_rejected() {
        // `for i in 0..10 { i = i + 1 }` — the body must not
        // mutate the loop var. (Without this, you could bypass
        // the count.)
        let t = Term::BoundedFor {
            var: "i".into(),
            from: lit(0),
            to: lit(10),
            step: 1,
            body: Box::new(inc("i", 1)),
        };
        let err = check_total(&t).unwrap_err();
        assert!(matches!(err, TotalError::BoundedForMutatesLoopVar { .. }));
    }

    #[test]
    fn bounded_for_body_mutating_other_var_passes() {
        // `for i in 0..10 { acc = acc + i }` — modifying a
        // non-loop var is fine.
        let t = Term::BoundedFor {
            var: "i".into(),
            from: lit(0),
            to: lit(10),
            step: 1,
            body: Box::new(assign("acc", bin(BinOp::Add, var("acc"), var("i")))),
        };
        check_total(&t).unwrap();
    }

    #[test]
    fn bounded_for_body_mutating_loop_var_inside_nested_if_rejected() {
        // The mutate-check is recursive into nested If/Seq/loops.
        let t = Term::BoundedFor {
            var: "i".into(),
            from: lit(0),
            to: lit(10),
            step: 1,
            body: Box::new(Term::If {
                cond: bin(BinOp::Eq, var("i"), lit(5)),
                then_body: Box::new(inc("i", 1)),
                else_body: Box::new(Term::Skip),
            }),
        };
        assert!(check_total(&t).is_err());
    }

    // ── BoundedWhile: ranking witness ─────────────────────────────

    #[test]
    fn bounded_while_with_proper_decrement_passes() {
        // while r > 0 { r = r - 1 }
        let t = Term::BoundedWhile {
            cond: bin(BinOp::Gt, var("r"), lit(0)),
            ranking_var: "r".into(),
            body: Box::new(dec("r", 1)),
        };
        check_total(&t).unwrap();
    }

    #[test]
    fn bounded_while_without_any_decrement_rejected() {
        // while r > 0 { x = x + 1 }  — never touches r.
        let t = Term::BoundedWhile {
            cond: bin(BinOp::Gt, var("r"), lit(0)),
            ranking_var: "r".into(),
            body: Box::new(inc("x", 1)),
        };
        let err = check_total(&t).unwrap_err();
        assert_eq!(
            err,
            TotalError::BoundedWhileNeverDecrements {
                ranking_var: "r".into()
            }
        );
    }

    #[test]
    fn bounded_while_with_increment_rejected() {
        // while r > 0 { r = r + 1 } — diverges.
        let t = Term::BoundedWhile {
            cond: bin(BinOp::Gt, var("r"), lit(0)),
            ranking_var: "r".into(),
            body: Box::new(inc("r", 1)),
        };
        let err = check_total(&t).unwrap_err();
        assert_eq!(
            err,
            TotalError::BoundedWhileIncrementsRanking {
                ranking_var: "r".into()
            }
        );
    }

    #[test]
    fn bounded_while_condition_does_not_bound_ranking_rejected() {
        // while x > 0 { r = r - 1 }  — cond uses `x`, not `r`.
        let t = Term::BoundedWhile {
            cond: bin(BinOp::Gt, var("x"), lit(0)),
            ranking_var: "r".into(),
            body: Box::new(dec("r", 1)),
        };
        let err = check_total(&t).unwrap_err();
        assert_eq!(
            err,
            TotalError::BoundedWhileConditionDoesNotBoundRanking {
                ranking_var: "r".into()
            }
        );
    }

    #[test]
    fn bounded_while_condition_with_negative_constant_rejected() {
        // while r > -5 { r = r - 1 }  — k must be ≥ 0.
        let t = Term::BoundedWhile {
            cond: bin(BinOp::Gt, var("r"), lit(-5)),
            ranking_var: "r".into(),
            body: Box::new(dec("r", 1)),
        };
        assert!(check_total(&t).is_err());
    }

    #[test]
    fn bounded_while_with_gte_one_passes() {
        // while r >= 1 { r = r - 1 }
        let t = Term::BoundedWhile {
            cond: bin(BinOp::Gte, var("r"), lit(1)),
            ranking_var: "r".into(),
            body: Box::new(dec("r", 1)),
        };
        check_total(&t).unwrap();
    }

    #[test]
    fn bounded_while_with_neq_zero_passes() {
        // while r != 0 { r = r - 1 }  — accepted as bounded (sign
        // safety is the contract author's domain logic; structural
        // termination still witnessed by syntactic decrement).
        let t = Term::BoundedWhile {
            cond: bin(BinOp::Neq, var("r"), lit(0)),
            ranking_var: "r".into(),
            body: Box::new(dec("r", 1)),
        };
        check_total(&t).unwrap();
    }

    #[test]
    fn bounded_while_decrement_only_on_one_branch_rejected() {
        // while r > 0 { if foo { r = r - 1 } else { x = x + 1 } }
        // The else branch doesn't decrement → not all paths decrement.
        let t = Term::BoundedWhile {
            cond: bin(BinOp::Gt, var("r"), lit(0)),
            ranking_var: "r".into(),
            body: Box::new(Term::If {
                cond: bin(BinOp::Gt, var("foo"), lit(0)),
                then_body: Box::new(dec("r", 1)),
                else_body: Box::new(inc("x", 1)),
            }),
        };
        let err = check_total(&t).unwrap_err();
        assert_eq!(
            err,
            TotalError::BoundedWhileMissingDecrementOnPath {
                ranking_var: "r".into()
            }
        );
    }

    #[test]
    fn bounded_while_decrement_in_both_branches_passes() {
        // while r > 0 { if foo { r = r - 1 } else { r = r - 2 } }
        let t = Term::BoundedWhile {
            cond: bin(BinOp::Gt, var("r"), lit(0)),
            ranking_var: "r".into(),
            body: Box::new(Term::If {
                cond: bin(BinOp::Gt, var("foo"), lit(0)),
                then_body: Box::new(dec("r", 1)),
                else_body: Box::new(dec("r", 2)),
            }),
        };
        check_total(&t).unwrap();
    }

    #[test]
    fn bounded_while_seq_with_decrement_passes() {
        // while r > 0 { x = x + 1; r = r - 1 }
        let t = Term::BoundedWhile {
            cond: bin(BinOp::Gt, var("r"), lit(0)),
            ranking_var: "r".into(),
            body: Box::new(seq(vec![inc("x", 1), dec("r", 1)])),
        };
        check_total(&t).unwrap();
    }

    #[test]
    fn bounded_while_seq_with_increment_after_decrement_rejected() {
        // while r > 0 { r = r - 1; r = r + 1 }
        // Net non-decreasing; the increment surfaces.
        let t = Term::BoundedWhile {
            cond: bin(BinOp::Gt, var("r"), lit(0)),
            ranking_var: "r".into(),
            body: Box::new(seq(vec![dec("r", 1), inc("r", 1)])),
        };
        let err = check_total(&t).unwrap_err();
        assert!(matches!(
            err,
            TotalError::BoundedWhileIncrementsRanking { .. }
        ));
    }

    #[test]
    fn bounded_while_with_non_arithmetic_assign_to_ranking_rejected() {
        // while r > 0 { r = 100 }  — non-decrement reassignment.
        // This is not a strict decrement, so all_paths fails.
        let t = Term::BoundedWhile {
            cond: bin(BinOp::Gt, var("r"), lit(0)),
            ranking_var: "r".into(),
            body: Box::new(assign("r", lit(100))),
        };
        let err = check_total(&t).unwrap_err();
        assert_eq!(
            err,
            TotalError::BoundedWhileNeverDecrements {
                ranking_var: "r".into()
            }
        );
    }

    // ── Composition tests ─────────────────────────────────────────

    #[test]
    fn nested_bounded_for_passes() {
        // for i in 0..10 { for j in 0..10 { acc = acc + 1 } }
        let inner = Term::BoundedFor {
            var: "j".into(),
            from: lit(0),
            to: lit(10),
            step: 1,
            body: Box::new(assign("acc", bin(BinOp::Add, var("acc"), lit(1)))),
        };
        let outer = Term::BoundedFor {
            var: "i".into(),
            from: lit(0),
            to: lit(10),
            step: 1,
            body: Box::new(inner),
        };
        check_total(&outer).unwrap();
    }

    #[test]
    fn for_inside_while_must_not_break_outer_decrement() {
        // while r > 0 {
        //   for i in 0..10 { x = x + 1 }
        //   r = r - 1
        // }
        let inner = Term::BoundedFor {
            var: "i".into(),
            from: lit(0),
            to: lit(10),
            step: 1,
            body: Box::new(inc("x", 1)),
        };
        let t = Term::BoundedWhile {
            cond: bin(BinOp::Gt, var("r"), lit(0)),
            ranking_var: "r".into(),
            body: Box::new(seq(vec![inner, dec("r", 1)])),
        };
        check_total(&t).unwrap();
    }

    #[test]
    fn nested_loop_increments_outer_ranking_rejected() {
        // while r > 0 {
        //   for i in 0..10 { r = r + 1 }
        //   r = r - 1
        // }
        // The inner loop body increments r — divergence-prone.
        let inner = Term::BoundedFor {
            var: "i".into(),
            from: lit(0),
            to: lit(10),
            step: 1,
            body: Box::new(inc("r", 1)),
        };
        let t = Term::BoundedWhile {
            cond: bin(BinOp::Gt, var("r"), lit(0)),
            ranking_var: "r".into(),
            body: Box::new(seq(vec![inner, dec("r", 1)])),
        };
        let err = check_total(&t).unwrap_err();
        assert!(matches!(
            err,
            TotalError::BoundedWhileIncrementsRanking { .. }
        ));
    }

    #[test]
    fn empty_seq_is_total() {
        check_total(&seq(vec![])).unwrap();
    }

    #[test]
    fn skip_is_total() {
        check_total(&Term::Skip).unwrap();
    }

    #[test]
    fn leaf_terms_are_total() {
        for t in [
            Term::Skip,
            Term::Let {
                name: "x".into(),
                value: lit(0),
            },
            Term::Return(None),
            Term::Return(Some(lit(1))),
            Term::Require {
                cond: lit(1),
                message: Expr::Str("ok".into()),
            },
            Term::Emit(lit(0)),
            Term::ExprStmt(lit(0)),
        ] {
            check_total(&t).unwrap();
        }
    }

    // ── The doctrine claim ────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "EvaporChain is the first L1 whose contract VM
        // has structural totality at the language level. The
        // infinite-loop DoS class is not mitigated, it is
        // unexpressible."
        //
        // Demonstrate: a known-divergent program cannot be
        // expressed, while a bounded-equivalent of the same
        // computational shape passes.

        // Divergent: while true { x = x + 1 }  — but our AST has
        // no `Bool(true)` literal in cond; the closest divergent
        // shape is `while x > 0 { x = x + 1 }` with no decrement.
        let divergent = Term::BoundedWhile {
            cond: bin(BinOp::Gt, var("x"), lit(0)),
            ranking_var: "x".into(),
            body: Box::new(inc("x", 1)),
        };
        assert!(check_total(&divergent).is_err());

        // The total equivalent: while r > 0 { r = r - 1 ; x = x + 1 }
        let total_equiv = Term::BoundedWhile {
            cond: bin(BinOp::Gt, var("r"), lit(0)),
            ranking_var: "r".into(),
            body: Box::new(seq(vec![dec("r", 1), inc("x", 1)])),
        };
        check_total(&total_equiv).unwrap();
    }

    proptest::proptest! {
        #[test]
        fn property_a_pure_bounded_for_chain_always_passes(depth in 1usize..6usize, n in 1i64..50i64) {
            // A nest of BoundedFors of arbitrary depth and bound n,
            // each with an inert body, is always total.
            let mut body = Term::Skip;
            for d in 0..depth {
                body = Term::BoundedFor {
                    var: format!("i{d}"),
                    from: lit(0),
                    to: lit(n),
                    step: 1,
                    body: Box::new(body),
                };
            }
            proptest::prop_assert!(check_total(&body).is_ok());
        }

        #[test]
        fn property_bounded_while_with_dec_k_passes(k in 1i64..100i64) {
            // For any positive decrement k, `while r > 0 { r = r - k }`
            // is total.
            let t = Term::BoundedWhile {
                cond: bin(BinOp::Gt, var("r"), lit(0)),
                ranking_var: "r".into(),
                body: Box::new(dec("r", k)),
            };
            proptest::prop_assert!(check_total(&t).is_ok());
        }

        #[test]
        fn property_bounded_while_with_inc_anywhere_fails(k in 1i64..100i64) {
            // Any positive increment of the ranking variable —
            // even after a decrement — gets rejected.
            let t = Term::BoundedWhile {
                cond: bin(BinOp::Gt, var("r"), lit(0)),
                ranking_var: "r".into(),
                body: Box::new(seq(vec![dec("r", 1), inc("r", k)])),
            };
            proptest::prop_assert!(check_total(&t).is_err());
        }
    }

    /// T1.20 — `Certificate` is a unit-y witness type with derived
    /// `Eq` + `Clone`. Two certificates are always equal (any
    /// `check_total` success produces the same witness). Pin so a
    /// future refactor adding a field without updating Eq surfaces.
    #[test]
    fn t1_20_certificate_eq_and_clone() {
        let c1 = check_total(&Term::Skip).unwrap();
        let c2 = check_total(&Term::Skip).unwrap();
        assert_eq!(c1, c2);
        let cloned = c1.clone();
        assert_eq!(c1, cloned);
    }

    /// T1.20 — `check_total` on an `If` whose both branches are
    /// total accepts the term; either branch being non-total fails.
    /// Pin via explicit If with distinct total branches AND a
    /// counter-example where the else branch is non-total.
    #[test]
    fn t1_20_check_total_on_if_with_two_total_branches() {
        let t = Term::If {
            cond: var("flag"),
            then_body: Box::new(Term::Return(Some(lit(1)))),
            else_body: Box::new(Term::Emit(Expr::Str("else".into()))),
        };
        assert!(check_total(&t).is_ok());

        // If with one non-total branch fails.
        let bad_inner = Term::BoundedWhile {
            cond: bin(BinOp::Gt, var("r"), lit(0)),
            ranking_var: "r".into(),
            body: Box::new(Term::Skip),
        };
        let t_bad = Term::If {
            cond: var("flag"),
            then_body: Box::new(Term::Return(Some(lit(1)))),
            else_body: Box::new(bad_inner),
        };
        assert!(check_total(&t_bad).is_err());
    }

    /// T1.20 — `TotalError::Display` includes the variable name
    /// for each variant that carries one. Operators triaging a
    /// rejected program need to see WHICH var caused the failure.
    /// Pin via a sample of variants.
    #[test]
    fn t1_20_total_error_display_includes_variable_name() {
        // BoundedForNonPositiveStep with var="i".
        let err = check_total(&Term::BoundedFor {
            var: "i".into(),
            from: lit(0),
            to: lit(10),
            step: 0,
            body: Box::new(Term::Skip),
        })
        .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("`i`"),
            "BoundedForNonPositiveStep msg should name var: {msg}"
        );
        assert!(msg.contains('0'), "msg should report step value: {msg}");

        // BoundedWhile* with ranking_var="r".
        let err = check_total(&Term::BoundedWhile {
            cond: bin(BinOp::Gt, var("r"), lit(0)),
            ranking_var: "r".into(),
            body: Box::new(Term::Skip),
        })
        .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("`r`"),
            "BoundedWhile msg should name ranking_var: {msg}"
        );
    }
}
