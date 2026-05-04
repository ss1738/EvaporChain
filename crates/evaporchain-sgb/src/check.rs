//! Linear-logic usage checker.
//!
//! Given a typing context `Γ = {x_1: T_1, ..., x_n: T_n}` and a
//! usage profile `U`, decide whether `U` is *legal*: every variable's
//! usage count is admissible by its type's outermost exponential
//! flavour.
//!
//! Rules per Girard 1987:
//! - **`Lin(_)`** — usage MUST be exactly 1 (no contraction, no
//!   weakening).
//! - **`Bang(_)` (= `!T`)** — usage may be 0, 1, 2, ... — bang admits
//!   contraction (duplication) and weakening (drop).
//! - **`Whimper(_)` (= `?T`)** — usage may be 0 or 1 — whimper admits
//!   weakening (silent decay-drop) but NOT contraction.
//! - **`Conn(_, _, _)`** — V1 ships connective handling at the
//!   *outermost* level by treating connectives like `Lin` (must be
//!   used exactly once). Real LL would recurse into the connective
//!   structure; that's the v2 elaboration. The crate exposes
//!   [`Connective`] so callers can build that elaboration on top.
//!
//! Failure modes are precise so a checker can produce useful error
//! messages: `LinearMisuse` (Lin used 0 or 2+ times), `WhimperMisuse`
//! (Whimper used 2+ times). Bang has no failure modes — by
//! construction, any non-negative count is legal.

use std::collections::HashMap;
use std::hash::Hash;
use thiserror::Error;

use crate::ty::Type;
use crate::usage::UsageProfile;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CheckError {
    #[error("linear variable {name:?} used {count} times (must be exactly 1)")]
    LinearMisuse { name: String, count: u64 },
    #[error("?-variable {name:?} used {count} times (must be 0 or 1; ? admits weakening but NOT contraction)")]
    WhimperMisuse { name: String, count: u64 },
}

/// Snapshot of (Γ, U) at check time — purely for diagnostics. Carries
/// the per-variable type identifier as a `String` so error messages
/// can echo it.
#[derive(Debug, Clone)]
pub struct ContextSnapshot {
    /// Variable name → its type-tag string (caller-supplied).
    pub typed_vars: HashMap<String, String>,
}

