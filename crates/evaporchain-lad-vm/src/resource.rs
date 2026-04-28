//! `Resource<T>` — generic substructural resource with optional
//! decay window.

use serde::{Deserialize, Serialize};

use crate::mode::Mode;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resource<T> {
    pub value: T,
    pub mode: Mode,
    pub created_at_epoch: u64,
    /// `None` for non-decaying modes; `Some(window)` only when
    /// `mode == Decaying`. Window is in epochs from `created_at_epoch`.
    pub decay_window: Option<u64>,
    /// Set true when consumed via `use_resource` or evaporated via
    /// `tick_decay`. A consumed resource cannot be used again.
    pub consumed: bool,
}

impl<T> Resource<T> {
    /// Linear resource — must be consumed exactly once.
    pub const fn linear(value: T, created_at_epoch: u64) -> Self {
        Self {
            value,
            mode: Mode::Linear,
            created_at_epoch,
            decay_window: None,
            consumed: false,
        }
    }

    /// Affine resource — may be consumed at most once.
    pub const fn affine(value: T, created_at_epoch: u64) -> Self {
        Self {
            value,
            mode: Mode::Affine,
            created_at_epoch,
            decay_window: None,
            consumed: false,
        }
    }

    /// Decaying resource — affine + automatic evaporation after `window`.
    pub const fn decaying(value: T, created_at_epoch: u64, window: u64) -> Self {
        Self {
            value,
            mode: Mode::Decaying,
            created_at_epoch,
            decay_window: Some(window),
            consumed: false,
        }
    }

    /// True iff this is a decaying resource that has aged past its
    /// window at `current_epoch`. Independent of `consumed`.
    pub fn is_evaporated(&self, current_epoch: u64) -> bool {
        match self.mode {
            Mode::Decaying => match self.decay_window {
                Some(window) => current_epoch >= self.created_at_epoch.saturating_add(window),
                None => false,
            },
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_construction() {
        let r = Resource::linear(42u64, 0);
        assert_eq!(r.mode, Mode::Linear);
        assert!(!r.consumed);
        assert_eq!(r.decay_window, None);
    }

    #[test]
    fn affine_construction() {
        let r = Resource::affine("hello".to_string(), 5);
        assert_eq!(r.mode, Mode::Affine);
    }

    #[test]
    fn decaying_construction_carries_window() {
        let r = Resource::decaying(123u32, 10, 100);
        assert_eq!(r.mode, Mode::Decaying);
        assert_eq!(r.decay_window, Some(100));
    }

    #[test]
    fn decaying_is_evaporated_past_window() {
        let r = Resource::decaying(0u8, 10, 50);
        // Created at 10, window 50 → evaporates at epoch 60.
        assert!(!r.is_evaporated(59));
        assert!(r.is_evaporated(60));
        assert!(r.is_evaporated(100));
    }

    #[test]
    fn linear_never_evaporates() {
        let r = Resource::linear(0u8, 10);
        assert!(!r.is_evaporated(u64::MAX));
    }
}
