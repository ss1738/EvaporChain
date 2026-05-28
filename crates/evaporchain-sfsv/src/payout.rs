//! Payout resolver — checks the predicate at `epoch_now` (with the
//! vault contract's live, engine-tracked energy) and, if satisfied,
//! credits the deposit to the current holder, marking the vault
//! Released.
//!
//! **EvaporScript-first (2026-05-16):** the caller supplies
//! `contract_energy` — the vault instance's *current* energy as tracked
//! by the evaporation engine (post-decay, post-`on_refresh`). This
//! module never recomputes decay (invariant #1); it only asks the
//! predicate "is the live energy below threshold yet?", exactly as the
//! `.es` `try_payout` reads its built-in `energy` field. The execution
//! layer that owns the contract instance is responsible for passing the
//! engine-true value.
//!
//! Payouts are *idempotent*: a Released vault cannot be paid out again
//! (`PayoutError::AlreadyReleased`). The higher transaction layer must
//! ensure at-most-once execution per vault per chain history.

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

/// Attempt to pay out the vault as of `epoch_now`, given the vault
/// contract's current engine-tracked energy `contract_energy`. Mutates
/// the vault to `Released` on success.
///
/// `contract_energy` is irrelevant for `EpochReached` vaults but must
/// still be supplied (the caller passes the live reading unconditionally;
/// the predicate ignores it where appropriate).
pub fn payout(
    vault: &mut Vault,
    epoch_now: Epoch,
    contract_energy: Energy,
) -> Result<PayoutResolution, PayoutError> {
    let holder = match vault.status {
        VaultStatus::Locked { current_holder } => current_holder,
        VaultStatus::Released { .. } => return Err(PayoutError::AlreadyReleased),
    };
    if !evaluate(
        &vault.predicate,
        PredicateContext {
            epoch_now,
            contract_energy,
        },
    ) {
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

    // For EpochReached vaults the live energy is irrelevant; pass the
    // deposit as a stand-in to prove it is ignored.
    const IGNORED_ENERGY: Energy = 1_000;

    #[test]
    fn payout_blocked_until_predicate_trips() {
        let mut v = vault_with(Predicate::EpochReached { release_epoch: 100 });
        let err = payout(&mut v, 50, IGNORED_ENERGY).unwrap_err();
        assert!(matches!(err, PayoutError::PredicateNotSatisfied { .. }));
        assert!(v.is_locked());
    }

    #[test]
    fn payout_releases_to_current_holder_at_first_satisfying_epoch() {
        let mut v = vault_with(Predicate::EpochReached { release_epoch: 100 });
        let res = payout(&mut v, 100, IGNORED_ENERGY).unwrap();
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
        payout(&mut v, 100, IGNORED_ENERGY).unwrap();
        let err = payout(&mut v, 110, IGNORED_ENERGY).unwrap_err();
        assert_eq!(err, PayoutError::AlreadyReleased);
    }

    #[test]
    fn payout_after_resale_credits_buyer_not_creator() {
        // Doctrine: "your future self can't sue" — once sold, the
        // original holder loses any payout claim.
        let mut v = vault_with(Predicate::EpochReached { release_epoch: 100 });
        v.transfer_claim(addr(0xBB), addr(0xCC)).unwrap();
        let res = payout(&mut v, 100, IGNORED_ENERGY).unwrap();
        assert_eq!(res.paid_to, addr(0xCC));
    }

    #[test]
    fn payout_with_energy_decay_predicate_reads_live_energy() {
        // `.es` model: release iff the vault's *live* engine energy is
        // below threshold. The engine supplies the value; payout never
        // recomputes decay.
        let mut v = vault_with(Predicate::EnergyDecaysBelow { threshold: 500 });
        // Live energy still 800 ≥ 500 → blocked (epoch is irrelevant).
        assert!(payout(&mut v, 5, 800).is_err());
        assert!(payout(&mut v, 9_999, 500).is_err()); // exactly 500 not < 500
                                                      // Engine decayed it to 499 < 500 → releases.
        let res = payout(&mut v, 1_000, 499).unwrap();
        assert_eq!(res.paid_to, addr(0xBB));
        assert_eq!(res.amount, 1_000);
    }

    #[test]
    fn payout_energy_decay_is_refresh_aware() {
        // Decayed below threshold but a boost (on_refresh) lifted live
        // energy back above it before payout was called → must NOT
        // release. The old frozen-formula predicate could not do this.
        let mut v = vault_with(Predicate::EnergyDecaysBelow { threshold: 500 });
        // refreshed back to 900 ⇒ blocked even though epochs elapsed
        let err = payout(&mut v, 2_100, 900).unwrap_err();
        assert!(matches!(err, PayoutError::PredicateNotSatisfied { .. }));
        assert!(v.is_locked());
        // later, decays again to 100 ⇒ releases
        let res = payout(&mut v, 3_000, 100).unwrap();
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
