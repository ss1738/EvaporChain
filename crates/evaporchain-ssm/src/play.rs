//! Play — a sequence of *justified moves* drawn from an arena.
//!
//! Every move except the initial Opponent question carries a
//! justification pointer to an earlier move in the same play. The
//! enabling-relation in the arena defines *which* moves can be
//! justified by which: a move can only be justified by a move whose
//! arena-parent equals the playing move's arena-id-parent.
//!
//! Well-bracketed condition (Hyland-Ong): every Answer must answer
//! the most recent unanswered Question of the *opposite* owner —
//! i.e. plays look like balanced parentheses if you tag Q open / A
//! close.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::arena::{Arena, MoveId, Owner};

/// A single move in a play. `move_id` references an arena
/// declaration; `justifier` references the prior move in the play
/// (or `None` for the initial Opponent question).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Move {
    pub move_id: MoveId,
    pub justifier: Option<MoveId>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PlayError {
    #[error("move {move_id:?} not declared in the arena")]
    UnknownMove { move_id: MoveId },
    #[error("first move must be the arena root with justifier=None")]
    BadInitialMove,
    #[error("move {move_id:?} requires a justifier (only the root may have None)")]
    MissingJustifier { move_id: MoveId },
    #[error("justifier {justifier:?} not yet played in this play")]
    JustifierNotInPlay { justifier: MoveId },
    #[error(
        "justification mismatch: arena says {move_id:?}'s parent is {expected:?}, play points to {actual:?}"
    )]
    JustifierMismatch {
        move_id: MoveId,
        expected: Option<MoveId>,
        actual: MoveId,
    },
    #[error("well-bracketing violation: Answer attempted with no open Question to close")]
    BracketingNoOpenQuestion,
    #[error(
        "well-bracketing violation: Answer must close the most recent unanswered Question of the opposite owner"
    )]
    BracketingWrongQuestion,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Play {
    pub moves: Vec<Move>,
}

impl Play {
    pub fn new() -> Self {
        Self { moves: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.moves.len()
    }

    pub fn is_empty(&self) -> bool {
        self.moves.is_empty()
    }

    /// Append a move, validating against the arena.
    pub fn append(&mut self, arena: &Arena, mv: Move) -> Result<(), PlayError> {
        let decl = arena.lookup(mv.move_id).ok_or(PlayError::UnknownMove {
            move_id: mv.move_id,
        })?;

        if self.moves.is_empty() {
            // Initial move: must be arena root with justifier=None.
            if mv.justifier.is_some() || mv.move_id != arena.root {
                return Err(PlayError::BadInitialMove);
            }
            self.moves.push(mv);
            return Ok(());
        }

        let justifier_id = mv.justifier.ok_or(PlayError::MissingJustifier {
            move_id: mv.move_id,
        })?;

        // Justifier must already be in the play.
        if !self.moves.iter().any(|m| m.move_id == justifier_id) {
            return Err(PlayError::JustifierNotInPlay {
                justifier: justifier_id,
            });
        }

        // Justifier must match the arena's parent for this move.
        if decl.parent != Some(justifier_id) {
            return Err(PlayError::JustifierMismatch {
                move_id: mv.move_id,
                expected: decl.parent,
                actual: justifier_id,
            });
        }

        // Well-bracketed condition: if this is an Answer, it must
        // close the most recent unanswered Question of the opposite
        // owner.
        if decl.label == crate::arena::MoveLabel::Answer {
            let pending = self.most_recent_unanswered_question(arena, decl.owner.opposite());
            match pending {
                None => return Err(PlayError::BracketingNoOpenQuestion),
                Some(q) if q == justifier_id => { /* OK — this answer closes that Q */ }
                Some(_) => return Err(PlayError::BracketingWrongQuestion),
            }
        }

        self.moves.push(mv);
        Ok(())
    }

    /// Iterate the play looking for the most recent Question of
    /// `target_owner` that has not yet been answered. Returns its
    /// `move_id`. Used by the well-bracketing check.
    fn most_recent_unanswered_question(
        &self,
        arena: &Arena,
        target_owner: Owner,
    ) -> Option<MoveId> {
        // Build the set of answered-question ids.
        let mut answered: std::collections::HashSet<MoveId> = std::collections::HashSet::new();
        for m in &self.moves {
            if let Some(decl) = arena.lookup(m.move_id) {
                if decl.label == crate::arena::MoveLabel::Answer {
                    if let Some(jid) = m.justifier {
                        answered.insert(jid);
                    }
                }
            }
        }
        // Walk backward, return the first matching Q that isn't answered.
        for m in self.moves.iter().rev() {
            let decl = arena.lookup(m.move_id)?;
            if decl.owner == target_owner
                && decl.label == crate::arena::MoveLabel::Question
                && !answered.contains(&m.move_id)
            {
                return Some(m.move_id);
            }
        }
        None
    }
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
        // Root: O-Q (id=1)
        //   ├─ P-A (id=2) — direct answer
        //   └─ P-Q (id=3) — sub-question
        //         └─ O-A (id=4) — answer to sub
        Arena::build(vec![
            MoveDecl {
                id: id(1),
                owner: Owner::Opponent,
                label: MoveLabel::Question,
                parent: None,
            },
            MoveDecl {
                id: id(2),
                owner: Owner::Proponent,
                label: MoveLabel::Answer,
                parent: Some(id(1)),
            },
            MoveDecl {
                id: id(3),
                owner: Owner::Proponent,
                label: MoveLabel::Question,
                parent: Some(id(1)),
            },
            MoveDecl {
                id: id(4),
                owner: Owner::Opponent,
                label: MoveLabel::Answer,
                parent: Some(id(3)),
            },
        ])
        .unwrap()
    }

