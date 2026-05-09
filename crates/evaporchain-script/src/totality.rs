//! Totality checker for mainline EvaporScript ASTs.
//!
//! Item B (V1) of the smart-contract layer: structural totality enforced
//! as a static lint pass on the mainline `Stmt`/`Expr` AST. The
//! companion `evaporchain-total-evaporscript` crate has a richer Total
//! AST with `BoundedFor` / `BoundedWhile` constructs that carry their
//! own termination witnesses; this module is the integration shim that
//! checks whether a *mainline* program would also pass under
//! total-programming semantics.
//!
//! ## V1 rule (strict)
//!
//! Reject any `Stmt::While`. The mainline `while` carries no syntactic
//! termination witness — gas metering bounds it dynamically, but we
//! cannot type-certify that it halts. A future V1.5 will recognise the
//! "while ranking > 0 with strict-decrement body" pattern and accept it
//! by translating to `BoundedWhile`; until then total mode is `while`-
//! free.
//!
//! ## What gets accepted
//!
//! Every loop-free construct: `Let`, `Assign`, `CompoundAssign`, `If`,
//! `Return`, `Require`, `Emit`, `ExprStmt`. Method calls in
//! expressions. Map and array operations. Lifecycle hooks
//! (`on_grace`, `on_refresh`, `on_evaporate`).
//!
//! ## Why this is the V1
//!
//! Looking at the seed-15 contracts in `contracts/evaporscript/`
//! (3 pilots + 12 stdlib): zero use `while`. All control flow is
//! `if`-based. The strict V1 rule lets us flip total mode on
//! immediately for the entire stdlib without porting work, and
//! establishes the API surface (`check_total_contract`,
//! `TotalityError`) that V1.5 extends without a breaking change.

use thiserror::Error;

use crate::parser::{Contract, Expr, Function, LifecycleHook, Stmt};

/// Reasons a mainline EvaporScript program fails the totality check.
///
/// Each variant pinpoints the specific construct that triggers the
/// rejection, so dApp authors get an actionable error rather than a
/// vague "non-total" stamp.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TotalityError {
    #[error(
        "totality: `while` loop in method `{method}` is not permitted under total mode \
         (the mainline grammar's while carries no syntactic termination witness; rewrite as \
         a bounded `for` or restructure as `if`-based control flow)"
    )]
    WhileNotPermitted { method: String },
    #[error(
        "totality: lifecycle hook `{hook}` contains a `while` loop — hooks must be total"
    )]
    WhileInLifecycleHook { hook: String },
}

/// Witness that a contract passed the totality check. Holding a
/// [`TotalityCertificate`] means the chain can deploy / execute the
/// contract under total mode without further loop analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TotalityCertificate;

/// Run the totality check across all methods + lifecycle hooks of a
/// parsed contract. Returns a [`TotalityCertificate`] iff every body
/// is loop-free.
pub fn check_total_contract(c: &Contract) -> Result<TotalityCertificate, TotalityError> {
    for f in &c.functions {
        check_function(f)?;
    }
    for hook in &c.lifecycle_hooks {
        check_hook(hook)?;
    }
    Ok(TotalityCertificate)
}

fn check_function(f: &Function) -> Result<(), TotalityError> {
    for stmt in &f.body {
        check_stmt(stmt, &f.name, false)?;
    }
    Ok(())
}

fn check_hook(h: &LifecycleHook) -> Result<(), TotalityError> {
    let (label, body) = match h {
        LifecycleHook::OnEvaporate(b) => ("on_evaporate", b),
        LifecycleHook::OnGrace(b) => ("on_grace", b),
        LifecycleHook::OnRefresh(b) => ("on_refresh", b),
    };
    for stmt in body {
        check_stmt(stmt, label, true)?;
    }
    Ok(())
}

