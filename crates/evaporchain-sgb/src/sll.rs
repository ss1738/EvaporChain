//! Soft Linear Logic (SLL) fragment predicate.
//!
//! Per Lafont 2004: types stratified by `bang_depth ≤ k` admit
//! polynomial-time normalisation in the size of the input. Practically:
//! contracts whose types stay in the SLL fragment are guaranteed
//! polytime termination *by typing*, with no separate gas analysis.
//!
//! V1 ships a coarse approximation: a type is in the SLL fragment iff
//! its **bang-depth ≤ 1** (no nested `!`). This is the first
//! Lafont-stratification level. Future versions can extend to higher
//! strata; for now, depth-1 is the cheap, decidable, useful filter.
//!
//! `?` is unconstrained — whimper is decay-eligible by definition;
//! polytime bounds aren't threatened by silently-dropped values.

use std::hash::Hash;

use crate::ty::Type;

/// Why a type was rejected from the SLL fragment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SllReason {
    /// Two `!` exponentials nested (`!!T`). SLL bans bang-depth > 1.
    NestedBang,
}

/// True iff `t` is in the V1-approximation SLL fragment (bang-depth ≤ 1).
pub fn is_sll_fragment<B: Eq + Hash + Clone>(t: &Type<B>) -> Result<(), SllReason> {
    bang_depth(t, false)
}

/// `under_bang` tracks whether we're currently inside a `!` constructor.
/// If we encounter another `!` while already under one, we've found
/// `!!T` — reject.
fn bang_depth<B: Eq + Hash + Clone>(t: &Type<B>, under_bang: bool) -> Result<(), SllReason> {
    match t {
        Type::Lin(_) => Ok(()),
        Type::Bang(inner) => {
            if under_bang {
                Err(SllReason::NestedBang)
            } else {
                bang_depth(inner, true)
            }
        }
        Type::Whimper(inner) => {
            // ? does NOT change the bang-depth count for SLL purposes.
            // (Lafont's stratification is about contraction-on-bang;
            // weakening-on-whimper is free.)
            bang_depth(inner, under_bang)
        }
        Type::Conn(_, l, r) => {
            bang_depth(l, under_bang)?;
            bang_depth(r, under_bang)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    enum Base {
        Energy,
        Account,
    }

    #[test]
    fn lin_is_sll() {
        let t: Type<Base> = Type::Lin(Base::Energy);
        is_sll_fragment(&t).unwrap();
    }

    #[test]
    fn bang_lin_is_sll() {
        let t: Type<Base> = Type::bang(Type::Lin(Base::Energy));
        is_sll_fragment(&t).unwrap();
    }

    #[test]
    fn whimper_lin_is_sll() {
        let t: Type<Base> = Type::whimper(Type::Lin(Base::Account));
        is_sll_fragment(&t).unwrap();
    }

    #[test]
    fn whimper_bang_lin_is_sll() {
        // ?(!T) — only one bang, sandwiched by ?. SLL accepts.
        let t: Type<Base> = Type::whimper(Type::bang(Type::Lin(Base::Energy)));
        is_sll_fragment(&t).unwrap();
    }

    #[test]
    fn nested_bang_is_rejected() {
        // !!T — SLL stratification says polytime bound goes superlinear.
        let t: Type<Base> =
            Type::bang(Type::bang(Type::Lin(Base::Energy)));
        let err = is_sll_fragment(&t).unwrap_err();
        assert_eq!(err, SllReason::NestedBang);
    }

    #[test]
    fn nested_bang_via_connective_is_rejected() {
        // L = !T, R = !T'; the connective itself isn't a bang, but each
        // arm has a bang. That's fine. But !(L ⊗ R) where L is itself a
        // bang IS rejected.
        let inner_bang: Type<Base> = Type::bang(Type::Lin(Base::Energy));
        let outer_bang: Type<Base> = Type::bang(Type::tensor(
            inner_bang,
            Type::Lin(Base::Account),
        ));
        let err = is_sll_fragment(&outer_bang).unwrap_err();
        assert_eq!(err, SllReason::NestedBang);
    }

    #[test]
    fn parallel_bangs_in_connective_are_sll() {
        // !T ⊗ !U — two bangs side by side at depth 1; neither is nested.
        let t: Type<Base> = Type::tensor(
            Type::bang(Type::Lin(Base::Energy)),
            Type::bang(Type::Lin(Base::Account)),
        );
        is_sll_fragment(&t).unwrap();
    }

    #[test]
    fn whimper_does_not_count_toward_bang_depth() {
        // ?(?(?T)) — nested whimpers, no bangs. Fine.
        let t: Type<Base> = Type::whimper(Type::whimper(Type::whimper(Type::Lin(
            Base::Energy,
        ))));
        is_sll_fragment(&t).unwrap();
    }
}
