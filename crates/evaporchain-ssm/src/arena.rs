//! Arena — the static game tree.
//!
//! A `MoveDecl` is a single move *declaration* (not yet played). Each
//! has:
//!
//! - an `id` (32-byte hash, caller-supplied)
//! - an `owner` (Proponent or Opponent)
//! - a `label` (Question or Answer in the Hyland-Ong sense — Q
//!   triggers play; A closes a Q)
//! - a `parent: Option<MoveId>` — the move that enables this one.
//!   The root move has `parent = None`.
//!
//! Arena invariants enforced at construction:
//! - There is exactly one root (a unique move with `parent = None`).
//! - Every non-root parent must be an already-declared move.
//! - The root must be an Opponent Question (the initial query that
//!   opens play).
//! - Owners *alternate* on parent-child edges (P enables O; O enables
//!   P). This is the "polarity" rule of HO games.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

pub type MoveId = [u8; 32];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Owner {
    /// The contract code's "side" — the strategy.
    Proponent,
    /// The caller / adversary / environment.
    Opponent,
}

impl Owner {
    pub fn opposite(self) -> Self {
        match self {
            Owner::Proponent => Owner::Opponent,
            Owner::Opponent => Owner::Proponent,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MoveLabel {
    /// A *question* — opens a thread that demands an answer.
    Question,
    /// An *answer* — closes the most recent unanswered question of
    /// the opposite owner (well-bracketed condition).
    Answer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveDecl {
    pub id: MoveId,
    pub owner: Owner,
    pub label: MoveLabel,
    pub parent: Option<MoveId>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ArenaError {
    #[error("duplicate move id")]
    DuplicateMoveId,
    #[error("multiple roots — only one move may have parent=None")]
    MultipleRoots,
    #[error("no root — exactly one move must have parent=None")]
    NoRoot,
    #[error("root must be Opponent Question (start of play)")]
    RootNotOpponentQuestion,
    #[error("parent {parent:?} does not exist for child {child:?}")]
    ParentNotFound { parent: MoveId, child: MoveId },
    #[error(
        "polarity violation: parent {parent_owner:?} → child {child_owner:?} (must alternate)"
    )]
    PolarityViolation {
        parent_owner: Owner,
        child_owner: Owner,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Arena {
    pub moves: Vec<MoveDecl>,
    pub root: MoveId,
}

impl Arena {
    pub fn build(moves: Vec<MoveDecl>) -> Result<Self, ArenaError> {
        // Detect duplicate ids.
        let mut seen = HashMap::new();
        for m in &moves {
            if seen.insert(m.id, m).is_some() {
                return Err(ArenaError::DuplicateMoveId);
            }
        }

        // Find the root.
        let root_id = {
            let roots: Vec<&MoveDecl> = moves.iter().filter(|m| m.parent.is_none()).collect();
            if roots.len() > 1 {
                return Err(ArenaError::MultipleRoots);
            }
            let root_decl = roots.into_iter().next().ok_or(ArenaError::NoRoot)?;
            if root_decl.owner != Owner::Opponent || root_decl.label != MoveLabel::Question {
                return Err(ArenaError::RootNotOpponentQuestion);
            }
            root_decl.id
        };

        // Every non-root parent must exist; polarity must alternate.
        for m in &moves {
            if let Some(p) = m.parent {
                let parent = seen.get(&p).ok_or(ArenaError::ParentNotFound {
                    parent: p,
                    child: m.id,
                })?;
                if parent.owner == m.owner {
                    return Err(ArenaError::PolarityViolation {
                        parent_owner: parent.owner,
                        child_owner: m.owner,
                    });
                }
            }
        }

        Ok(Self {
            moves,
            root: root_id,
        })
    }

    pub fn lookup(&self, id: MoveId) -> Option<&MoveDecl> {
        self.moves.iter().find(|m| m.id == id)
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

    fn opp_q(i: u8, parent: Option<MoveId>) -> MoveDecl {
        MoveDecl {
            id: id(i),
            owner: Owner::Opponent,
            label: MoveLabel::Question,
            parent,
        }
    }

    fn prop_q(i: u8, parent: Option<MoveId>) -> MoveDecl {
        MoveDecl {
            id: id(i),
            owner: Owner::Proponent,
            label: MoveLabel::Question,
            parent,
        }
    }

    fn prop_a(i: u8, parent: Option<MoveId>) -> MoveDecl {
        MoveDecl {
            id: id(i),
            owner: Owner::Proponent,
            label: MoveLabel::Answer,
            parent,
        }
    }

    #[test]
    fn opposite_inverts() {
        assert_eq!(Owner::Proponent.opposite(), Owner::Opponent);
        assert_eq!(Owner::Opponent.opposite(), Owner::Proponent);
    }

    #[test]
    fn build_rejects_no_root() {
        let err = Arena::build(vec![]).unwrap_err();
        assert_eq!(err, ArenaError::NoRoot);
    }

    #[test]
    fn build_rejects_multiple_roots() {
        let err = Arena::build(vec![opp_q(1, None), opp_q(2, None)]).unwrap_err();
        assert_eq!(err, ArenaError::MultipleRoots);
    }

    #[test]
    fn build_rejects_duplicate_ids() {
        let err = Arena::build(vec![opp_q(1, None), opp_q(1, None)]).unwrap_err();
        assert_eq!(err, ArenaError::DuplicateMoveId);
    }

    #[test]
    fn build_rejects_proponent_root() {
        let err = Arena::build(vec![prop_q(1, None)]).unwrap_err();
        assert_eq!(err, ArenaError::RootNotOpponentQuestion);
    }

    #[test]
    fn build_rejects_missing_parent() {
        let err = Arena::build(vec![
            opp_q(1, None),
            prop_a(2, Some(id(99))), // parent doesn't exist
        ])
        .unwrap_err();
        assert!(matches!(err, ArenaError::ParentNotFound { .. }));
    }

    #[test]
    fn build_rejects_polarity_violation() {
        // Opponent → Opponent edge.
        let err = Arena::build(vec![
            opp_q(1, None),
            opp_q(2, Some(id(1))), // O parent, O child — illegal
        ])
        .unwrap_err();
        assert!(matches!(err, ArenaError::PolarityViolation { .. }));
    }

    #[test]
    fn build_accepts_well_formed_arena() {
        let arena = Arena::build(vec![
            opp_q(1, None),         // root: O Q
            prop_a(2, Some(id(1))), // P A — answers root
            prop_q(3, Some(id(1))), // P Q — also justified by root
        ])
        .unwrap();
        assert_eq!(arena.root, id(1));
        assert_eq!(arena.moves.len(), 3);
    }

    #[test]
    fn lookup_returns_decl() {
        let arena = Arena::build(vec![opp_q(1, None), prop_a(2, Some(id(1)))]).unwrap();
        assert_eq!(arena.lookup(id(1)).unwrap().owner, Owner::Opponent);
        assert_eq!(arena.lookup(id(2)).unwrap().owner, Owner::Proponent);
        assert!(arena.lookup(id(99)).is_none());
    }

    #[test]
    fn round_trip_serde() {
        let arena = Arena::build(vec![opp_q(1, None), prop_a(2, Some(id(1)))]).unwrap();
        let s = serde_json::to_string(&arena).unwrap();
        let back: Arena = serde_json::from_str(&s).unwrap();
        assert_eq!(arena, back);
    }
}
