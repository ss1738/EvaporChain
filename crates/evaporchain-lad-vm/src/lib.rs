//! Linear-Affine-Decay VM (LAD) substrate.
//!
//! Per `research/INVENTION_STACK.md` §4.1 row 12:
//!
//! > **Linear-Affine-Decay VM** — Move resources × decay. "Use it or
//! > evaporate." Forces liveness as a type-system property.
//!
//! Source attribution (DP-VM agent, round 2): "Move × Wadler-Girard
//! linear logic × decay."
//!
//! ## What this substrate ships
//!
//! Real Move-style linearity needs *compiler-enforced* substructural
//! types — that lives in a future `evaporchain-script-lad` (the LAD
//! frontend to the existing scripting layer). This crate ships the
//! *runtime* data structures + the operational semantics so the
//! frontend has a target to lower into.
//!
//! Three substructural modes:
//!
//! - **Linear** — must be consumed exactly once.
//! - **Affine** — may be consumed at most once (drop is allowed).
//! - **Decaying** — affine, plus an explicit decay window. Consumes
//!   itself implicitly when `current_epoch ≥ created_at + window`.
//!
//! Liveness as a type-system property: a `Decaying` resource that
//! the script never `use`s gets recorded as *evaporated* on the next
//! epoch tick — automatic GC for stale state, and the chain can
//! refuse to honour a tx that touches an evaporated resource.
//!
//! ## Module map
//!
//! - [`mode`] — `Mode` enum.
//! - [`resource`] — `Resource<T> { value, mode, created_at,
//!   decay_window, consumed }`.
//! - [`ops`] — `use_resource`, `drop_resource`, `tick_decay`.

pub mod mode;
pub mod ops;
pub mod resource;

pub use mode::Mode;
pub use ops::{drop_resource, tick_decay, use_resource, OpError};
pub use resource::Resource;

#[cfg(test)]
mod press_claim_tests {
    //! The press claim lives as a test (INVENTION_STACK §4.1 row 12):
    //! "Move resources × decay. 'Use it or evaporate.' Forces liveness
    //! as a type-system property."
    //!
    //! Three invariants that MUST hold at the runtime level:
    //!
    //! 1. **Linear**: consumed exactly once — drop rejected, second use rejected.
    //! 2. **Affine**: consumed at most once — drop allowed.
    //! 3. **Decaying**: implicitly evaporates past window — no transaction needed.

    use crate::{drop_resource, tick_decay, use_resource, Mode, OpError, Resource};

    #[test]
    fn linear_must_be_consumed_exactly_once() {
        // Happy path: first use returns the value.
        let r = Resource::linear(42u64, 0);
        let (v, receipt) = use_resource(r, 1).unwrap();
        assert_eq!(v, 42);
        assert!(receipt.consumed);
    }

    #[test]
    fn linear_second_use_rejected() {
        let mut r = Resource::linear(0u8, 0);
        r.consumed = true;
        let err = use_resource(r, 1).unwrap_err();
        assert_eq!(err, OpError::AlreadyConsumed);
    }

    #[test]
    fn linear_cannot_be_dropped() {
        // Core invariant: drop on Linear rejects. There is no escape hatch.
        let r = Resource::linear("one-time-ticket", 0);
        let err = drop_resource(r).unwrap_err();
        assert_eq!(err, OpError::LinearCannotDrop);
    }

    #[test]
    fn affine_can_be_dropped_without_use() {
        // Affine resources may be discarded — no obligation to consume.
        let r = Resource::affine("coupon", 0);
        drop_resource(r).unwrap();
    }

    #[test]
    fn decaying_evaporates_without_transaction() {
        // Doctrine claim: zero TXs needed — the resource evaporates by itself.
        // tick_decay at window+1 marks consumed; use then returns Evaporated.
        let r = Resource::decaying("day-pass", 0u64, 10);
        let ticked = tick_decay(r, 11);
        assert!(ticked.consumed, "resource must be marked evaporated at window+1");
        let err = use_resource(ticked, 11).unwrap_err();
        assert_eq!(err, OpError::AlreadyConsumed);
    }

    #[test]
    fn decaying_use_past_window_rejected_directly() {
        // Even without an explicit tick, use_resource rejects a decayed resource.
        let r = Resource::decaying("pass", 0u64, 10);
        let err = use_resource(r, 100).unwrap_err();
        assert_eq!(err, OpError::Evaporated);
    }

    #[test]
    fn mode_identity_preserved() {
        let linear  = Resource::linear(1u8, 0);
        let affine  = Resource::affine(2u8, 0);
        let decaying = Resource::decaying(3u8, 0, 50);
        assert_eq!(linear.mode,  Mode::Linear);
        assert_eq!(affine.mode,  Mode::Affine);
        assert_eq!(decaying.mode, Mode::Decaying);
    }
}
