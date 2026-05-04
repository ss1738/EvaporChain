//! Final-death observation surface.
//!
//! The chain's `MortisMonitor` lives inside consensus and ticks every
//! block — fully internal. Application-layer consumers (UI banners,
//! dApps that should gracefully degrade, light clients, indexers,
//! marketplace front-ends) need a *stable read-only observation
//! surface* over the chain's mortality state. This adapter provides it.
//!
//! Per `research/INVENTION_STACK.md` §A2.5:
//!
//! > **Mortis** — when refresh pool falls below ε for N epochs, the
//! > final state root is auto-minted as a single unowned NFT visible
//! > to all light clients forever.
//!
//! ## What this adapter does
//!
//! Maps a `MortisMonitor` (consensus internal) to a `MortalityState`
//! (application-facing). The state is a coarse traffic-light enum
//! the wallet/dApp UI can render:
//!
//! - **`Alive`** — refresh pool above floor; chain healthy.
//! - **`Sickening { remaining_epochs }`** — refresh pool below floor
//!   but the chain has not yet committed to die. The dApp can show a
//!   countdown banner; the wallet can warn users to take final
//!   actions.
//! - **`Dead`** — trigger fired. The chain's death certificate is
//!   minted; from this point onwards, light clients can verify the
//!   final state root forever.
//!
//! Three structural decisions:
//!
//! 1. **Pure-function over the live monitor.** The adapter does NOT
//!    own state; it observes a `&MortisMonitor` and returns a
//!    `MortalityState`. Validators agree on the monitor; consumers
//!    agree on the projection. Same pattern as the freshness-bucket
//!    function in WitnessFit.
//!
//! 2. **`Sickening.remaining_epochs` is a *promise*, not a guarantee.**
//!    If the refresh pool recovers before the trigger fires, the
//!    countdown resets — the chain is *not* committed to die just
//!    because it was sickening. This is the homeostasis claim:
//!    sickening states reverse if metabolism recovers.
//!
//! 3. **`Dead` is terminal.** The chain's death is latched in the
//!    monitor; once observed `Dead`, the state never returns to
//!    `Alive` regardless of subsequent refresh-pool fluctuations.
//!    Light clients who see `Dead` can stop expecting new blocks.

use evaporchain_mortis::MortisMonitor;
use serde::{Deserialize, Serialize};

/// Application-facing chain mortality state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MortalityState {
    /// Refresh pool is healthy; chain is operating normally.
    Alive,
    /// Refresh pool has been below floor; if the run continues for
    /// `remaining_epochs` more, the chain triggers Mortis. The pool
    /// recovering before then resets the count.
    Sickening { remaining_epochs: u64 },
    /// Mortis has triggered; the chain has signed its death
    /// certificate. Light clients can stop expecting new blocks.
    Dead,
}

impl MortalityState {
    /// True iff the chain is in a state where new blocks are
    /// expected (`Alive` or `Sickening`).
    pub fn is_alive(&self) -> bool {
        !matches!(self, MortalityState::Dead)
    }

    /// True iff the wallet/dApp UI should warn the user (any
    /// non-Alive state).
    pub fn should_warn(&self) -> bool {
        !matches!(self, MortalityState::Alive)
    }

    /// True iff the chain has terminally died.
    pub fn is_dead(&self) -> bool {
        matches!(self, MortalityState::Dead)
    }
}

