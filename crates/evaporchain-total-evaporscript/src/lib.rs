//! Total-Programming EvaporScript — structural totality checker.
//!
//! ## What this crate is
//!
//! A restricted AST and a static checker that statically rejects
//! every non-total program. Every loop carries a syntactic
//! termination witness:
//!
//! - [`Term::BoundedFor`] is a counted loop. The body is statically
//!   forbidden from mutating the loop variable, so iteration is
//!   bounded by `(to - from) / step` at type level.
//! - [`Term::BoundedWhile`] carries a *named ranking variable*. The
//!   loop condition must syntactically guarantee the ranking
//!   variable is positive, and the body must contain a syntactic
//!   strict-decrement of that ranking variable on every control-
//!   flow path. The chain knows the loop terminates because it
//!   cannot encode a `BoundedWhile` whose ranking variable doesn't
//!   strictly decrease.
//!
//! There is **no general `while`**. There is **no general
//!   recursion**. The only recursion construct is structural:
//!   nested `BoundedFor` / `BoundedWhile` whose ranking is itself
//!   total.
//!
//! ## Why this kills the infinite-loop DoS class
//!
//! Gas metering kills *individual* infinite-loop attacks by
//! running the program until it hits the gas limit. That works,
//! but every contract still has the *type* "may not terminate."
//! Total programming makes "this program terminates" a
//! *type-level* fact: the chain knows before execution that any
//! program that passed the checker will halt. The infinite-loop
//! DoS class doesn't get mitigated, it ceases to be expressible.
//!
//! This is the paradigm-grade claim: EvaporChain is the first L1
//! whose contract VM has structural totality at the language
//! level.
//!
//! ## What this crate does NOT do
//!
//! - It does NOT prove correctness. A program can be total and
//!   wrong. (Total Coq programs can be wrong about their spec
//!   too.)
//! - It does NOT verify the ranking decreases by *value*. It
//!   verifies the body contains a syntactic strict-decrement
//!   instruction `x = x - k` for a positive literal `k`. This is
//!   sound but conservative: an arithmetic program that
//!   terminates by some non-syntactic reason will be rejected.
//!   The escape hatch is to express the program in terms of a
//!   ranking variable that the checker can see.
//! - It does NOT lift legacy EvaporScript (`evaporchain-script`)
//!   parser ASTs into `Term`. A future lift can be added; for V1
//!   the chain code that wants total guarantees writes `Term`s
//!   directly.
//!
//! ## Module map
//!
//! - [`term`] — the restricted [`Term`] / [`Expr`] AST.
//! - [`check`] — [`check_total`] + [`TotalError`] + [`Certificate`].

pub mod check;
pub mod term;

pub use check::{check_total, Certificate, TotalError};
pub use term::{BinOp, Expr, Term, UnaryOp};
