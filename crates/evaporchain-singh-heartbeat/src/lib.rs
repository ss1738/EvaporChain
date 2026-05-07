//! Singh-Heartbeat (EvaporWallet-Pulse).
//!
//! Per `research/INVENTION_STACK.md` §A5.4:
//!
//! > Persistent ambient signal (visual + haptic) encoding aggregate
//! > wallet energy as a pulse rate. Healthy: slow 60bpm green pulse.
//! > Decay below threshold: arrhythmic red. Apple Watch complication
//! > shows sparkline of wallet's heartbeat over 24h.
//!
//! > **Cultural lineage:** Apple Watch heart-rate haptics; Tesla
//! > heartbeat-on-screen; Nest Leaf icon; Oura Ring rest signal.
//!
//! > **Pitch:** *"your crypto wallet has a pulse."*
//!
//! ## What this crate ships
//!
//! Pure-function backing logic for the ambient pulse — the wallet UI
//! (mobile / watch / extension) renders pixels and haptics; this
//! crate computes the validator-agreed `Vitals` tuple at any epoch
//! from a slice of [`evaporchain_singh_triage::TriageItem`]s.
//!
//! Three structural decisions:
//!
//! 1. **The pulse rate is a function of *aggregate health*, not item
//!    count.** A wallet with 500 healthy items pulses the same rate
//!    as one with 5 healthy items, because both are "fine." A wallet
//!    where the average remaining-life-fraction is below threshold
//!    pulses fast and irregular. The rate maps to **how worried you
//!    should be**, not how busy you are.
//!
//! 2. **Arrhythmia is an explicit signal**, not a metaphor.
//!    Concretely: if the *worst* item is far-decayed while others are
//!    healthy, the pulse pattern carries an `arrhythmia_amplitude`
//!    that the haptic engine renders as missed beats. The rate is
//!    aggregate; the rhythm is per-item-tail. This is what makes the
//!    wallet "feel wrong" before the user sees the inbox.
//!
//! 3. **Colour is qualitative, not raw RGB.** The chain agrees on a
//!    `PulseColor` enum (Green / Amber / Red); the wallet maps to
//!    actual hex codes. Validators don't agree on pixels; they agree
//!    on the qualitative state. Same pattern Singh-Sabi uses for
//!    visual entropy.
//!
//! ## Module map
//!
//! - [`vitals`] — [`Vitals`] tuple + [`pulse_at`] computation.
//! - [`color`] — [`PulseColor`] enum + thresholds.
//! - [`sparkline`] — [`sparkline_24h`]: 24-tick history series the
//!   wallet renders as a watch-complication sparkline.

pub mod color;
pub mod sparkline;
pub mod vitals;

pub use color::{color_for_health, PulseColor};
pub use sparkline::{sparkline_24h, SparklinePoint};
pub use vitals::{pulse_at, Vitals};

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Singh-Heartbeat ships traffic-light qualitative
    /// state — Green (≥0.75), Amber ([0.40, 0.75)), Red (<0.40) — so
    /// validators agree on STATE, not pixels. The wallet maps colours
    /// to brand-specific hex; the chain agrees only on the qualitative
    /// bucket. Out-of-range health values clamp into the closest
    /// valid bucket."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Boundaries: 0.75 = Green, 0.74 = Amber, 0.40 = Amber, 0.39 = Red.
        assert_eq!(color_for_health(1.0), PulseColor::Green);
        assert_eq!(color_for_health(0.75), PulseColor::Green);
        assert_eq!(color_for_health(0.74), PulseColor::Amber);
        assert_eq!(color_for_health(0.50), PulseColor::Amber);
        assert_eq!(color_for_health(0.40), PulseColor::Amber);
        assert_eq!(color_for_health(0.39), PulseColor::Red);
        assert_eq!(color_for_health(0.0), PulseColor::Red);

        // Out-of-range values clamp.
        assert_eq!(color_for_health(-0.5), PulseColor::Red);
        assert_eq!(color_for_health(2.0), PulseColor::Green);

        // Determinism: same health → same colour.
        assert_eq!(color_for_health(0.5), color_for_health(0.5),);
    }
}
