//! Singh Attractor Consensus V2 — Bell-anchored fallback +
//! bounded Lyapunov drift.
//!
//! V1 (`evaporchain-singh-attractor`) ships basin-membership and a
//! deterministic *nearest-centre* fallback. The fallback is
//! predictable: any validator can compute it from public state, so
//! a malicious proposer can target the fallback attractor by
//! pushing state into the no-basin region.
//!
//! V2 closes that gap:
//!
//! 1. **Bell-anchored fallback** — when no basin contains the
//!    state, V2 uses a 32-byte certificate seed (typically from
//!    `evaporchain-bell-beacon-v2::BellCertificate.seed`) to draw
//!    the fallback attractor by *energy-distance-weighted sampling*.
//!    Closer attractors are likelier, but not deterministic; the
//!    seed makes the choice unpredictable until the certificate
//!    is published. Anti-grinding follows from the Bell-Beacon's
//!    own anti-grinding properties (sorted-tag seed derivation).
//!
//! 2. **Bounded Lyapunov drift** — V2 also returns the next-step
//!    nudge `drift = clamp(center − state, ±drift_rate)`. The
//!    chain applies this drift to the energy state each epoch.
//!    Strict-decrement of `|state − center|` ⇒ Lyapunov stability
//!    on the basin's interior.
//!
//! 3. **In-basin selection unchanged** — when a basin contains
//!    the state, V2 picks that attractor identically to V1. Only
//!    the fallback path consumes the certificate seed.

pub mod draw;

pub use draw::{draw_attractor, AttractorV2, DrawError, DrawResult};

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Singh Attractor V2 picks in-basin attractors
    /// deterministically (V1 behaviour, seed unused) and the
    /// fallback path consumes the Bell-certificate seed for
    /// energy-distance-weighted sampling. Different seeds may yield
    /// different fallback selections; the same seed is reproducible.
    /// Drift is bounded by per-attractor drift_rate."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Two attractors, basin_radius=10 around centres 100 and 1000.
        let attractors = vec![AttractorV2::new(100, 10, 5), AttractorV2::new(1000, 10, 7)];

        // In-basin: state=105 lands inside the first basin →
        // selection is independent of seed.
        let seed_a = [0xAA; 32];
        let seed_b = [0xBB; 32];
        let r1 = draw_attractor(105, &attractors, &seed_a).unwrap();
        let r2 = draw_attractor(105, &attractors, &seed_b).unwrap();
        assert!(!r1.used_fallback);
        assert!(!r2.used_fallback);
        assert_eq!(r1.selected_index, r2.selected_index);
        assert_eq!(r1.selected_center, 100);
        // Drift bounded: |105−100|=5 ≤ drift_rate=5.
        assert_eq!(r1.drift.unsigned_abs(), 5);

        // Out-of-basin: state=500 is in the gap between basins.
        // Same seed → byte-identical result; uses fallback.
        let s = 500u64;
        let r3 = draw_attractor(s, &attractors, &seed_a).unwrap();
        let r4 = draw_attractor(s, &attractors, &seed_a).unwrap();
        assert!(r3.used_fallback);
        assert_eq!(r3, r4);

        // Drift bounded by selected attractor's drift_rate.
        let chosen = &attractors[r3.selected_index];
        assert!(r3.drift.unsigned_abs() <= chosen.drift_rate as u128);

        // Empty list → DrawError::Empty.
        assert!(matches!(
            draw_attractor(s, &[], &seed_a),
            Err(DrawError::Empty)
        ));
    }
}
