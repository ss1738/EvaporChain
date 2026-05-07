//! Linear-logic type AST.
//!
//! `Type<B>` is parametric over an opaque "base type" — the chain's
//! domain types (Account, Energy, ContractId, ...) plug in here as
//! the leaves. The connectives are domain-agnostic; the linear
//! checker only needs the AST shape.

use serde::{Deserialize, Serialize};

/// Three connectives shipped in V1. Multiplicative `Tensor`, additive
/// `With` and `Plus`. Linear logic's full set is larger (par, top,
/// bottom, etc.) — V1 ships the practical subset that covers smart-
/// contract use cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Connective {
    /// `L ⊗ R` — both available; no choice.
    Tensor,
    /// `L & R` — pick *one* side at use site.
    With,
    /// `L ⊕ R` — runtime hands you one side; you don't choose.
    Plus,
}

/// The type AST.
///
/// `B` is an opaque base type — the chain's domain types plug in here.
/// `Hash` is required so types can key into proof-cache maps; we
/// require `B: Eq + Hash`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Type<B>
where
    B: Eq + std::hash::Hash,
{
    /// `Lin(B)` — must be used exactly once.
    Lin(B),
    /// `!B` — may be duplicated; decay-immune.
    Bang(Box<Type<B>>),
    /// `?B` — may be silently dropped; decay-eligible.
    Whimper(Box<Type<B>>),
    /// Binary connective.
    Conn(Connective, Box<Type<B>>, Box<Type<B>>),
}

impl<B: Eq + std::hash::Hash + Clone> Type<B> {
    /// Convenience: `!T` constructor.
    pub fn bang(inner: Type<B>) -> Self {
        Type::Bang(Box::new(inner))
    }

    /// Convenience: `?T` constructor.
    pub fn whimper(inner: Type<B>) -> Self {
        Type::Whimper(Box::new(inner))
    }

    /// Convenience: `L ⊗ R`.
    pub fn tensor(l: Type<B>, r: Type<B>) -> Self {
        Type::Conn(Connective::Tensor, Box::new(l), Box::new(r))
    }

    /// Convenience: `L & R`.
    pub fn with(l: Type<B>, r: Type<B>) -> Self {
        Type::Conn(Connective::With, Box::new(l), Box::new(r))
    }

    /// Convenience: `L ⊕ R`.
    pub fn plus(l: Type<B>, r: Type<B>) -> Self {
        Type::Conn(Connective::Plus, Box::new(l), Box::new(r))
    }

    /// Is the *outermost* exponential a `Bang`?
    pub fn is_bang(&self) -> bool {
        matches!(self, Type::Bang(_))
    }

    /// Is the *outermost* exponential a `Whimper`?
    pub fn is_whimper(&self) -> bool {
        matches!(self, Type::Whimper(_))
    }

    /// Linear depth: `Lin = 0; !T = 1 + depth(T); ?T = 1 + depth(T);
    /// Conn(_, L, R) = 1 + max(depth(L), depth(R))`. Used by the SLL
    /// fragment check for a coarse polytime bound.
    pub fn depth(&self) -> u32 {
        match self {
            Type::Lin(_) => 0,
            Type::Bang(t) | Type::Whimper(t) => 1 + t.depth(),
            Type::Conn(_, l, r) => 1 + l.depth().max(r.depth()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny base-type domain so tests can construct types.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    enum Base {
        Energy,
        Account,
    }

    #[test]
    fn lin_is_neither_bang_nor_whimper() {
        let t: Type<Base> = Type::Lin(Base::Energy);
        assert!(!t.is_bang());
        assert!(!t.is_whimper());
        assert_eq!(t.depth(), 0);
    }

    #[test]
    fn bang_and_whimper_classify() {
        let b: Type<Base> = Type::bang(Type::Lin(Base::Energy));
        let w: Type<Base> = Type::whimper(Type::Lin(Base::Account));
        assert!(b.is_bang());
        assert!(!b.is_whimper());
        assert!(w.is_whimper());
        assert!(!w.is_bang());
        assert_eq!(b.depth(), 1);
        assert_eq!(w.depth(), 1);
    }

    #[test]
    fn tensor_with_plus_have_depth_one_plus_max() {
        let inner: Type<Base> = Type::bang(Type::Lin(Base::Energy));
        let outer: Type<Base> = Type::tensor(inner.clone(), Type::Lin(Base::Account));
        // depth(inner) = 1, depth(Lin) = 0 ⇒ depth(tensor) = 2
        assert_eq!(outer.depth(), 2);
    }

    #[test]
    fn round_trip_serde() {
        let t: Type<Base> = Type::tensor(
            Type::bang(Type::Lin(Base::Energy)),
            Type::whimper(Type::Lin(Base::Account)),
        );
        let s = serde_json::to_string(&t).unwrap();
        let back: Type<Base> = serde_json::from_str(&s).unwrap();
        assert_eq!(t, back);
    }
}
