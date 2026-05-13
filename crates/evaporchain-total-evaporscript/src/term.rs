//! Restricted AST for total EvaporScript.
//!
//! Three structural decisions:
//!
//! 1. **No general `while`.** The non-counted iteration construct
//!    is `BoundedWhile { ranking_var, ... }`. The checker rejects
//!    any `BoundedWhile` whose body doesn't syntactically
//!    strict-decrement `ranking_var` on every path.
//!
//! 2. **`BoundedFor`'s loop variable is read-only inside the
//!    body.** The checker walks the body and rejects any
//!    `Assign { target: <loop var> }`. This is what makes
//!    iteration bounded by `(to - from) / step` independent of
//!    body behaviour.
//!
//! 3. **All expressions are pure integer arithmetic with explicit
//!    Lit/Var.** No function calls, no map access, no array
//!    access — those add control-flow surface that's much harder
//!    to certify total in a single pass. They can be added in V2
//!    once the V1 termination calculus is locked in.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Term {
    /// No-op.
    Skip,
    /// Bind a fresh local. The chain's typed environment tracks the
    /// binding for downstream `Var` lookups.
    Let { name: String, value: Expr },
    /// Reassign an existing local (the `target`).
    Assign { target: String, value: Expr },
    /// Sequence terms; total iff all elements are total.
    Seq(Vec<Term>),
    /// Conditional; total iff both branches are total.
    If {
        cond: Expr,
        then_body: Box<Term>,
        else_body: Box<Term>,
    },
    /// Counted loop. Iterates `var` from `from` (inclusive) to `to`
    /// (exclusive) by `step` (must be a strictly positive literal —
    /// the checker enforces this). The body is statically forbidden
    /// from mutating `var`. Termination follows because `(to - from)
    /// / step` is fixed at the start of the loop.
    BoundedFor {
        var: String,
        from: Expr,
        to: Expr,
        step: i64,
        body: Box<Term>,
    },
    /// Conditional loop. Carries a `ranking_var` and a `cond`.
    /// Termination conditions, all checked statically:
    ///   - `cond` must syntactically constrain `ranking_var > 0`
    ///     (or > some positive literal).
    ///   - The body must contain a syntactic strict-decrement of
    ///     `ranking_var` (`Assign { ranking_var, Bin(Sub, ranking_var, k) }`
    ///     for positive literal `k`).
    ///   - On *every* control-flow path through the body, the
    ///     decrement must be reachable.
    BoundedWhile {
        cond: Expr,
        ranking_var: String,
        body: Box<Term>,
    },
    /// Early return from a function. Total because it's a leaf.
    Return(Option<Expr>),
    /// Assertion — aborts on false. Total because it's a leaf.
    Require { cond: Expr, message: Expr },
    /// Emit an event. Total because it's a leaf.
    Emit(Expr),
    /// Pure expression evaluated for side-effect on environment
    /// (e.g., a state read). Leaf.
    ExprStmt(Expr),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Expr {
    /// Integer literal.
    Lit(i64),
    /// Variable reference.
    Var(String),
    /// String literal — used by Require/Emit messages.
    Str(String),
    /// Binary integer operation.
    Bin {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    /// Unary operation.
    Un { op: UnaryOp, e: Box<Expr> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Neq,
    Gt,
    Lt,
    Gte,
    Lte,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnaryOp {
    Not,
    Neg,
}

impl Expr {
    /// True iff this expression is the literal `Lit(k)` for some
    /// strictly positive `k`.
    pub fn is_positive_literal(&self) -> bool {
        matches!(self, Expr::Lit(k) if *k > 0)
    }

    /// If this expression is a binary subtract with a left-Var and a
    /// right-Lit (a strict-decrement shape), returns
    /// `(var_name, decrement)`. Otherwise None.
    pub fn as_strict_decrement(&self) -> Option<(&str, i64)> {
        if let Expr::Bin {
            op: BinOp::Sub,
            lhs,
            rhs,
        } = self
        {
            if let (Expr::Var(name), Expr::Lit(k)) = (lhs.as_ref(), rhs.as_ref()) {
                if *k > 0 {
                    return Some((name.as_str(), *k));
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_decrement_recognised() {
        let e = Expr::Bin {
            op: BinOp::Sub,
            lhs: Box::new(Expr::Var("i".into())),
            rhs: Box::new(Expr::Lit(1)),
        };
        assert_eq!(e.as_strict_decrement(), Some(("i", 1)));
    }

    #[test]
    fn non_decrement_not_recognised() {
        // i + 1 is not a strict decrement.
        let plus = Expr::Bin {
            op: BinOp::Add,
            lhs: Box::new(Expr::Var("i".into())),
            rhs: Box::new(Expr::Lit(1)),
        };
        assert_eq!(plus.as_strict_decrement(), None);

        // i - 0 is not a STRICT decrement.
        let zero_dec = Expr::Bin {
            op: BinOp::Sub,
            lhs: Box::new(Expr::Var("i".into())),
            rhs: Box::new(Expr::Lit(0)),
        };
        assert_eq!(zero_dec.as_strict_decrement(), None);

        // i - (-3) — the rhs is a negative literal. Not a positive
        // strict decrement.
        let neg_dec = Expr::Bin {
            op: BinOp::Sub,
            lhs: Box::new(Expr::Var("i".into())),
            rhs: Box::new(Expr::Lit(-3)),
        };
        assert_eq!(neg_dec.as_strict_decrement(), None);
    }

    #[test]
    fn round_trip_serde() {
        let t = Term::BoundedFor {
            var: "i".into(),
            from: Expr::Lit(0),
            to: Expr::Lit(10),
            step: 1,
            body: Box::new(Term::Skip),
        };
        let s = serde_json::to_string(&t).unwrap();
        let back: Term = serde_json::from_str(&s).unwrap();
        assert_eq!(t, back);
    }

    /// T1.20 — Expr::is_positive_literal (lines 122-124).
    #[test]
    fn t1_20_expr_is_positive_literal() {
        assert!(Expr::Lit(1).is_positive_literal());
        assert!(Expr::Lit(100).is_positive_literal());
        assert!(!Expr::Lit(0).is_positive_literal());
        assert!(!Expr::Lit(-1).is_positive_literal());
        assert!(!Expr::Var("x".into()).is_positive_literal());
    }

    /// T1.20 — Expr::as_strict_decrement matches `Var - Lit(k>0)`
    /// (line 140) and rejects all other shapes.
    #[test]
    fn t1_20_expr_as_strict_decrement() {
        let dec = Expr::Bin {
            op: BinOp::Sub,
            lhs: Box::new(Expr::Var("r".into())),
            rhs: Box::new(Expr::Lit(1)),
        };
        assert_eq!(dec.as_strict_decrement(), Some(("r", 1)));

        // Lit zero — rejected.
        let zero = Expr::Bin {
            op: BinOp::Sub,
            lhs: Box::new(Expr::Var("r".into())),
            rhs: Box::new(Expr::Lit(0)),
        };
        assert!(zero.as_strict_decrement().is_none());

        // Wrong shape: Var - Var.
        let wrong = Expr::Bin {
            op: BinOp::Sub,
            lhs: Box::new(Expr::Var("r".into())),
            rhs: Box::new(Expr::Var("s".into())),
        };
        assert!(wrong.as_strict_decrement().is_none());

        // Wrong op: Add.
        let add = Expr::Bin {
            op: BinOp::Add,
            lhs: Box::new(Expr::Var("r".into())),
            rhs: Box::new(Expr::Lit(1)),
        };
        assert!(add.as_strict_decrement().is_none());

        // Not a Bin at all.
        assert!(Expr::Lit(5).as_strict_decrement().is_none());
    }

    /// T1.20 — soundness pin: `Lit(k) - Var` is NOT a strict
    /// decrement. The decrement shape required by the termination
    /// calculus is `Var - Lit(k>0)`. A refactor that loosened the
    /// pattern to "either side could be the Var" would silently
    /// accept `k - r` as a decrement of `r`, but `k - r` doesn't
    /// decrement `r` at all (it computes an unrelated value). This
    /// pin protects the BoundedWhile termination guarantee.
    #[test]
    fn t1_20_as_strict_decrement_rejects_reversed_operands() {
        // Lit - Var: this is k - r, NOT a decrement of r.
        let reversed = Expr::Bin {
            op: BinOp::Sub,
            lhs: Box::new(Expr::Lit(5)),
            rhs: Box::new(Expr::Var("r".into())),
        };
        assert!(
            reversed.as_strict_decrement().is_none(),
            "Lit - Var must NOT be classified as a strict decrement"
        );
        // Lit - Lit: also not a decrement (no variable).
        let lit_lit = Expr::Bin {
            op: BinOp::Sub,
            lhs: Box::new(Expr::Lit(10)),
            rhs: Box::new(Expr::Lit(1)),
        };
        assert!(lit_lit.as_strict_decrement().is_none());
    }

    /// T1.20 — Term variants serde round-trip. Existing test only
    /// covers `BoundedFor`; this pins `BoundedWhile`, `Seq`, `If`
    /// + nested. The chain stores compiled Total-EvaporScript
    /// programs on-disk via serde, so a variant that broke serde
    /// would silently fail to load.
    #[test]
    fn t1_20_term_variants_serde_round_trip() {
        // BoundedWhile.
        let bw = Term::BoundedWhile {
            cond: Expr::Bin {
                op: BinOp::Gt,
                lhs: Box::new(Expr::Var("r".into())),
                rhs: Box::new(Expr::Lit(0)),
            },
            ranking_var: "r".into(),
            body: Box::new(Term::Assign {
                target: "r".into(),
                value: Expr::Bin {
                    op: BinOp::Sub,
                    lhs: Box::new(Expr::Var("r".into())),
                    rhs: Box::new(Expr::Lit(1)),
                },
            }),
        };
        let s = serde_json::to_string(&bw).unwrap();
        assert_eq!(bw, serde_json::from_str::<Term>(&s).unwrap());

        // Seq containing If.
        let seq = Term::Seq(vec![
            Term::Skip,
            Term::If {
                cond: Expr::Var("flag".into()),
                then_body: Box::new(Term::Return(Some(Expr::Lit(1)))),
                else_body: Box::new(Term::Return(Some(Expr::Lit(0)))),
            },
            Term::Require {
                cond: Expr::Lit(1),
                message: Expr::Str("ok".into()),
            },
            Term::Emit(Expr::Str("event".into())),
        ]);
        let s = serde_json::to_string(&seq).unwrap();
        assert_eq!(seq, serde_json::from_str::<Term>(&s).unwrap());
    }

    /// T1.20 — `Expr::Un` variant serde round-trip. Not is_positive
    /// nor as_strict_decrement matches it, and existing tests use
    /// only `Lit`, `Var`, `Bin`. Pin both unary ops (Not, Neg).
    #[test]
    fn t1_20_expr_unary_serde_round_trip() {
        let not_e = Expr::Un {
            op: UnaryOp::Not,
            e: Box::new(Expr::Var("flag".into())),
        };
        let s = serde_json::to_string(&not_e).unwrap();
        assert_eq!(not_e, serde_json::from_str::<Expr>(&s).unwrap());

        let neg_e = Expr::Un {
            op: UnaryOp::Neg,
            e: Box::new(Expr::Lit(42)),
        };
        let s = serde_json::to_string(&neg_e).unwrap();
        assert_eq!(neg_e, serde_json::from_str::<Expr>(&s).unwrap());
    }
}