/// Verify `usage` against the typed context.
///
/// `context` maps variable names to their `Type<B>`. For every
/// variable in `context`, compute its usage from `usage` and check
/// the count against the variable's outermost exponential flavour.
///
/// Variables in `usage` but not in `context` are ignored (the parser
/// may have produced a use site for an undeclared variable; that's
/// the parser's bug to surface, not ours).
pub fn check<B: Eq + Hash + Clone>(
    context: &HashMap<String, Type<B>>,
    usage: &UsageProfile,
) -> Result<(), CheckError> {
    for (name, ty) in context {
        let count = usage.count(name);
        match ty {
            Type::Lin(_) | Type::Conn(_, _, _) => {
                // Lin must be exactly 1. V1 also treats top-level
                // connectives as Lin (must be used exactly once).
                if count != 1 {
                    return Err(CheckError::LinearMisuse {
                        name: name.clone(),
                        count,
                    });
                }
            }
            Type::Bang(_) => {
                // Bang accepts any count, including 0 (weakening) and
                // 2+ (contraction). No constraint to violate.
            }
            Type::Whimper(_) => {
                // Whimper accepts 0 (weakening — the chain's λ silently
                // drops) but NOT 2+ (no contraction).
                if count > 1 {
                    return Err(CheckError::WhimperMisuse {
                        name: name.clone(),
                        count,
                    });
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ty::{Connective, Type};

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    enum Base {
        Energy,
        Account,
    }

    fn ctx_with(pairs: &[(&str, Type<Base>)]) -> HashMap<String, Type<Base>> {
        let mut m = HashMap::new();
        for (n, t) in pairs {
            m.insert((*n).to_string(), t.clone());
        }
        m
    }

    // ─── Lin: exactly-once ───────────────────────────────────────────

    #[test]
    fn lin_used_once_is_ok() {
        let ctx = ctx_with(&[("x", Type::Lin(Base::Energy))]);
        let mut u = UsageProfile::new();
        u.add_use("x");
        check(&ctx, &u).unwrap();
    }

    #[test]
    fn lin_used_zero_times_errors() {
        let ctx = ctx_with(&[("x", Type::Lin(Base::Energy))]);
        let u = UsageProfile::new();
        let err = check(&ctx, &u).unwrap_err();
        assert!(matches!(
            err,
            CheckError::LinearMisuse { count: 0, .. }
        ));
    }

    #[test]
    fn lin_used_twice_errors() {
        let ctx = ctx_with(&[("x", Type::Lin(Base::Energy))]);
        let mut u = UsageProfile::new();
        u.add_use("x");
        u.add_use("x");
        let err = check(&ctx, &u).unwrap_err();
        assert!(matches!(
            err,
            CheckError::LinearMisuse { count: 2, .. }
        ));
    }

    // ─── Bang: any-count ─────────────────────────────────────────────

    #[test]
    fn bang_used_zero_times_is_ok() {
        // !T admits weakening — explicit drop is legal.
        let ctx =
            ctx_with(&[("x", Type::bang(Type::Lin(Base::Energy)))]);
        let u = UsageProfile::new();
        check(&ctx, &u).unwrap();
    }

    #[test]
    fn bang_used_many_times_is_ok() {
        let ctx =
            ctx_with(&[("x", Type::bang(Type::Lin(Base::Energy)))]);
        let mut u = UsageProfile::new();
        for _ in 0..7 {
            u.add_use("x");
        }
        check(&ctx, &u).unwrap();
    }

    // ─── Whimper: 0 or 1 ─────────────────────────────────────────────

    #[test]
    fn whimper_used_zero_is_ok() {
        // ?T admits silent decay-drop — chain's λ takes it without notice.
        let ctx =
            ctx_with(&[("x", Type::whimper(Type::Lin(Base::Account)))]);
        let u = UsageProfile::new();
        check(&ctx, &u).unwrap();
    }

    #[test]
    fn whimper_used_once_is_ok() {
        let ctx =
            ctx_with(&[("x", Type::whimper(Type::Lin(Base::Account)))]);
        let mut u = UsageProfile::new();
        u.add_use("x");
        check(&ctx, &u).unwrap();
    }

    #[test]
    fn whimper_used_twice_errors() {
        // The doctrine guarantee: ? admits weakening but not contraction.
        // Duplicating an ephemeral value would let it survive its own
        // decay window — must be rejected at check time.
        let ctx =
            ctx_with(&[("x", Type::whimper(Type::Lin(Base::Account)))]);
        let mut u = UsageProfile::new();
        u.add_use("x");
        u.add_use("x");
        let err = check(&ctx, &u).unwrap_err();
        assert!(matches!(
            err,
            CheckError::WhimperMisuse { count: 2, .. }
        ));
    }

    // ─── Mixed contexts ─────────────────────────────────────────────

    #[test]
    fn mixed_lin_and_bang_pass_independently() {
        let ctx = ctx_with(&[
            ("a", Type::Lin(Base::Energy)),
            ("b", Type::bang(Type::Lin(Base::Account))),
        ]);
        let mut u = UsageProfile::new();
        u.add_use("a"); // Lin: exactly 1 ✓
        u.add_use("b"); // Bang: any count ✓
        u.add_use("b");
        u.add_use("b");
        check(&ctx, &u).unwrap();
    }

    #[test]
    fn the_decay_synergy_claim_lives_as_a_test() {
        // Doctrine: "λ-decay *is* the categorical weakening rule applied
        // automatically." Concretely: a `?` value used 0 times (because
        // the chain's λ silently decayed it before the contract could
        // touch it) MUST type-check, while a `Lin` value used 0 times
        // MUST NOT (ephemerality is a type, not a runtime convention).
        let ephemeral_decayed = ctx_with(&[(
            "ephemeral",
            Type::whimper(Type::Lin(Base::Account)),
        )]);
        let scarce_lost = ctx_with(&[("scarce", Type::Lin(Base::Energy))]);
        let none_used = UsageProfile::new();
        check(&ephemeral_decayed, &none_used).unwrap();
        let err = check(&scarce_lost, &none_used).unwrap_err();
        assert!(matches!(err, CheckError::LinearMisuse { count: 0, .. }));
    }

    // ─── Outer-connective handled as Lin ────────────────────────────

    #[test]
    fn tensor_top_level_is_treated_as_lin() {
        let ctx = ctx_with(&[(
            "pair",
            Type::Conn(
                Connective::Tensor,
                Box::new(Type::Lin(Base::Energy)),
                Box::new(Type::Lin(Base::Account)),
            ),
        )]);
        let mut u = UsageProfile::new();
        u.add_use("pair");
        check(&ctx, &u).unwrap();
        // Used twice ⇒ misuse.
        let mut u2 = UsageProfile::new();
        u2.add_use("pair");
        u2.add_use("pair");
        assert!(matches!(
            check(&ctx, &u2).unwrap_err(),
            CheckError::LinearMisuse { count: 2, .. }
        ));
    }
}
