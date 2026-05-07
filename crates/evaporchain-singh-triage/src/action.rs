//! Three swipe actions: Refresh, LetDie, Archive.
//!
//! Each is a pure transformation on a `TriageItem`. The wallet
//! presents these as swipe gestures; the chain enforces the
//! semantics. Validators agree on the post-action state.
//!
//! Refresh costs energy from the actor's balance (paid back into
//! the refresh pool by the higher transaction layer); this crate
//! computes the *effect on the item*, not the payment side.

use evaporchain_types::{Energy, Epoch};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::item::TriageItem;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    /// Pay to bump `energy_at_anchor` back up and reset
    /// `last_refreshed_epoch` to `epoch_now`.
    Refresh { top_up: Energy },
    /// Stop tracking the item — no payment, no refresh. The item
    /// continues decaying naturally; this is a UI-only "archive."
    LetDie,
    /// Mark the item ghosted — caller's higher layer can prune.
    Archive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionOutcome {
    Refreshed {
        new_energy: Energy,
        anchored_at: Epoch,
    },
    LetDie,
    Archived,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ActionError {
    #[error("refresh top_up must be > 0")]
    ZeroTopUp,
    #[error("refresh would overflow Energy::MAX")]
    Overflow,
}

/// Apply an action; mutate the item in place; return the outcome
/// payload for the wallet to render.
pub fn apply_action(
    item: &mut TriageItem,
    action: Action,
    epoch_now: Epoch,
) -> Result<ActionOutcome, ActionError> {
    match action {
        Action::Refresh { top_up } => {
            if top_up == 0 {
                return Err(ActionError::ZeroTopUp);
            }
            // Compute current energy at epoch_now, add the top-up,
            // re-anchor to epoch_now.
            let now_e = item.energy_at(epoch_now);
            let new = now_e.checked_add(top_up).ok_or(ActionError::Overflow)?;
            item.energy_at_anchor = new;
            item.last_refreshed_epoch = epoch_now;
            Ok(ActionOutcome::Refreshed {
                new_energy: new,
                anchored_at: epoch_now,
            })
        }
        Action::LetDie => Ok(ActionOutcome::LetDie),
        Action::Archive => Ok(ActionOutcome::Archived),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> TriageItem {
        TriageItem::new([0; 32], 1000, 100, 0).unwrap()
    }

    #[test]
    fn refresh_zero_top_up_rejected() {
        let mut it = fresh();
        let err = apply_action(&mut it, Action::Refresh { top_up: 0 }, 50).unwrap_err();
        assert_eq!(err, ActionError::ZeroTopUp);
    }

    #[test]
    fn refresh_bumps_energy_and_re_anchors() {
        let mut it = fresh();
        // At epoch 100, energy decayed by ~half. Top up 500.
        let outcome = apply_action(&mut it, Action::Refresh { top_up: 500 }, 100).unwrap();
        match outcome {
            ActionOutcome::Refreshed {
                new_energy,
                anchored_at,
            } => {
                assert!(new_energy > 500);
                assert!(new_energy <= 1500);
                assert_eq!(anchored_at, 100);
            }
            other => panic!("expected Refreshed, got {other:?}"),
        }
        assert_eq!(it.last_refreshed_epoch, 100);
    }

    #[test]
    fn refresh_overflow_rejected() {
        let mut it = TriageItem::new([0; 32], u64::MAX - 100, 100, 0).unwrap();
        // top_up=1000 + current ≈ MAX-100 ⇒ overflow.
        let err = apply_action(&mut it, Action::Refresh { top_up: 1000 }, 0).unwrap_err();
        assert_eq!(err, ActionError::Overflow);
    }

    #[test]
    fn let_die_does_not_mutate_item() {
        let mut it = fresh();
        let snap = it.clone();
        let outcome = apply_action(&mut it, Action::LetDie, 50).unwrap();
        assert_eq!(outcome, ActionOutcome::LetDie);
        assert_eq!(it, snap);
    }

    #[test]
    fn archive_does_not_mutate_item() {
        // The item-level `Archive` is a UI signal — higher layer prunes.
        let mut it = fresh();
        let snap = it.clone();
        let outcome = apply_action(&mut it, Action::Archive, 50).unwrap();
        assert_eq!(outcome, ActionOutcome::Archived);
        assert_eq!(it, snap);
    }

    #[test]
    fn round_trip_serde_action_and_outcome() {
        let a = Action::Refresh { top_up: 250 };
        let s = serde_json::to_string(&a).unwrap();
        let back: Action = serde_json::from_str(&s).unwrap();
        assert_eq!(a, back);
        let o = ActionOutcome::Refreshed {
            new_energy: 1000,
            anchored_at: 42,
        };
        let s = serde_json::to_string(&o).unwrap();
        let back: ActionOutcome = serde_json::from_str(&s).unwrap();
        assert_eq!(o, back);
    }
}
