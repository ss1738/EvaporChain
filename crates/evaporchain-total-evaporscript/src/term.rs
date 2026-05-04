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
}
