//! Symbolic evaluation of a `Term` body against a fixed input
//! state.
//!
//! This is a small evaluator over the V1 `Term`/`Expr` shape, using
//! `HashMap<String, i64>` as the symbol environment. We evaluate
//! one pass through the body and return the post-state.

use std::collections::HashMap;

use thiserror::Error;

use evaporchain_total_evaporscript::{BinOp, Expr, Term, UnaryOp};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EvalError {
    #[error("undefined variable {0}")]
    UndefinedVar(String),
    #[error("division by zero")]
    DivByZero,
    #[error("modulo by zero")]
    ModByZero,
    #[error("nested loops not supported in fuel certification — body must be loop-free")]
    NestedLoop,
    #[error("string literal cannot appear in arithmetic context")]
    StrInArith,
    #[error("arithmetic overflow")]
    Overflow,
}

/// Symbol environment: variable name → integer value.
pub type Env = HashMap<String, i64>;

/// Evaluate one pass through `term`, mutating `env`.
///
/// **Loop-bodies must be loop-free.** A `BoundedFor` /
/// `BoundedWhile` inside the body would require iterative
/// re-execution; V1 of V2 rejects them. Nested loops are V1's
/// responsibility (each carries its own ranking witness).
pub fn evaluate(term: &Term, env: &mut Env) -> Result<(), EvalError> {
    match term {
        Term::Skip | Term::Return(_) | Term::Require { .. } | Term::Emit(_) | Term::ExprStmt(_) => {
            Ok(())
        }
        Term::Let { name, value } => {
            let v = eval_expr(value, env)?;
            env.insert(name.clone(), v);
            Ok(())
        }
        Term::Assign { target, value } => {
            let v = eval_expr(value, env)?;
            env.insert(target.clone(), v);
            Ok(())
        }
        Term::Seq(items) => {
            for it in items {
                evaluate(it, env)?;
            }
            Ok(())
        }
        Term::If { cond, then_body, else_body } => {
            let c = eval_expr(cond, env)?;
            // Treat any non-zero as true (consistent with C-style
            // boolean integers used in V1).
            if c != 0 {
                evaluate(then_body, env)?;
            } else {
                evaluate(else_body, env)?;
            }
            Ok(())
        }
        Term::BoundedFor { .. } | Term::BoundedWhile { .. } => Err(EvalError::NestedLoop),
    }
}

/// Evaluate an expression. Pure i64 with checked arithmetic.
pub fn eval_expr(e: &Expr, env: &Env) -> Result<i64, EvalError> {
    match e {
        Expr::Lit(k) => Ok(*k),
        Expr::Var(name) => env
            .get(name)
            .copied()
            .ok_or_else(|| EvalError::UndefinedVar(name.clone())),
        Expr::Str(_) => Err(EvalError::StrInArith),
        Expr::Bin { op, lhs, rhs } => {
            let a = eval_expr(lhs, env)?;
            let b = eval_expr(rhs, env)?;
            apply_bin(*op, a, b)
        }
        Expr::Un { op, e } => {
            let a = eval_expr(e, env)?;
            Ok(apply_un(*op, a))
        }
    }
}

fn apply_bin(op: BinOp, a: i64, b: i64) -> Result<i64, EvalError> {
    use BinOp::*;
    Ok(match op {
        Add => a.checked_add(b).ok_or(EvalError::Overflow)?,
        Sub => a.checked_sub(b).ok_or(EvalError::Overflow)?,
        Mul => a.checked_mul(b).ok_or(EvalError::Overflow)?,
        Div => {
            if b == 0 {
                return Err(EvalError::DivByZero);
            }
            a.checked_div(b).ok_or(EvalError::Overflow)?
        }
        Mod => {
            if b == 0 {
                return Err(EvalError::ModByZero);
            }
            a.checked_rem(b).ok_or(EvalError::Overflow)?
        }
        Eq => (a == b) as i64,
        Neq => (a != b) as i64,
        Gt => (a > b) as i64,
        Lt => (a < b) as i64,
        Gte => (a >= b) as i64,
        Lte => (a <= b) as i64,
        And => ((a != 0) && (b != 0)) as i64,
        Or => ((a != 0) || (b != 0)) as i64,
    })
}

