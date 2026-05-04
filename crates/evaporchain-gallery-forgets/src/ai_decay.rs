//! AI-Decay-Art runtime parameter.
//!
//! Per `research/INVENTION_STACK.md` §A2.3:
//!
//! > **AI-Decay-Art** — generative pieces taking chain-energy as
//! > runtime parameter; output literally changes as state evaporates.
//! > Basinski's Disintegration Loops on-chain.
//!
//! The shader (off-chain GLSL/WGSL) consumes a single `f64` seed plus
//! the token id. Validators agree on the seed; nobody renders pixels
//! on chain.
//!
//! The seed scalar is computed deterministically from the gallery
//! state at `epoch_now`:
//!
//! ```text
//!     seed = total_alive_energy / sum_initial_energy
//! ```
//!
//! - At epoch 0 (gallery just opened): `seed = 1.0` — full intensity.
//! - As exhibits die: numerator drops; `seed → 0.0`.
//! - When the gallery closes (all dead): `seed = 0.0` — silence.
//!
//! The shader interprets `seed` as opacity, palette saturation, audio
//! gain, whatever the artist chose. The chain doesn't care; it just
//! publishes the scalar.

use evaporchain_types::{Energy, Epoch};
use serde::{Deserialize, Serialize};

use crate::gallery::Gallery;

/// Snapshot of the gallery's runtime seed at a given epoch.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AiSeedSnapshot {
    pub epoch: Epoch,
    /// Sum of the patina_score of every exhibit at `epoch`.
    pub total_alive_energy: Energy,
    /// Sum of the initial_energy of every exhibit (constant after
    /// each deposit; recomputed for convenience).
    pub total_initial_energy: Energy,
    /// `total_alive_energy / total_initial_energy`. Range [0.0, 1.0].
    pub seed: f64,
}

/// Compute the runtime seed snapshot for `gallery` at `epoch_now`.
pub fn runtime_seed(gallery: &Gallery, epoch_now: Epoch) -> AiSeedSnapshot {
    let mut total_alive: u128 = 0;
    let mut total_initial: u128 = 0;
    for ex in &gallery.exhibits {
        total_alive = total_alive.saturating_add(ex.mayfly.score_at(epoch_now) as u128);
        total_initial =
            total_initial.saturating_add(ex.mayfly.params.initial as u128);
    }
    let seed = if total_initial == 0 {
        // Empty gallery: define seed=1.0 (the artist's "open canvas"
        // pre-deposit). When the first work lands, the seed snaps to
        // its real value.
        1.0
    } else {
        total_alive as f64 / total_initial as f64
    };
    AiSeedSnapshot {
        epoch: epoch_now,
        total_alive_energy: total_alive.min(Energy::MAX as u128) as Energy,
        total_initial_energy: total_initial.min(Energy::MAX as u128) as Energy,
        seed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gallery::Gallery;
    use crate::mayfly::MayflyToken;
    use evaporchain_types::AccountAddress;

    fn id(b: u8) -> [u8; 32] {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    fn addr(b: u8) -> AccountAddress {
        id(b)
    }

    fn fresh_gallery() -> Gallery {
        Gallery::open(id(0xFF), "TestGallery", addr(0xC1), 0)
    }

    fn mayfly(token_byte: u8, initial: u64, hl: u64) -> MayflyToken {
        MayflyToken::mint(id(token_byte), addr(0xAA), initial, hl, 0).unwrap()
    }

    #[test]
    fn empty_gallery_seed_is_one() {
        let g = fresh_gallery();
        let snap = runtime_seed(&g, 0);
        assert_eq!(snap.seed, 1.0);
        assert_eq!(snap.total_alive_energy, 0);
        assert_eq!(snap.total_initial_energy, 0);
    }

    #[test]
    fn at_deposit_epoch_seed_is_one() {
        // Just-deposited exhibit: alive == initial ⇒ seed = 1.0.
        let mut g = fresh_gallery();
        let m = mayfly(1, 1000, 100);
        g.deposit(addr(0xC1), id(1), addr(0xAA), m, 0).unwrap();
        let snap = runtime_seed(&g, 0);
        assert_eq!(snap.seed, 1.0);
        assert_eq!(snap.total_alive_energy, 1000);
        assert_eq!(snap.total_initial_energy, 1000);
    }

    #[test]
    fn seed_drops_as_exhibits_decay() {
        let mut g = fresh_gallery();
        let m = mayfly(1, 1000, 100);
        g.deposit(addr(0xC1), id(1), addr(0xAA), m, 0).unwrap();
        // After one half-life, seed should drop materially below 1.0.
        let snap = runtime_seed(&g, 100);
        assert!(snap.seed < 1.0);
        assert!(snap.seed > 0.0);
    }

    #[test]
    fn closed_gallery_seed_is_zero() {
        // Doctrine: "When the gallery closes (all dead): seed = 0.0
        // — silence."
        let mut g = fresh_gallery();
        let m = mayfly(1, 4, 1); // dies fast
        g.deposit(addr(0xC1), id(1), addr(0xAA), m, 0).unwrap();
        let snap = runtime_seed(&g, 10_000);
        assert_eq!(snap.seed, 0.0);
        assert_eq!(snap.total_alive_energy, 0);
    }

    #[test]
    fn seed_is_weighted_by_per_exhibit_energy() {
        // Two exhibits with very different initial energies. The seed
        // is the energy-weighted alive fraction, not a simple count.
        let mut g = fresh_gallery();
        // Big exhibit: enormous half-life so 10 epochs are well within
        // the first halving — ≈ initial energy preserved.
        let big = mayfly(1, 1_000_000, 1_000_000);
        let small = mayfly(2, 4, 1); // dies fast
        g.deposit(addr(0xC1), id(1), addr(0xAA), big, 0).unwrap();
        g.deposit(addr(0xC1), id(2), addr(0xBB), small, 0).unwrap();
        let snap = runtime_seed(&g, 10);
        // The small one is dead; the big one is barely decayed. The
        // seed should be very close to 1.0 because the big one
        // dominates the weighted average.
        assert!(
            snap.seed > 0.99,
            "energy-weighted seed should be ≈1.0, got {}",
            snap.seed
        );
    }

    #[test]
    fn seed_is_monotone_non_increasing_in_time() {
        // Doctrine: "output literally changes as state evaporates."
        // The chain's seed scalar is monotone non-increasing in epoch
        // — never refreshes spontaneously.
        let mut g = fresh_gallery();
        let m = mayfly(1, 1000, 100);
        g.deposit(addr(0xC1), id(1), addr(0xAA), m, 0).unwrap();
        let mut prior = runtime_seed(&g, 0).seed;
        for t in [10, 50, 100, 500, 5000] {
            let s = runtime_seed(&g, t).seed;
            assert!(s <= prior, "seed went up at t={t}: {prior} → {s}");
            prior = s;
        }
    }

    #[test]
    fn round_trip_serde() {
        let snap = AiSeedSnapshot {
            epoch: 42,
            total_alive_energy: 800,
            total_initial_energy: 1000,
            seed: 0.8,
        };
        let s = serde_json::to_string(&snap).unwrap();
        let back: AiSeedSnapshot = serde_json::from_str(&s).unwrap();
        assert_eq!(back.epoch, 42);
        assert_eq!(back.seed, 0.8);
    }
}
