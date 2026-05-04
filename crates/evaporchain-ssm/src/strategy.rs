//! Strategy — Proponent's response function.
//!
//! A `Strategy` is a partial map from "play prefix ending in an
//! Opponent move" to "Proponent's next move." V1 represents the
//! prefix by its **P-view** (the Proponent-side projection of the
//! play) — that's the *innocence* condition: the strategy depends
//! only on what Proponent has seen of its own thread, not on the
//! full play history.
//!
//! Practically, the P-view is computed as: take the last move; if
//! it's an O-move, include it; then add its justifier; recurse.
//! For the well-bracketed innocent fragment this is just the chain
//! of O-moves the P will respond to plus their justifying P-moves.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

use crate::arena::{Arena, MoveId, Owner};
use crate::play::{Move, Play};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StrategyError {
    #[error("strategy is non-innocent: same P-view maps to two different responses")]
    NonInnocent,
}

/// Strategy as a `(p_view → response)` map. The p-view is
/// represented as the sequence of move ids encountered when walking
/// back from the last move to the root via the P-view rule.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Strategy {
    map: HashMap<Vec<MoveId>, MoveId>,
}

impl Strategy {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a (view → response) mapping. Returns the old response
    /// if any — caller should `is_innocent`-check after a batch of
    /// inserts.
    pub fn record(&mut self, p_view: Vec<MoveId>, response: MoveId) -> Option<MoveId> {
        self.map.insert(p_view, response)
    }

    pub fn lookup(&self, p_view: &[MoveId]) -> Option<MoveId> {
        self.map.get(p_view).copied()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Compute the P-view of a play.
///
/// The P-view is the "Proponent's view" of the play: the sequence of
/// moves Proponent sees when responding to the most recent O-move.
/// Algorithm (well-bracketed fragment):
///
/// 1. Start at the last move. If it's not an Opponent move, the
///    P-view is undefined (we only ask P to respond at O-positions).
/// 2. Include it.
/// 3. Walk to its justifier. Include the justifier.
/// 4. From there, walk to the *immediately preceding O-move on the
///    same thread* via the justifier chain, and so on.
///
/// V1 simplification: take the chain of `(last_move, last_move's
/// justifier, that move's justifier, ...)` until reaching the root.
/// This is the H-O P-view in the well-bracketed innocent fragment.
pub fn p_view(arena: &Arena, play: &Play) -> Option<Vec<MoveId>> {
    let last = play.moves.last()?;
    let last_decl = arena.lookup(last.move_id)?;
    if last_decl.owner != Owner::Opponent {
        return None;
    }

    let mut view = vec![last.move_id];
    let mut cursor = last.justifier;
    while let Some(jid) = cursor {
        view.push(jid);
        // Find that move in the play and follow its justifier.
        let next = play.moves.iter().find(|m| m.move_id == jid)?;
        cursor = next.justifier;
    }
    Some(view)
}

/// Verify a strategy is innocent over the supplied set of (play,
/// expected_response) pairs. Two pairs with the same P-view but
/// different responses break innocence.
pub fn is_innocent(
    arena: &Arena,
    cases: &[(Play, MoveId)],
) -> Result<(), StrategyError> {
    let mut seen: HashMap<Vec<MoveId>, MoveId> = HashMap::new();
    for (play, response) in cases {
        if let Some(view) = p_view(arena, play) {
            match seen.get(&view) {
                Some(prior) if prior != response => return Err(StrategyError::NonInnocent),
                _ => {
                    seen.insert(view, *response);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::{MoveDecl, MoveLabel};

    fn id(b: u8) -> MoveId {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    fn arena_simple() -> Arena {
        Arena::build(vec![
            MoveDecl { id: id(1), owner: Owner::Opponent, label: MoveLabel::Question, parent: None },
            MoveDecl { id: id(2), owner: Owner::Proponent, label: MoveLabel::Answer, parent: Some(id(1)) },
            MoveDecl { id: id(3), owner: Owner::Proponent, label: MoveLabel::Question, parent: Some(id(1)) },
            MoveDecl { id: id(4), owner: Owner::Opponent, label: MoveLabel::Answer, parent: Some(id(3)) },
        ])
        .unwrap()
    }

    #[test]
    fn p_view_at_root_is_just_root() {
        let arena = arena_simple();
        let mut play = Play::new();
        play.append(&arena, Move { move_id: id(1), justifier: None }).unwrap();
        let view = p_view(&arena, &play).unwrap();
        assert_eq!(view, vec![id(1)]);
    }

    #[test]
    fn p_view_undefined_at_p_position() {
        // After P moves, there's no P-view to compute (P doesn't owe
        // a response when it's its own turn).
        let arena = arena_simple();
        let mut play = Play::new();
        play.append(&arena, Move { move_id: id(1), justifier: None }).unwrap();
        play.append(&arena, Move { move_id: id(3), justifier: Some(id(1)) }).unwrap();
        assert!(p_view(&arena, &play).is_none());
    }

    #[test]
    fn p_view_walks_justifier_chain() {
        let arena = arena_simple();
        let mut play = Play::new();
        play.append(&arena, Move { move_id: id(1), justifier: None }).unwrap();
        play.append(&arena, Move { move_id: id(3), justifier: Some(id(1)) }).unwrap();
        play.append(&arena, Move { move_id: id(4), justifier: Some(id(3)) }).unwrap();
        let view = p_view(&arena, &play).unwrap();
        // P-view: [4, 3, 1] — the latest O-move and its chain back to root.
        assert_eq!(view, vec![id(4), id(3), id(1)]);
    }

    #[test]
    fn strategy_record_and_lookup() {
        let mut s = Strategy::new();
        s.record(vec![id(1)], id(2));
        assert_eq!(s.lookup(&[id(1)]), Some(id(2)));
        assert_eq!(s.lookup(&[id(99)]), None);
    }

    #[test]
    fn innocent_strategy_passes_check() {
        let arena = arena_simple();
        // Two plays with the same P-view at the root: both must map
        // to the same response.
        let mut play_a = Play::new();
        play_a.append(&arena, Move { move_id: id(1), justifier: None }).unwrap();
        let mut play_b = Play::new();
        play_b.append(&arena, Move { move_id: id(1), justifier: None }).unwrap();
        is_innocent(&arena, &[(play_a, id(2)), (play_b, id(2))]).unwrap();
    }

    #[test]
    fn non_innocent_strategy_caught() {
        let arena = arena_simple();
        // Same P-view → different responses ⇒ non-innocent.
        let mut play_a = Play::new();
        play_a.append(&arena, Move { move_id: id(1), justifier: None }).unwrap();
        let mut play_b = Play::new();
        play_b.append(&arena, Move { move_id: id(1), justifier: None }).unwrap();
        let err = is_innocent(&arena, &[(play_a, id(2)), (play_b, id(3))]).unwrap_err();
        assert_eq!(err, StrategyError::NonInnocent);
    }

    // Note: Strategy uses HashMap<Vec<MoveId>, MoveId>; JSON can't
    // serialize Vec-of-byte-array map keys directly. Same caveat as
    // `Residual`: bincode works, JSON would need a hex-string wrapper.
}