fn apply_un(op: UnaryOp, a: i64) -> i64 {
    match op {
        UnaryOp::Not => (a == 0) as i64,
        UnaryOp::Neg => a.checked_neg().unwrap_or(i64::MIN),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_total_evaporscript::{BinOp, Expr, Term};

    fn lit(k: i64) -> Expr { Expr::Lit(k) }
    fn var(s: &str) -> Expr { Expr::Var(s.into()) }
    fn bin(op: BinOp, l: Expr, r: Expr) -> Expr {
        Expr::Bin { op, lhs: Box::new(l), rhs: Box::new(r) }
    }
    fn assign(target: &str, value: Expr) -> Term {
        Term::Assign { target: target.into(), value }
    }
    fn seq(items: Vec<Term>) -> Term {
        Term::Seq(items)
    }
    fn empty_else() -> Box<Term> {
        Box::new(Term::Skip)
    }

    // ── expression evaluation ────────────────────────────────────

    #[test]
    fn lit_evaluates_to_value() {
        let env = Env::new();
        assert_eq!(eval_expr(&lit(42), &env).unwrap(), 42);
    }

    #[test]
    fn var_lookup_succeeds() {
        let mut env = Env::new();
        env.insert("x".into(), 7);
        assert_eq!(eval_expr(&var("x"), &env).unwrap(), 7);
    }

    #[test]
    fn undefined_var_errors() {
        let env = Env::new();
        let err = eval_expr(&var("missing"), &env).unwrap_err();
        assert!(matches!(err, EvalError::UndefinedVar(_)));
    }

    #[test]
    fn arithmetic_evaluates() {
        let mut env = Env::new();
        env.insert("a".into(), 10);
        env.insert("b".into(), 3);
        let e = bin(BinOp::Mul, var("a"), bin(BinOp::Sub, var("a"), var("b")));
        // 10 * (10 - 3) = 70.
        assert_eq!(eval_expr(&e, &env).unwrap(), 70);
    }

    #[test]
    fn div_by_zero_errors() {
        let env = Env::new();
        let e = bin(BinOp::Div, lit(10), lit(0));
        let err = eval_expr(&e, &env).unwrap_err();
        assert_eq!(err, EvalError::DivByZero);
    }

    #[test]
    fn mod_by_zero_errors() {
        let env = Env::new();
        let e = bin(BinOp::Mod, lit(10), lit(0));
        let err = eval_expr(&e, &env).unwrap_err();
        assert_eq!(err, EvalError::ModByZero);
    }

    #[test]
    fn add_overflow_surfaces() {
        let env = Env::new();
        let e = bin(BinOp::Add, lit(i64::MAX), lit(1));
        let err = eval_expr(&e, &env).unwrap_err();
        assert_eq!(err, EvalError::Overflow);
    }

    // ── term evaluation ──────────────────────────────────────────

    #[test]
    fn skip_is_noop() {
        let mut env = Env::new();
        env.insert("x".into(), 5);
        evaluate(&Term::Skip, &mut env).unwrap();
        assert_eq!(env.get("x"), Some(&5));
    }

    #[test]
    fn assign_updates_env() {
        let mut env = Env::new();
        env.insert("r".into(), 10);
        evaluate(&assign("r", bin(BinOp::Sub, var("r"), lit(3))), &mut env).unwrap();
        assert_eq!(env.get("r"), Some(&7));
    }

    #[test]
    fn seq_evaluates_in_order() {
        let mut env = Env::new();
        env.insert("r".into(), 10);
        let t = seq(vec![
            assign("r", bin(BinOp::Sub, var("r"), lit(3))),
            assign("r", bin(BinOp::Mul, var("r"), lit(2))),
        ]);
        evaluate(&t, &mut env).unwrap();
        // 10 - 3 = 7; 7 * 2 = 14.
        assert_eq!(env.get("r"), Some(&14));
    }

    #[test]
    fn if_takes_then_branch_when_true() {
        let mut env = Env::new();
        env.insert("r".into(), 10);
        env.insert("flag".into(), 1);
        let t = Term::If {
            cond: var("flag"),
            then_body: Box::new(assign("r", lit(99))),
            else_body: Box::new(assign("r", lit(0))),
        };
        evaluate(&t, &mut env).unwrap();
        assert_eq!(env.get("r"), Some(&99));
    }

    #[test]
    fn if_takes_else_branch_when_false() {
        let mut env = Env::new();
        env.insert("r".into(), 10);
        env.insert("flag".into(), 0);
        let t = Term::If {
            cond: var("flag"),
            then_body: Box::new(assign("r", lit(99))),
            else_body: Box::new(assign("r", lit(0))),
        };
        evaluate(&t, &mut env).unwrap();
        assert_eq!(env.get("r"), Some(&0));
    }

    #[test]
    fn nested_loops_rejected() {
        let mut env = Env::new();
        let nested = Term::BoundedFor {
            var: "i".into(),
            from: lit(0),
            to: lit(5),
            step: 1,
            body: Box::new(Term::Skip),
        };
        let err = evaluate(&nested, &mut env).unwrap_err();
        assert_eq!(err, EvalError::NestedLoop);

        let nested_w = Term::BoundedWhile {
            cond: lit(1),
            ranking_var: "x".into(),
            body: Box::new(Term::Skip),
        };
        let _ = empty_else();
        assert_eq!(evaluate(&nested_w, &mut env).unwrap_err(), EvalError::NestedLoop);
    }
}
