//! Singh-Girard Bang-Whimper Types (SGB).
//!
//! Per `research/INVENTION_STACK.md` §A5.1:
//!
//! > **Formalism:** Girard 1987 linear logic. `!T` admits duplication
//! > (decay-immune); `?T` admits silent loss (decay-eligible); de
//! > Morgan duals. Light Linear Logic (Girard 1998) and Soft Linear
//! > Logic (Lafont 2004) give *polytime* fragments.
//!
//! > **Decay synergy — STRUCTURAL:** the duality is exact. λ-decay
//! > *is* the categorical weakening rule applied automatically.
//! > Existing affine/linear chains (Move, Sui) only use `!`'s shadow;
//! > `?` has never been exposed as a contract type because no chain
//! > had a principled "decay" primitive to pair with it.
//!
//! > **Pitch:** *"first system where ephemerality is a type, not a
//! > runtime convention."* Type theorists have been waiting 30 years
//! > for industrial use of `?`.
//!
//! ## What this crate ships
//!
//! A first-pass type system + checker for linear-logic types
//! parameterised over an opaque base-type domain `B`. Three exponent
//! flavours:
//!
//! - **`Lin(B)`** — pure linear: must be used **exactly once**.
//! - **`Bang(B)`** = `!B` — the *bang*: may be **duplicated** (used
//!   any number of times) but *cannot* be silently dropped (decay
//!   does not affect it; explicit `derelict` is the only discard).
//! - **`Whimper(B)`** = `?B` — the *whimper*: may be **silently
//!   dropped** (and decay can take it freely) but *cannot* be
//!   duplicated (no contraction).
//!
//! These are the four structural rules of linear logic split into the
//! two exponentials per Girard 1987. The decay primitive maps cleanly
//! onto `?` — the categorical weakening rule applied automatically by
//! the chain's λ.
//!
//! Plus the three connectives:
//!
//! - **`Tensor(L, R)`** — `L ⊗ R` — both available; no choice.
//! - **`With(L, R)`** — `L & R` — additive conjunction; pick *either*
//!   side (additive duplication).
//! - **`Plus(L, R)`** — `L ⊕ R` — additive disjunction; you get
//!   *one* of them; the runtime decides which.
//!
//! ## What gets you (mechanically)
//!
//! 1. **No silent leak** of ephemeral state. A `?` value can decay
//!    freely; a `!` value never can (compile-time guarantee).
//! 2. **Polytime gas bounds for free** under the SLL fragment
//!    (Lafont 2004): contracts whose types stay in the SLL fragment
//!    are guaranteed polynomial-time termination. The *type
//!    discipline* is the gas analysis. Kills whole DoS classes.
//! 3. **Compositional reasoning** about which values can decay and
//!    which cannot — the type prints the answer; no runtime audit
//!    needed.
//!
//! ## Module map
//!
//! - [`ty`] — [`Type<B>`], [`Connective`]; the type AST.
//! - [`usage`] — [`UsageProfile`]; how many times each variable was
//!   used; the linear-checker's bookkeeping.
//! - [`check`] — [`check`]: verify a `UsageProfile` is *legal* given
//!   the variable's type (Lin = exactly 1, Bang = ≥ 0, Whimper = 0
//!   or 1).
//! - [`sll`] — [`is_sll_fragment`]: predicate identifying types that
//!   live in the polytime Soft Linear Logic fragment.

pub mod check;
pub mod sll;
pub mod ty;
pub mod usage;

pub use check::{check, CheckError, ContextSnapshot};
pub use sll::{is_sll_fragment, SllReason};
pub use ty::{Connective, Type};
pub use usage::{UsageProfile, UsageProfileError};
