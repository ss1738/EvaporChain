//! V1 totality gate for EvaporScript deploys.
//!
//! When the chain runs with governance flag `script_vm_mode = "total"`,
//! every deployed contract must be *total*: its execution terminates
//! regardless of input or gas budget. The V1 rule that certifies this is
//! purely syntactic — **a contract is total iff it contains no `while`
//! loop** anywhere in its functions or lifecycle hooks. With `while`
//! removed, the only control flow left is straight-line code, bounded
//! branching (`if`/`else`), and `return`, so every method runs in time
//! linear in its own source size.
//!
//! This is deliberately stricter and simpler than the bounded-while
//! termination calculus in the `evaporchain-total-evaporscript` crate
//! (which certifies `BoundedWhile` loops via a syntactic ranking witness
//! over its own `Term` IR). Admitting witnessed bounded loops into the
//! production deploy gate is a V2 path; V1 is no-`while`.
//!
//! The default `script_vm_mode` (`permissive` / unset) skips this gate
//! entirely — gas metering bounds runtime there. The gate runs only when
//! an operator flips the flag to `total`.

use crate::parser::{Contract, LifecycleHook, Stmt};
use crate::ScriptError;

/// Certify that `contract` is total under the V1 rule (no `while`).
///
/// Returns `Err(ScriptError::Compile)` naming the first offending
/// function or lifecycle hook if any `while` loop is present — including
/// loops nested inside `if`/`else`. The seed stdlib is total-clean, so
/// this passes for every shipped contract.
pub fn check_total_contract(contract: &Contract) -> Result<(), ScriptError> {
    for f in &contract.functions {
        if block_has_while(&f.body) {
            return Err(reject(&contract.name, &format!("function `{}`", f.name)));
        }
    }
    for hook in &contract.lifecycle_hooks {
        let (name, body) = match hook {
            LifecycleHook::OnEvaporate(b) => ("on_evaporate", b),
            LifecycleHook::OnGrace(b) => ("on_grace", b),
            LifecycleHook::OnRefresh(b) => ("on_refresh", b),
        };
        if block_has_while(body) {
            return Err(reject(&contract.name, &format!("lifecycle hook `{name}`")));
        }
    }
    Ok(())
}

fn reject(contract: &str, location: &str) -> ScriptError {
    ScriptError::Compile(format!(
        "contract `{contract}` is not total: `while` loop in {location} is forbidden \
         under script_vm_mode=total (V1 totality rule: no unbounded loops)"
    ))
}

/// True if any statement in the block is, or syntactically encloses, a
/// `while` loop. Recurses into `if`/`else` bodies; every other statement
/// is a leaf that cannot enclose a nested statement block.
fn block_has_while(stmts: &[Stmt]) -> bool {
    stmts.iter().any(stmt_has_while)
}

fn stmt_has_while(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::While { .. } => true,
        Stmt::If {
            then_body,
            else_body,
            ..
        } => block_has_while(then_body) || else_body.as_deref().is_some_and(block_has_while),
        // Leaf statements — no nested statement block, so no `while`.
        // Exhaustive (no wildcard) on purpose: a future block-bearing
        // statement variant must be classified here explicitly.
        Stmt::Let { .. }
        | Stmt::Assign { .. }
        | Stmt::CompoundAssign { .. }
        | Stmt::Return(_)
        | Stmt::Require { .. }
        | Stmt::Emit(_)
        | Stmt::ExprStmt(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn check(src: &str) -> Result<(), ScriptError> {
        check_total_contract(&parse(src).expect("fixture must parse"))
    }

    #[test]
    fn accepts_total_clean_contract() {
        let src = r#"
contract Counter {
    state { count: u64 = 0 }
    fn bump() { self.count = self.count + 1 }
    fn maybe(x: u64) {
        if x > 0 { self.count = self.count + x } else { self.count = 0 }
    }
}
"#;
        assert!(check(src).is_ok());
    }

    #[test]
    fn rejects_top_level_while() {
        let src = r#"
contract Looper {
    state { n: u64 = 0 }
    fn climb() { while self.n < 5 { self.n += 1 } }
}
"#;
        let err = check(src).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("not total"), "got: {msg}");
        assert!(msg.contains("climb"), "should name the function: {msg}");
    }

    #[test]
    fn rejects_while_nested_in_if() {
        let src = r#"
contract Nested {
    state { n: u64 = 0 }
    fn go(x: u64) {
        if x > 0 {
            while self.n < x { self.n += 1 }
        }
    }
}
"#;
        assert!(check(src).is_err());
    }

    #[test]
    fn rejects_while_nested_in_else() {
        let src = r#"
contract NestedElse {
    state { n: u64 = 0 }
    fn go(x: u64) {
        if x > 0 { self.n = 1 } else { while self.n < 3 { self.n += 1 } }
    }
}
"#;
        assert!(check(src).is_err());
    }

    #[test]
    fn rejects_while_in_lifecycle_hook() {
        let src = r#"
contract HookLoop {
    state { n: u64 = 0 }
    fn noop() { self.n = 0 }
    on_evaporate() { while self.n < 2 { self.n += 1 } }
    on_grace() { emit("low") }
    on_refresh() { emit("up") }
}
"#;
        let err = check(src).unwrap_err();
        assert!(
            format!("{err}").contains("on_evaporate"),
            "should name the hook: {err}"
        );
    }
}