    #[test]
    fn empty_play_is_empty() {
        let p = Play::new();
        assert!(p.is_empty());
        assert_eq!(p.len(), 0);
    }

    #[test]
    fn first_move_must_be_root_with_no_justifier() {
        let arena = arena_simple();
        let mut p = Play::new();
        // Wrong: not the root.
        let err = p
            .append(
                &arena,
                Move {
                    move_id: id(2),
                    justifier: None,
                },
            )
            .unwrap_err();
        assert_eq!(err, PlayError::BadInitialMove);
        // Wrong: root but with a justifier.
        let err = p
            .append(
                &arena,
                Move {
                    move_id: id(1),
                    justifier: Some(id(99)),
                },
            )
            .unwrap_err();
        assert_eq!(err, PlayError::BadInitialMove);
        // Correct.
        p.append(
            &arena,
            Move {
                move_id: id(1),
                justifier: None,
            },
        )
        .unwrap();
        assert_eq!(p.len(), 1);
    }

    #[test]
    fn unknown_move_rejected() {
        let arena = arena_simple();
        let mut p = Play::new();
        let err = p
            .append(
                &arena,
                Move {
                    move_id: id(99),
                    justifier: None,
                },
            )
            .unwrap_err();
        assert!(matches!(err, PlayError::UnknownMove { .. }));
    }

    #[test]
    fn non_root_requires_justifier() {
        let arena = arena_simple();
        let mut p = Play::new();
        p.append(
            &arena,
            Move {
                move_id: id(1),
                justifier: None,
            },
        )
        .unwrap();
        let err = p
            .append(
                &arena,
                Move {
                    move_id: id(2),
                    justifier: None,
                },
            )
            .unwrap_err();
        assert!(matches!(err, PlayError::MissingJustifier { .. }));
    }

    #[test]
    fn justifier_must_be_in_play() {
        let arena = arena_simple();
        let mut p = Play::new();
        p.append(
            &arena,
            Move {
                move_id: id(1),
                justifier: None,
            },
        )
        .unwrap();
        // Try to justify by id(99) — not yet played and not in arena.
        let err = p
            .append(
                &arena,
                Move {
                    move_id: id(2),
                    justifier: Some(id(99)),
                },
            )
            .unwrap_err();
        assert!(matches!(err, PlayError::JustifierNotInPlay { .. }));
    }

    #[test]
    fn justifier_must_match_arena_parent() {
        let arena = arena_simple();
        let mut p = Play::new();
        p.append(
            &arena,
            Move {
                move_id: id(1),
                justifier: None,
            },
        )
        .unwrap();
        p.append(
            &arena,
            Move {
                move_id: id(3),
                justifier: Some(id(1)),
            },
        )
        .unwrap();
        // Try to justify (4) by (1) — but arena parent of (4) is (3).
        let err = p
            .append(
                &arena,
                Move {
                    move_id: id(4),
                    justifier: Some(id(1)),
                },
            )
            .unwrap_err();
        assert!(matches!(err, PlayError::JustifierMismatch { .. }));
    }

    #[test]
    fn well_bracketed_play_accepted() {
        let arena = arena_simple();
        let mut p = Play::new();
        p.append(
            &arena,
            Move {
                move_id: id(1),
                justifier: None,
            },
        )
        .unwrap();
        // P sub-question to root.
        p.append(
            &arena,
            Move {
                move_id: id(3),
                justifier: Some(id(1)),
            },
        )
        .unwrap();
        // O answers the sub-question (id(3) was the most recent
        // unanswered P question — bracket nesting respected).
        p.append(
            &arena,
            Move {
                move_id: id(4),
                justifier: Some(id(3)),
            },
        )
        .unwrap();
        // P now answers the root (id(1) was the most recent
        // unanswered O question).
        p.append(
            &arena,
            Move {
                move_id: id(2),
                justifier: Some(id(1)),
            },
        )
        .unwrap();
        assert_eq!(p.len(), 4);
    }

    #[test]
    fn bracketing_violation_caught() {
        // Play: O-Q1 → P-A2 (answers root) → then try to send another
        // P-A2-clone: there's no open question to close.
        let arena = arena_simple();
        let mut p = Play::new();
        p.append(
            &arena,
            Move {
                move_id: id(1),
                justifier: None,
            },
        )
        .unwrap();
        p.append(
            &arena,
            Move {
                move_id: id(2),
                justifier: Some(id(1)),
            },
        )
        .unwrap();
        // Append a second answer with no open Q — caught.
        let err = p
            .append(
                &arena,
                Move {
                    move_id: id(2),
                    justifier: Some(id(1)),
                },
            )
            .unwrap_err();
        assert_eq!(err, PlayError::BracketingNoOpenQuestion);
    }
}
