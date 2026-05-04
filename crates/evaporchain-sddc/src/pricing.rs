//! Monotone-descending step price curve for SDDC.
//!
//! Price descends linearly per epoch from `ceiling` to `floor` over the
//! window `[opened_at, opened_at + duration_epochs]`. After the window
//! closes the curve clamps to `floor`.
//!
//! Linear (not exponential) so the math is integer-only and the
//! per-epoch price is independent of bid arrival pattern. Bidders can
//! compute the price at any epoch from the auction header alone — no
//! state lookup.
//!
//! Invariants:
//! - `floor ≤ ceiling` (rejected at construction)
//! - `current_price(opened_at) == ceiling`
//! - `current_price(opened_at + duration_epochs) == floor`
//! - `current_price(t1) >= current_price(t2)` whenever `t1 ≤ t2`
//!   (monotone non-increasing in time)

use evaporchain_types::{Energy, Epoch};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PricingError {
    #[error("ceiling must be ≥ floor")]
    InvertedBounds,
    #[error("duration_epochs must be > 0")]
    ZeroDuration,
}

/// Current price at `epoch_now`, given a linear descent from `ceiling`
/// to `floor` over `duration_epochs` starting at `opened_at`.
///
/// Returns `floor` for any `epoch_now` past the window's end.
pub fn current_price(
    ceiling: Energy,
    floor: Energy,
    opened_at: Epoch,
    duration_epochs: u64,
    epoch_now: Epoch,
) -> Result<Energy, PricingError> {
    if floor > ceiling {
        return Err(PricingError::InvertedBounds);
    }
    if duration_epochs == 0 {
        return Err(PricingError::ZeroDuration);
    }
    if epoch_now <= opened_at {
        return Ok(ceiling);
    }
    let elapsed = epoch_now.saturating_sub(opened_at);
    if elapsed >= duration_epochs {
        return Ok(floor);
    }
    // Linear interpolation in u128 to avoid overflow on
    // (ceiling - floor) * elapsed for large energies.
    let span = (ceiling - floor) as u128;
    let elapsed_u = elapsed as u128;
    let dur_u = duration_epochs as u128;
    let drop = span * elapsed_u / dur_u;
    Ok(ceiling - drop as Energy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_inverted_bounds() {
        assert_eq!(
            current_price(50, 100, 0, 10, 5).unwrap_err(),
            PricingError::InvertedBounds
        );
    }

    #[test]
    fn rejects_zero_duration() {
        assert_eq!(
            current_price(100, 50, 0, 0, 5).unwrap_err(),
            PricingError::ZeroDuration
        );
    }

    #[test]
    fn at_open_is_ceiling() {
        assert_eq!(current_price(1000, 100, 50, 100, 50).unwrap(), 1000);
    }

    #[test]
    fn before_open_is_ceiling() {
        // Some clock skew — caller may pass an epoch before opened_at.
        // Treat as not-yet-started so the price is still the ceiling.
        assert_eq!(current_price(1000, 100, 50, 100, 0).unwrap(), 1000);
    }

    #[test]
    fn at_close_is_floor() {
        assert_eq!(current_price(1000, 100, 0, 100, 100).unwrap(), 100);
    }

    #[test]
    fn past_close_clamps_to_floor() {
        assert_eq!(current_price(1000, 100, 0, 100, 9999).unwrap(), 100);
    }

    #[test]
    fn linear_midpoint() {
        // ceiling=1000, floor=100, span=900, dur=100.
        // At elapsed=50 ⇒ drop=450 ⇒ price=550.
        assert_eq!(current_price(1000, 100, 0, 100, 50).unwrap(), 550);
    }

    #[test]
    fn equal_floor_and_ceiling_is_constant() {
        // Degenerate but valid: a constant-price auction.
        for t in [0, 50, 100, 200] {
            assert_eq!(current_price(500, 500, 0, 100, t).unwrap(), 500);
        }
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Price must be monotone non-increasing in time.
        #[test]
        fn monotone_non_increasing(
            ceiling in 100u64..1_000_000,
            floor in 0u64..100,
            opened_at in 0u64..1_000,
            duration in 1u64..1_000,
            t_a in 0u64..2_000,
            extra in 0u64..2_000,
        ) {
            let p_a = current_price(ceiling, floor, opened_at, duration, t_a).unwrap();
            let p_b = current_price(
                ceiling,
                floor,
                opened_at,
                duration,
                t_a.saturating_add(extra),
            )
            .unwrap();
            prop_assert!(p_b <= p_a, "price went up: {} → {}", p_a, p_b);
        }

        /// Price is always within [floor, ceiling].
        #[test]
        fn in_bounds(
            ceiling in 0u64..1_000_000,
            floor in 0u64..1_000_000,
            opened_at in 0u64..1_000,
            duration in 1u64..1_000,
            epoch_now in 0u64..2_000,
        ) {
            let (lo, hi) = (floor.min(ceiling), floor.max(ceiling));
            let res = current_price(hi, lo, opened_at, duration, epoch_now);
            // We constructed (hi, lo) with hi >= lo so this must succeed.
            let p = res.unwrap();
            prop_assert!(p >= lo && p <= hi);
        }
    }
}
