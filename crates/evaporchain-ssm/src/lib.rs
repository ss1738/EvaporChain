//! Singh Strategy Machines (SSM).
//!
//! Per `research/INVENTION_STACK.md` §A5.1:
//!
//! > **Formalism:** Hyland-Ong-Nickau arenas + Abramsky-Jagadeesan-
//! > Malacaria innocent strategies. A contract is an **arena** A; a
//! > program is an **innocent strategy** σ : A. Execution = play
//! > between Proponent (contract code) and Opponent (callers /
//! > adversary / environment). PCF-completeness via HO games.
//!
//! > **Decay synergy — STRUCTURAL:** Game arenas have *justification
//! > pointers* (a move justifies later moves). Energy budget =
//! > bounded depth of the justification tree. **A move is legal iff
//! > its justifier still has positive λ-residual.** Decay is the
//! > *visibility condition* on plays. Unjustified-by-decayed-move ⇒
//! > illegal play ⇒ rejected at strategy level.
//!
//! > **Pitch:** *"the contract is a proof you can win against any
//! > adversary, mechanically."*
//!
//! ## What this crate ships
//!
//! A first-pass model of the V1 SSM fragment:
//!
//! - **Arena** ([`arena`]) — the game tree of moves. Each move has an
//!   owner (Proponent or Opponent), a label, and a parent (the
//!   *enabling* move). The root has no parent.
//! - **Play** ([`play`]) — a sequence of moves. Each move except the
//!   initial Opponent question carries a *justification pointer* to
//!   an earlier move in the same play. Bracketing rules (Hyland-Ong's
//!   well-bracketed condition) are checked: every Proponent answer
//!   must answer the most recent unanswered Opponent question.
//! - **Decay-residual** ([`residual`]) — each move has a `λ_residual`.
//!   A justification pointer to a move with `residual == 0` is
//!   illegal — the doctrine's "decay is the visibility condition."
//! - **Strategy** ([`strategy`]) — a partial map from "play prefix
//!   ending in Opponent move" to "Proponent's response." A strategy
//!   is *innocent* iff it depends only on the P-view (the
//!   Proponent-side projection of the play).
//!
//! V1 ships the well-bracketed, single-threaded, innocent fragment —
//! per A5.1, *"shippable in 2026."* Production extensions (parallel,
//! state, history-sensitive) are future work; the AST + checker
//! shape stay the same.
//!
//! ## Module map
//!
//! - [`arena`] — [`Arena`], [`MoveDecl`], [`Owner`].
//! - [`play`] — [`Play`], [`Move`], well-bracketed checks.
//! - [`residual`] — [`Residual`] tracking; the λ-visibility predicate.
//! - [`strategy`] — [`Strategy`] map + [`is_innocent`] check.

pub mod arena;
pub mod play;
pub mod residual;
pub mod strategy;

pub use arena::{Arena, ArenaError, MoveDecl, MoveLabel, Owner};
pub use play::{Move, Play, PlayError};
pub use residual::{Residual, ResidualError};
pub use strategy::{is_innocent, Strategy, StrategyError};
