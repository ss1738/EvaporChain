//! λ-residual tracking — the doctrine's "decay is the visibility
//! condition on plays."
//!
//! Every move in a play has an attached `Residual` (an `Energy`
//! counter). When a later move is justified by an earlier one, the
//! justifier's residual must still be positive. A residual of zero
//! means the move has *decayed* and can no longer enable later
//! moves — its "visibility window" has closed.
//!
//! The chain's λ-decay is what drains residuals over time. This
//! module ships the bookkeeping; the rate model lives in higher
//! layers (the same pattern as Singh-Sabi: this crate exposes the
//! data structure, the chain supplies the rate).

use evaporchain_types::Energy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

use crate::arena::MoveId;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ResidualError {
    #[error("move {move_id:?} has zero residual — its justification window has closed")]
    DecayedJustifier { move_id: MoveId },
    #[error("move {move_id:?} has no residual recorded")]
    Unknown { move_id: MoveId },
}

/// Per-move residual energy. Wrapped so callers can extend with
/// metadata (drain rate, last-touched epoch, etc.) without breaking
/// the API.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Residual {
    by_move: HashMap<MoveId, Energy>,
}

impl Residual {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a move's initial residual (typically at the time the move
    /// is appended to the play).
    pub fn set(&mut self, move_id: MoveId, e: Energy) {
        self.by_move.insert(move_id, e);
    }

    pub fn get(&self, move_id: MoveId) -> Option<Energy> {
        self.by_move.get(&move_id).copied()
    }

    /// Drain a move's residual by `delta` (saturating at 0).
    pub fn drain(&mut self, move_id: MoveId, delta: Energy) {
        if let Some(r) = self.by_move.get_mut(&move_id) {
            *r = r.saturating_sub(delta);
        }
    }

    /// **The λ-visibility predicate.** A justification is legal iff
    /// the justifier's residual is strictly positive.
    pub fn is_visible(&self, move_id: MoveId) -> Result<bool, ResidualError> {
        let e = self
            .get(move_id)
            .ok_or(ResidualError::Unknown { move_id })?;
        Ok(e > 0)
    }

    /// Convenience: check visibility with an `Err` if the justifier
    /// has decayed. Useful at the call site where the play wants to
    /// short-circuit.
    pub fn require_visible(&self, move_id: MoveId) -> Result<(), ResidualError> {
        match self.is_visible(move_id)? {
            true => Ok(()),
            false => Err(ResidualError::DecayedJustifier { move_id }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> MoveId {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    #[test]
    fn fresh_move_is_visible() {
        let mut r = Residual::new();
        r.set(id(1), 100);
        assert!(r.is_visible(id(1)).unwrap());
    }

    #[test]
    fn drained_move_becomes_invisible() {
        let mut r = Residual::new();
        r.set(id(1), 50);
        r.drain(id(1), 50);
        assert!(!r.is_visible(id(1)).unwrap());
    }

    #[test]
    fn require_visible_errors_on_decayed_justifier() {
        let mut r = Residual::new();
        r.set(id(1), 0);
        let err = r.require_visible(id(1)).unwrap_err();
        assert!(matches!(err, ResidualError::DecayedJustifier { .. }));
    }

    #[test]
    fn unknown_move_errors() {
        let r = Residual::new();
        let err = r.is_visible(id(99)).unwrap_err();
        assert!(matches!(err, ResidualError::Unknown { .. }));
    }

    #[test]
    fn drain_saturates_at_zero() {
        let mut r = Residual::new();
        r.set(id(1), 10);
        r.drain(id(1), 1_000_000);
        assert_eq!(r.get(id(1)), Some(0));
        assert!(!r.is_visible(id(1)).unwrap());
    }

    #[test]
    fn unjustified_by_decayed_move_is_an_illegal_play() {
        // Doctrine claim: "Unjustified-by-decayed-move ⇒ illegal play
        // ⇒ rejected at strategy level." Here the strategy code can
        // call `require_visible` before allowing a move to be appended.
        let mut r = Residual::new();
        r.set(id(1), 5);
        // Time passes; chain's λ drains all of move 1's residual.
        r.drain(id(1), 5);
        // A later P-move that wants to be justified by id(1) is now
        // illegal — `require_visible` returns DecayedJustifier.
        let err = r.require_visible(id(1)).unwrap_err();
        assert!(matches!(err, ResidualError::DecayedJustifier { .. }));
    }

    // Note: Residual uses HashMap<[u8;32], Energy>; JSON can't
    // serialize byte-array map keys directly. A production serde
    // impl would hex-encode keys; V1 leaves the in-memory shape
    // simple. Bincode round-trips work; the round-trip-via-JSON
    // case would require a dedicated wrapper.
}
