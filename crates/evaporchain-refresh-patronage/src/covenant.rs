use evaporchain_energy_kernel::refresh_pool::NamespaceId;
use evaporchain_types::Energy;
use serde::{Deserialize, Serialize};

/// A Patronage Covenant — one state object's pledge to voluntarily over-pay
/// rent, donating `donation_per_epoch` extra energy per epoch into the global
/// patronage pool credit.
///
/// The full pre-funded amount (`donation_per_epoch × epochs`) is held here and
/// drains to the pool via `honour`. Revoke refunds whatever remains.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatronageCovenant {
    pub object_id: Vec<u8>,
    pub namespace_id: NamespaceId,
    /// Extra energy per epoch donated on top of standard rent.
    pub donation_per_epoch: Energy,
    pub created_epoch: u64,
    /// First epoch NOT covered (exclusive bound).
    pub expires_epoch: u64,
    /// Energy drawn from the namespace at pledge time and not yet released.
    pub pre_funded: Energy,
    /// Cumulative energy already donated to the patronage pool credit.
    pub patronage_score: Energy,
    /// Most recent epoch for which `honour` was called successfully.
    /// `None` until first successful `honour` call.
    pub last_honoured_epoch: Option<u64>,
}

impl PatronageCovenant {
    /// Remaining funded epochs (may be fractional due to saturation arithmetic
    /// — treat as a lower bound).
    pub fn remaining_epochs(&self, current_epoch: u64) -> u64 {
        self.expires_epoch.saturating_sub(current_epoch)
    }

    pub fn is_active(&self, epoch: u64) -> bool {
        epoch < self.expires_epoch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cov(created: u64, expires: u64, last: Option<u64>) -> PatronageCovenant {
        PatronageCovenant {
            object_id: vec![1, 2, 3],
            namespace_id: vec![0xAA; 4],
            donation_per_epoch: 100,
            created_epoch: created,
            expires_epoch: expires,
            pre_funded: 1_000,
            patronage_score: 0,
            last_honoured_epoch: last,
        }
    }

    #[test]
    fn remaining_epochs_pre_expiry() {
        // Expires at 100, current 75 → 25 epochs left.
        let c = cov(0, 100, None);
        assert_eq!(c.remaining_epochs(75), 25);
    }

    #[test]
    fn remaining_epochs_at_expiry_is_zero() {
        let c = cov(0, 100, None);
        assert_eq!(c.remaining_epochs(100), 0);
    }

    #[test]
    fn remaining_epochs_post_expiry_saturates_to_zero() {
        // saturating_sub: 100 - 200 = 0, not negative.
        let c = cov(0, 100, None);
        assert_eq!(c.remaining_epochs(200), 0);
    }

    #[test]
    fn is_active_strict_less_than_expires() {
        let c = cov(0, 100, None);
        assert!(c.is_active(0));
        assert!(c.is_active(99));
        // expires_epoch is exclusive — at expiry the covenant has lapsed.
        assert!(!c.is_active(100));
        assert!(!c.is_active(101));
    }

    #[test]
    fn round_trip_serde() {
        let c = cov(5, 50, Some(20));
        let json = serde_json::to_string(&c).unwrap();
        let back: PatronageCovenant = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }
}