/// Project the consensus-side `MortisMonitor` to an application-facing
/// `MortalityState`. Pure function; deterministic; validators agree on
/// the monitor and consumers agree on the projection.
pub fn observe(monitor: &MortisMonitor) -> MortalityState {
    if monitor.is_triggered() {
        return MortalityState::Dead;
    }
    if monitor.consecutive_below == 0 {
        return MortalityState::Alive;
    }
    let remaining = monitor
        .condition
        .sustained_epochs
        .saturating_sub(monitor.consecutive_below);
    MortalityState::Sickening {
        remaining_epochs: remaining,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_mortis::{MortisCondition, MortisMonitor};

    fn cond(floor: u64, n: u64) -> MortisCondition {
        MortisCondition::new(floor, n)
    }

    #[test]
    fn fresh_monitor_is_alive() {
        let m = MortisMonitor::new(cond(1000, 10));
        assert_eq!(observe(&m), MortalityState::Alive);
        assert!(observe(&m).is_alive());
        assert!(!observe(&m).should_warn());
    }

    #[test]
    fn one_below_tick_is_sickening_with_full_runway() {
        let mut m = MortisMonitor::new(cond(1000, 10));
        m.tick(1, 500);
        let s = observe(&m);
        // 1 of 10 epochs below; 9 left.
        assert_eq!(s, MortalityState::Sickening { remaining_epochs: 9 });
        assert!(s.is_alive());
        assert!(s.should_warn());
    }

    #[test]
    fn sickening_remaining_decreases() {
        let mut m = MortisMonitor::new(cond(1000, 5));
        m.tick(1, 500);
        assert_eq!(observe(&m), MortalityState::Sickening { remaining_epochs: 4 });
        m.tick(2, 500);
        assert_eq!(observe(&m), MortalityState::Sickening { remaining_epochs: 3 });
        m.tick(3, 500);
        assert_eq!(observe(&m), MortalityState::Sickening { remaining_epochs: 2 });
    }

    #[test]
    fn pool_recovery_returns_to_alive() {
        // Doctrine: "sickening states reverse if metabolism recovers."
        let mut m = MortisMonitor::new(cond(1000, 5));
        m.tick(1, 500);
        m.tick(2, 500);
        assert_eq!(
            observe(&m),
            MortalityState::Sickening { remaining_epochs: 3 }
        );
        // Pool recovers — counter resets.
        m.tick(3, 5000);
        assert_eq!(observe(&m), MortalityState::Alive);
    }

    #[test]
    fn sustained_breach_transitions_to_dead() {
        let mut m = MortisMonitor::new(cond(1000, 3));
        m.tick(1, 500);
        m.tick(2, 500);
        m.tick(3, 500); // triggers
        let s = observe(&m);
        assert_eq!(s, MortalityState::Dead);
        assert!(!s.is_alive());
        assert!(s.is_dead());
    }

    #[test]
    fn dead_is_terminal_even_if_pool_recovers() {
        // Doctrine: "Dead is terminal." Once the chain has signed its
        // death certificate, no amount of pool recovery brings it back.
        let mut m = MortisMonitor::new(cond(1000, 2));
        m.tick(1, 0);
        m.tick(2, 0); // dead
        assert_eq!(observe(&m), MortalityState::Dead);
        // A miraculous recovery in the underlying monitor would still
        // leave the state Dead (the monitor latches `triggered`).
        m.tick(3, 999_999_999);
        assert_eq!(observe(&m), MortalityState::Dead);
    }

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Doctrine: "EvaporChain is the first chain that signs its own
        // death certificate." Operationalised: the application layer
        // can observe the chain's mortality as a coarse 3-state enum
        // (Alive → Sickening → Dead), with Dead being terminal.
        let mut m = MortisMonitor::new(cond(1000, 3));
        // 1. Alive while healthy.
        assert!(observe(&m).is_alive());
        // 2. Sickening as the pool falls.
        m.tick(1, 0);
        assert!(matches!(observe(&m), MortalityState::Sickening { .. }));
        m.tick(2, 0);
        // 3. Dead at the trigger.
        m.tick(3, 0);
        assert!(observe(&m).is_dead());
        // 4. The death is irrevocable.
        m.tick(99, 999_999);
        assert!(observe(&m).is_dead());
    }

    #[test]
    fn round_trip_serde() {
        for state in [
            MortalityState::Alive,
            MortalityState::Sickening { remaining_epochs: 5 },
            MortalityState::Dead,
        ] {
            let s = serde_json::to_string(&state).unwrap();
            let back: MortalityState = serde_json::from_str(&s).unwrap();
            assert_eq!(state, back);
        }
    }

    #[test]
    fn warn_signals_for_non_alive_states() {
        // UI/dApp utility: the wallet should warn during Sickening
        // AND Dead but not during Alive. Verify the predicate flag.
        assert!(!MortalityState::Alive.should_warn());
        assert!(MortalityState::Sickening { remaining_epochs: 1 }.should_warn());
        assert!(MortalityState::Dead.should_warn());
    }

    #[test]
    fn alive_signals_for_pre_dead_states() {
        // Light clients should keep expecting blocks during Alive AND
        // Sickening; they can stop only when Dead.
        assert!(MortalityState::Alive.is_alive());
        assert!(MortalityState::Sickening { remaining_epochs: 1 }.is_alive());
        assert!(!MortalityState::Dead.is_alive());
    }
}
