//! `Observation` — one price observation + its energy + timestamp.

use serde::{Deserialize, Serialize};

/// Fixed-point price: 1.0 = 1_000_000 micros. u64 covers prices
/// up to ~1.8 × 10^13 with 6 decimal places — enough for any
/// practical asset price.
pub type PriceMicros = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct Observation {
    /// Epoch at which the price was observed. Used for window
    /// eviction.
    pub epoch: u64,
    /// Price in fixed-point micros.
    pub price_micros: PriceMicros,
    /// Energy-at-observation. Acts as the weight in the EW-TWAP
    /// average. Validators agree on this; the chain's higher
    /// layer supplies it (typically the attesting party's stake
    /// energy, possibly multiplied by a freshness factor).
    pub energy: u64,
}

impl Observation {
    pub fn new(epoch: u64, price_micros: PriceMicros, energy: u64) -> Self {
        Self {
            epoch,
            price_micros,
            energy,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_serde() {
        let o = Observation::new(100, 1_500_000, 1000);
        let s = serde_json::to_string(&o).unwrap();
        let back: Observation = serde_json::from_str(&s).unwrap();
        assert_eq!(o, back);
    }
}
