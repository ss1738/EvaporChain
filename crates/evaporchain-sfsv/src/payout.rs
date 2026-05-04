//! Payout resolver — checks the predicate at `epoch_now` and, if true,
//! credits the deposit to the current holder, marking the vault Released.
//!
//! Payouts are *idempotent* in the sense that a Released vault cannot
//! be paid out again (`PayoutError::AlreadyReleased`). Validators must
//! call `payout` only once per vault per chain history; the higher
//! transaction layer is responsible for ensuring at-most-once execution.

use evaporchain_types::{AccountAddress, Energy, Epoch};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::predicate::{evaluate, PredicateContext};
use crate::vault::{Vault, VaultStatus};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PayoutError {
    #[error("predicate not yet satisfied at epoch {epoch_now}")]
    PredicateNotSatisfied { epoch_now: Epoch },
    #[error("vault was already released — cannot pay out twice")]
    AlreadyReleased,
}

/// Result of a successful payout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayoutResolution {
    pub paid_to: AccountAddress,
    pub amount: Energy,
    pub payout_at: Epoch,
}

/// Attempt to pay out the vault as of `epoch_now`. Mutates the vault
/// to `Released` on success.
pub fn payout(vault: &mut Vault, epoch_now: Epoch) -> Result<PayoutResolution, PayoutError> {
    let holder = match vault.status {
        VaultStatus::Locked { current_holder } => current_holder,
        VaultStatus::Released { .. } => return Err(PayoutError::AlreadyReleased),
    };
    if !evaluate(&vault.predicate, PredicateContext { epoch_now }) {
        return Err(PayoutError::PredicateNotSatisfied { epoch_now });
    }
    let amount = vault.deposit;
    vault.mark_released(holder, epoch_now);
    Ok(PayoutResolution {
        paid_to: holder,
        amount,
        payout_at: epoch_now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::predicate::Predicate;
    use crate::vault::Vault;

    fn id(b: u8) -> [u8; 32] {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    fn addr(b: u8) -> AccountAddress {
        id(b)
    }

    fn vault_with(predicate: Predicate) -> Vault {
        Vault::create(id(0xFF), addr(0xAA), addr(0xBB), 1_000, predicate, 0).unwrap()
    }

    #[test]
    fn payout_blocked_until_predicate_trips() {
        let mut v = vault_with(Predicate::EpochReached { release_epoch: 100 });
        let err = payout(&mut v, 50).unwrap_err();
        assert!(matches!(err, PayoutError::PredicateNotSatisfied { .. }));
        assert!(v.is_locked());
    }

    #[test]
    fn payout_releases_to_current_holder_at_first_satisfying_epoch() {
        let mut v = vault_with(Predicate::EpochReached { release_epoch: 100 });
        let res = payout(&mut v, 100).unwrap();
        assert_eq!(res.paid_to, addr(0xBB));
        assert_eq!(res.amount, 1_000);
        assert_eq!(res.payout_at, 100);
        assert!(!v.is_locked());
        assert_eq!(
            v.status,
            VaultStatus::Released {
                paid_to: addr(0xBB),
                payout_at: 100,
            }
        );
    }

    #[test]
    fn second_payout_attempt_errors() {
        let mut v = vault_with(Predicate::EpochReached { release_epoch: 100 });
        payout(&mut v, 100).unwrap();
        let err = payout(&mut v, 110).unwrap_err();
        assert_eq!(err, PayoutError::AlreadyReleased);
    }

    #[test]
    fn payout_after_resale_credits_buyer_not_creator() {
        // Doctrine claim: "your future self can't sue" — once sold,
        // the original holder loses any payout claim.
        let mut v = vault_with(Predicate::EpochReached { release_epoch: 100 });
        // Future-self (holder) sells to 0xCC.
        v.transfer_claim(addr(0xBB), addr(0xCC)).unwrap();
        let res = payout(&mut v, 100).unwrap();
        // 0xCC gets the deposit, not the original future_self (0xBB).
        assert_eq!(res.paid_to, addr(0xCC));
    }

    #[test]
    fn payout_with_energy_decay_predicate() {
        // Energy starts at 1000, half-life 10. Threshold = 1.
        // After 100 epochs, energy ≈ 1000 / 2^10 ≈ 0 < 1.
        let pred = Predicate::EnergyDecaysBelow {
            initial_energy: 1000,
            half_life: 10,
            created_at_epoch: 0,
            threshold: 1,
        };
        let mut v = vault_with(pred);
        // At epoch 5 (half a half-life), still ≥ 1 → blocked.
        assert!(payout(&mut v, 5).is_err());
        // At epoch 1000, fully decayed → satisfies.
        let res = payout(&mut v, 1000).unwrap();
        assert_eq!(res.paid_to, addr(0xBB));
    }

    #[test]
    fn round_trip_serde_resolution() {
        let r = PayoutResolution {
            paid_to: addr(0xAB),
            amount: 7_777,
            payout_at: 42,
        };
        let s = serde_json::to_string(&r).unwrap();
        let back: PayoutResolution = serde_json::from_str(&s).unwrap();
        assert_eq!(r, back);
    }
}
