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
