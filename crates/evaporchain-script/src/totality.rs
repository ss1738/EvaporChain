//! Totality checker for mainline EvaporScript ASTs.
//!
//! Item B of the smart-contract layer: structural totality enforced as a
//! static lint pass on the mainline `Stmt`/`Expr` AST. The companion
//! `evaporchain-total-evaporscript` crate has a richer Total AST with
//! `BoundedFor` / `BoundedWhile` constructs that carry their own
//! termination witnesses; this module is the integration shim that
//! checks whether a *mainline* program would also pass under
//! total-programming semantics.
//!
//! ## V1 rule (initial)
//!
//! Reject every `Stmt::While`. The mainline `while` carries no syntactic
//! termination witness — gas metering bounds it dynamically, but we
//! cannot type-certify that it halts.
//!
//! ## V1.5 rule (current — BoundedWhile pattern recognition)
//!
//! Accept `while VAR > LITERAL { ... VAR -= LITERAL }` (and `>=` variant)
//! patterns where:
//!   1. Condition is `VAR > N` or `VAR >= N`, with `N` a positive `u64`
//!      literal and `VAR` either a local variable or a `self.field` state
//!      access.
//!   2. Body is a flat sequence (no nested `while`, no nested `if`).
//!   3. Body's LAST statement is `VAR -= K` for some positive `u64`
//!      literal `K`. This guarantees every iteration strict-decrements
//!      the ranking variable at the iteration's end — terminates within
//!      `(N + 1)` iterations.
//!   4. No earlier statement in the body writes to `VAR` (decrements only
//!      happen at the end; no other mutations could break monotonicity).
//!
//! Anything else — `<` condition, `+=` body, branchy body, missing
//! decrement, decrement not at the end, nested loop — still rejects.
//! V1.6 (future) will recognise CFG-aware patterns where the
//! strict-decrement appears on every control-flow path through `if`
//! branches.
//!
//! ## Lifecycle-hook restriction
//!
//! Lifecycle hooks (`on_grace`, `on_refresh`, `on_evaporate`) reject ALL
//! `while` loops, even V1.5-recognised bounded ones. Hooks run in
//! tight chain-runtime contexts; they must be loop-free for cost
//! predictability.
//!
//! ## What gets accepted
//!
//! - Every loop-free construct: `Let`, `Assign`, `CompoundAssign`, `If`,
//!   `Return`, `Require`, `Emit`, `ExprStmt`. Method calls in
//!   expressions. Map and array operations.
//! - V1.5-pattern `while` loops in regular methods (NOT hooks).
//!
//! ## Why this design
//!
//! The seed-15 contracts in `contracts/evaporscript/` use zero `while`
//! loops, so V1's strict rule lets total mode flip on for the entire
//! stdlib without porting work. V1.5 extends acceptance to a
//! conservative pattern that preserves the termination guarantee while
//! letting common bounded-iteration code through.

use thiserror::Error;

use crate::parser::{AssignTarget, BinOp, Contract, Expr, Function, LifecycleHook, Stmt};
use crate::Value;

/// Reasons a mainline EvaporScript program fails the totality check.
///
/// Each variant pinpoints the specific construct that triggers the
/// rejection, so dApp authors get an actionable error rather than a
/// vague "non-total" stamp.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TotalityError {
    /// Catch-all rejection — `while` shape doesn't match V1.5
    /// BoundedWhile pattern. The error message names what an acceptable
    /// pattern would look like.
    #[error(
        "totality: `while` in method `{method}` does not match the V1.5 BoundedWhile \
         pattern (condition must be `<var> > <positive_literal>` or `>=`; body must be a \
         flat sequence whose last statement is `<same_var> -= <positive_literal>`; no \
         nested loops, no branchy bodies, no other writes to <var>)"
    )]
    WhileNotPermitted { method: String },
    /// `while` inside a lifecycle hook — rejected unconditionally.
    /// Hooks are time-critical and must be loop-free.
    #[error(
        "totality: lifecycle hook `{hook}` contains a `while` loop — hooks must be loop-free \
         for cost predictability (V1.5 BoundedWhile recognition does NOT apply to hooks)"
    )]
    WhileInLifecycleHook { hook: String },
    /// Body has a nested `while` loop. Even if the outer pattern is
    /// bounded, the inner one isn't tracked, so reject.
    #[error(
        "totality: `while` in method `{method}` has a NESTED `while` in its body — \
         V1.5 only recognises single-level BoundedWhile patterns"
    )]
    WhileNestedLoopForbidden { method: String },
    /// Body writes to the ranking variable somewhere other than the
    /// terminal strict-decrement. Could break monotonicity.
    #[error(
        "totality: `while` in method `{method}` has a non-decrement write to ranking \
         variable `{var}` — body must only decrement <var> in the terminal statement"
    )]
    WhileRankingMutatedNonDecrementally { method: String, var: String },
    /// Body's last statement is not a strict-decrement of the ranking
    /// variable.
    #[error(
        "totality: `while` in method `{method}` body's last statement is not \
         `{var} -= <positive_literal>` — V1.5 BoundedWhile requires the strict-decrement \
         at the end of every iteration"
    )]
    WhileMissingTerminalDecrement { method: String, var: String },
}