fn check_stmt(s: &Stmt, ctx_name: &str, is_hook: bool) -> Result<(), TotalityError> {
    match s {
        Stmt::While { .. } => {
            if is_hook {
                Err(TotalityError::WhileInLifecycleHook {
                    hook: ctx_name.to_string(),
                })
            } else {
                Err(TotalityError::WhileNotPermitted {
                    method: ctx_name.to_string(),
                })
            }
        }
        Stmt::If {
            then_body,
            else_body,
            ..
        } => {
            for s in then_body {
                check_stmt(s, ctx_name, is_hook)?;
            }
            if let Some(else_body) = else_body {
                for s in else_body {
                    check_stmt(s, ctx_name, is_hook)?;
                }
            }
            Ok(())
        }
        Stmt::Let { value, .. }
        | Stmt::Assign { value, .. }
        | Stmt::CompoundAssign { value, .. } => check_expr_loop_free(value),
        Stmt::Return(maybe) => {
            if let Some(e) = maybe {
                check_expr_loop_free(e)?;
            }
            Ok(())
        }
        Stmt::Require { condition, message } => {
            check_expr_loop_free(condition)?;
            check_expr_loop_free(message)
        }
        Stmt::Emit(e) | Stmt::ExprStmt(e) => check_expr_loop_free(e),
    }
}

/// Expressions are loop-free in the mainline grammar — no while-
/// expression form, no comprehensions. This walker exists so we can
/// extend with future expression-level termination concerns (e.g.
/// recursion-via-method-call detection in V1.5) without adding a
/// second AST walker.
fn check_expr_loop_free(_e: &Expr) -> Result<(), TotalityError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;

    fn parse_or_panic(src: &str) -> Contract {
        parser::parse(src).expect("parse must succeed")
    }

    #[test]
    fn empty_contract_is_total() {
        let src = r#"
            contract Empty {
                state {}
                on_grace() { }
                on_refresh() { }
                on_evaporate() { }
            }
        "#;
        let c = parse_or_panic(src);
        check_total_contract(&c).expect("empty contract must be total");
    }

    #[test]
    fn if_only_contract_is_total() {
        let src = r#"
            contract IfOnly {
                state {
                    n: u64 = 0
                }
                fn bump() {
                    if self.n == 0 {
                        self.n = 1
                    }
                    self.n += 1
                }
                on_grace() { }
                on_refresh() { }
                on_evaporate() { }
            }
        "#;
        let c = parse_or_panic(src);
        check_total_contract(&c).expect("if-only contract must be total");
    }

    #[test]
    fn while_in_method_is_rejected() {
        let src = r#"
            contract Loops {
                state {
                    n: u64 = 0
                }
                fn count_up() {
                    while self.n < 10 {
                        self.n += 1
                    }
                }
                on_grace() { }
                on_refresh() { }
                on_evaporate() { }
            }
        "#;
        let c = parse_or_panic(src);
        let err = check_total_contract(&c).expect_err("while must reject");
        match err {
            TotalityError::WhileNotPermitted { method } => {
                assert_eq!(method, "count_up");
            }
            other => panic!("wrong error variant: {other:?}"),
        }
    }

    #[test]
    fn while_in_lifecycle_hook_is_rejected() {
        let src = r#"
            contract LoopHook {
                state {
                    n: u64 = 0
                }
                on_grace() { }
                on_refresh() { }
                on_evaporate() {
                    while self.n < 10 {
                        self.n += 1
                    }
                }
            }
        "#;
        let c = parse_or_panic(src);
        let err = check_total_contract(&c).expect_err("while in hook must reject");
        match err {
            TotalityError::WhileInLifecycleHook { hook } => {
                assert_eq!(hook, "on_evaporate");
            }
            other => panic!("wrong error variant: {other:?}"),
        }
    }

    #[test]
    fn while_inside_if_is_still_rejected() {
        let src = r#"
            contract Nested {
                state {
                    n: u64 = 0
                }
                fn deep() {
                    if self.n > 0 {
                        while self.n > 0 {
                            self.n -= 1
                        }
                    }
                }
                on_grace() { }
                on_refresh() { }
                on_evaporate() { }
            }
        "#;
        let c = parse_or_panic(src);
        let err = check_total_contract(&c).expect_err("nested while must reject");
        match err {
            TotalityError::WhileNotPermitted { method } => {
                assert_eq!(method, "deep");
            }
            other => panic!("wrong error variant: {other:?}"),
        }
    }
}