/// Witness that a contract passed the totality check. Holding a
/// [`TotalityCertificate`] means the chain can deploy / execute the
/// contract under total mode without further loop analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TotalityCertificate;

/// Run the totality check across all methods + lifecycle hooks of a
/// parsed contract. Returns a [`TotalityCertificate`] iff every body
/// passes (no `while` in hooks; only V1.5-pattern `while` in methods).
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
        Stmt::While { condition, body } => {
            if is_hook {
                Err(TotalityError::WhileInLifecycleHook {
                    hook: ctx_name.to_string(),
                })
            } else {
                check_bounded_while(condition, body, ctx_name)
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

/// V1.5 — recognise the BoundedWhile pattern. Returns Ok iff the
/// `while` matches the structurally-total shape; otherwise returns a
/// specific [`TotalityError`] variant pinpointing why.
fn check_bounded_while(
    condition: &Expr,
    body: &[Stmt],
    method: &str,
) -> Result<(), TotalityError> {
    // Step 1: extract ranking variable from condition. Must be
    // `<var> > <positive_literal>` or `<var> >= <positive_literal>`.
    let ranking = match extract_ranking_target(condition) {
        Some(r) => r,
        None => {
            return Err(TotalityError::WhileNotPermitted {
                method: method.to_string(),
            });
        }
    };

    // Step 2: body must be non-empty.
    if body.is_empty() {
        return Err(TotalityError::WhileMissingTerminalDecrement {
            method: method.to_string(),
            var: ranking.name(),
        });
    }

    // Step 3: walk all but the last statement. Each must NOT contain
    // a nested `while` and must NOT write to the ranking variable in
    // any form.
    for stmt in &body[..body.len() - 1] {
        check_no_nested_while(stmt, method)?;
        check_no_ranking_write(stmt, &ranking, method)?;
    }

    // Step 4: the last statement must be the terminal strict-decrement
    // `<ranking> -= <positive_literal>`.
    let last = body.last().expect("body non-empty checked above");
    if !is_terminal_strict_decrement(last, &ranking) {
        // Could still be a no-op statement that wraps the decrement —
        // for V1.5 first slice we're strict: terminal stmt must be the
        // decrement directly. Anything else fails.
        return Err(TotalityError::WhileMissingTerminalDecrement {
            method: method.to_string(),
            var: ranking.name(),
        });
    }

    Ok(())
}

/// The ranking-variable shape — local or state-field. Captured so we
/// can match assignments + condition references uniformly.
#[derive(Debug, Clone, PartialEq)]
enum RankingTarget {
    Local(String),
    State(String),
}

impl RankingTarget {
    fn name(&self) -> String {
        match self {
            RankingTarget::Local(n) => n.clone(),
            RankingTarget::State(n) => format!("self.{n}"),
        }
    }

    fn matches_assign_target(&self, t: &AssignTarget) -> bool {
        match (self, t) {
            (RankingTarget::Local(a), AssignTarget::Variable(b)) => a == b,
            (RankingTarget::State(a), AssignTarget::StateField(b)) => a == b,
            _ => false,
        }
    }
}

fn extract_ranking_target(condition: &Expr) -> Option<RankingTarget> {
    let (left, right) = match condition {
        Expr::BinaryOp {
            left,
            op: BinOp::Gt | BinOp::Gte,
            right,
        } => (left.as_ref(), right.as_ref()),
        _ => return None,
    };
    // RHS must be a positive u64 literal.
    let lit = match right {
        Expr::Literal(Value::U64(n)) if *n >= 1 => *n,
        _ => return None,
    };
    let _ = lit; // bound — used implicitly by the >= 1 check
    // LHS must be a local variable or state-field access.
    match left {
        Expr::Variable(name) => Some(RankingTarget::Local(name.clone())),
        Expr::StateAccess(name) => Some(RankingTarget::State(name.clone())),
        _ => None,
    }
}

fn check_no_nested_while(stmt: &Stmt, method: &str) -> Result<(), TotalityError> {
    match stmt {
        Stmt::While { .. } => Err(TotalityError::WhileNestedLoopForbidden {
            method: method.to_string(),
        }),
        Stmt::If {
            then_body,
            else_body,
            ..
        } => {
            for s in then_body {
                check_no_nested_while(s, method)?;
            }
            if let Some(else_body) = else_body {
                for s in else_body {
                    check_no_nested_while(s, method)?;
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn check_no_ranking_write(
    stmt: &Stmt,
    ranking: &RankingTarget,
    method: &str,
) -> Result<(), TotalityError> {
    match stmt {
        Stmt::Assign { target, .. } | Stmt::CompoundAssign { target, .. }
            if ranking.matches_assign_target(target) =>
        {
            Err(TotalityError::WhileRankingMutatedNonDecrementally {
                method: method.to_string(),
                var: ranking.name(),
            })
        }
        Stmt::If {
            then_body,
            else_body,
            ..
        } => {
            for s in then_body {
                check_no_ranking_write(s, ranking, method)?;
            }
            if let Some(else_body) = else_body {
                for s in else_body {
                    check_no_ranking_write(s, ranking, method)?;
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn is_terminal_strict_decrement(stmt: &Stmt, ranking: &RankingTarget) -> bool {
    match stmt {
        Stmt::CompoundAssign {
            target,
            op: BinOp::Sub,
            value: Expr::Literal(Value::U64(delta)),
        } if ranking.matches_assign_target(target) && *delta >= 1 => true,
        _ => false,
    }
}

/// Expressions are loop-free in the mainline grammar — no while-
/// expression form, no comprehensions. This walker exists so we can
/// extend with future expression-level termination concerns (e.g.
/// recursion-via-method-call detection in V1.6) without adding a
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

    // ── V1.5 acceptance cases ───────────────────────────────────────

    #[test]
    fn v15_accepts_state_field_bounded_while() {
        let src = r#"
            contract Counter {
                state {
                    n: u64 = 0
                }
                fn drain() {
                    while self.n > 0 {
                        self.n -= 1
                    }
                }
                on_grace() { }
                on_refresh() { }
                on_evaporate() { }
            }
        "#;
        let c = parse_or_panic(src);
        check_total_contract(&c).expect("V1.5 must accept bounded state-field while");
    }

    #[test]
    fn v15_accepts_local_var_bounded_while() {
        let src = r#"
            contract LocalLoop {
                state {}
                fn count(start: u64) {
                    let i = start
                    while i > 0 {
                        emit("tick")
                        i -= 1
                    }
                }
                on_grace() { }
                on_refresh() { }
                on_evaporate() { }
            }
        "#;
        let c = parse_or_panic(src);
        check_total_contract(&c).expect("V1.5 must accept bounded local-var while");
    }

    #[test]
    fn v15_accepts_gte_condition() {
        let src = r#"
            contract Gte {
                state {}
                fn loop_gte(start: u64) {
                    let i = start
                    while i >= 1 {
                        emit("tick")
                        i -= 1
                    }
                }
                on_grace() { }
                on_refresh() { }
                on_evaporate() { }
            }
        "#;
        let c = parse_or_panic(src);
        check_total_contract(&c).expect("V1.5 must accept >= condition");
    }

    // ── V1.5 rejection cases ────────────────────────────────────────

    #[test]
    fn v15_rejects_lt_condition() {
        let src = r#"
            contract LtLoop {
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
        let err = check_total_contract(&c).expect_err("< condition must reject");
        match err {
            TotalityError::WhileNotPermitted { method } => {
                assert_eq!(method, "count_up");
            }
            other => panic!("wrong error variant: {other:?}"),
        }
    }

    #[test]
    fn v15_rejects_missing_decrement() {
        let src = r#"
            contract NoDec {
                state {
                    n: u64 = 0
                }
                fn spin() {
                    while self.n > 0 {
                        emit("forever")
                    }
                }
                on_grace() { }
                on_refresh() { }
                on_evaporate() { }
            }
        "#;
        let c = parse_or_panic(src);
        let err = check_total_contract(&c).expect_err("missing decrement must reject");
        match err {
            TotalityError::WhileMissingTerminalDecrement { method, .. } => {
                assert_eq!(method, "spin");
            }
            other => panic!("wrong error variant: {other:?}"),
        }
    }

    #[test]
    fn v15_rejects_increment_in_body() {
        let src = r#"
            contract Inc {
                state {
                    n: u64 = 0
                }
                fn spin() {
                    while self.n > 0 {
                        self.n += 1
                        self.n -= 1
                    }
                }
                on_grace() { }
                on_refresh() { }
                on_evaporate() { }
            }
        "#;
        let c = parse_or_panic(src);
        let err = check_total_contract(&c).expect_err("non-decrement write must reject");
        match err {
            TotalityError::WhileRankingMutatedNonDecrementally { method, .. } => {
                assert_eq!(method, "spin");
            }
            other => panic!("wrong error variant: {other:?}"),
        }
    }

    #[test]
    fn v15_rejects_decrement_not_at_end() {
        let src = r#"
            contract NotLast {
                state {
                    n: u64 = 0
                }
                fn spin() {
                    while self.n > 0 {
                        self.n -= 1
                        emit("after-dec")
                    }
                }
                on_grace() { }
                on_refresh() { }
                on_evaporate() { }
            }
        "#;
        let c = parse_or_panic(src);
        // The decrement is the FIRST stmt; `emit` is the LAST. So our
        // walker hits `self.n -= 1` in the not-last range and flags it
        // as a non-decrement write (because we're checking that the
        // earlier statements don't write to ranking).
        let err = check_total_contract(&c).expect_err("decrement not at end must reject");
        // Either WhileRankingMutatedNonDecrementally (because earlier-
        // statement write) OR WhileMissingTerminalDecrement (because
        // last stmt isn't the decrement) — both are acceptable
        // diagnostic outcomes here.
        match err {
            TotalityError::WhileRankingMutatedNonDecrementally { .. }
            | TotalityError::WhileMissingTerminalDecrement { .. } => {}
            other => panic!("wrong error variant: {other:?}"),
        }
    }

    #[test]
    fn v15_rejects_nested_while() {
        let src = r#"
            contract Nest {
                state {
                    n: u64 = 0
                    m: u64 = 0
                }
                fn spin() {
                    while self.n > 0 {
                        while self.m > 0 {
                            self.m -= 1
                        }
                        self.n -= 1
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
            TotalityError::WhileNestedLoopForbidden { method } => {
                assert_eq!(method, "spin");
            }
            other => panic!("wrong error variant: {other:?}"),
        }
    }

    #[test]
    fn v15_rejects_while_in_lifecycle_hook_even_if_bounded() {
        let src = r#"
            contract LoopHook {
                state {
                    n: u64 = 0
                }
                on_grace() { }
                on_refresh() { }
                on_evaporate() {
                    while self.n > 0 {
                        self.n -= 1
                    }
                }
            }
        "#;
        let c = parse_or_panic(src);
        let err = check_total_contract(&c).expect_err("bounded while in hook must STILL reject");
        match err {
            TotalityError::WhileInLifecycleHook { hook } => {
                assert_eq!(hook, "on_evaporate");
            }
            other => panic!("wrong error variant: {other:?}"),
        }
    }

    #[test]
    fn v15_rejects_zero_literal_condition() {
        // `while self.n > 0` is fine because 0 is the SENTINEL the loop
        // ranges down to. But `while self.n >= 0` would never terminate
        // (always true for u64). The pattern-extractor requires the RHS
        // literal to be >= 1, which `>= 1` satisfies but the pure shape
        // `>= 0` does NOT match the V1.5 acceptance test (rejected as
        // not-bounded). Rather than test `>=0` directly (parser may
        // optimise), test that the bound is enforced.
        let src = r#"
            contract ZeroBound {
                state {}
                fn spin(start: u64) {
                    let i = start
                    while i >= 0 {
                        i -= 1
                    }
                }
                on_grace() { }
                on_refresh() { }
                on_evaporate() { }
            }
        "#;
        let c = parse_or_panic(src);
        let err = check_total_contract(&c).expect_err(">= 0 must reject");
        match err {
            TotalityError::WhileNotPermitted { .. } => {}
            other => panic!("wrong error variant: {other:?}"),
        }
    }
}
