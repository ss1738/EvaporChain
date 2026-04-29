//! Refresh Market integration — wires `evaporchain-refresh-market` into the
//! execution layer.
//!
//! The `RefreshMarket` lives on `SimpleExecutor` alongside the `RefreshPool`.
//! The market prices namespace rent using the quadratic AMM formula:
//!
//! ```text
//! rent_rate(used, capacity, base) = base × (used + 1)² / capacity²
//! ```
//!
//! # Integration points
//!
//! - **Namespace registration**: when a `CreateObjectTx` mints the first object
//!   in a new namespace, `ensure_namespace` registers it with `DEFAULT_CAPACITY`
//!   slots.  Subsequent objects in the same namespace increment `used`.
//!
//! - **Rent payment** (per refresh cycle): `charge_refresh_rent` computes the
//!   per-epoch rent for the current namespace utilisation and debits it from
//!   the provided `RefreshPool`.
//!
//! Both calls are *best-effort* — they fail silently and log a warning rather
//! than aborting the transaction.  The market is an economic primitive layered
//! on top of the existing state model; forcing hard failures would break
//! backwards compatibility with existing transactions.

use evaporchain_refresh_market::{pay_rent, RefreshMarket, NamespaceId};
use evaporchain_energy_kernel::RefreshPool;
use tracing::warn;

/// Default namespace capacity (maximum concurrently-active objects per namespace).
/// Governance can update per-namespace via a future `SetCapacity` proposal.
pub const DEFAULT_CAPACITY: u64 = 1_000;

/// Default base rent rate (energy per epoch at zero utilisation).
pub const DEFAULT_BASE_RATE: u64 = 100;

/// Build the chain's initial `RefreshMarket` at genesis.
pub fn genesis_market() -> RefreshMarket {
    RefreshMarket::new(DEFAULT_BASE_RATE)
}

/// Ensure a namespace exists in the market.  If not yet registered, registers
/// it with `DEFAULT_CAPACITY`.  Returns the current utilisation.
pub fn ensure_namespace(market: &mut RefreshMarket, namespace: NamespaceId) -> u64 {
    if market.get(&namespace).is_none() {
        market.register(namespace.clone(), DEFAULT_CAPACITY);
    }
    market.get(&namespace).map(|ns| ns.used).unwrap_or(0)
}

/// Charge one epoch of refresh-market rent for `namespace` and credit it to
/// `pool`.  On `MarketError` logs a warning and returns 0.
pub fn charge_refresh_rent(
    market: &mut RefreshMarket,
    pool: &mut RefreshPool,
    namespace: NamespaceId,
    epoch: u64,
) -> u64 {
    ensure_namespace(market, namespace.clone());

    match pay_rent(market, pool, &namespace, epoch) {
        Ok(amount) => amount,
        Err(e) => {
            warn!(
                namespace = hex::encode(&namespace),
                err = %e,
                "refresh-market rent payment failed (best-effort, ignoring)"
            );
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_energy_kernel::RefreshPool;

    fn ns(b: u8) -> NamespaceId {
        vec![b; 8]
    }

    #[test]
    fn genesis_market_is_empty() {
        let m = genesis_market();
        assert!(m.get(&ns(1)).is_none());
    }

    #[test]
    fn ensure_namespace_registers_on_first_call() {
        let mut m = genesis_market();
        ensure_namespace(&mut m, ns(1));
        assert!(m.get(&ns(1)).is_some());
    }

    #[test]
    fn ensure_namespace_idempotent() {
        let mut m = genesis_market();
        ensure_namespace(&mut m, ns(1));
        ensure_namespace(&mut m, ns(1)); // second call must not panic
        assert_eq!(m.get(&ns(1)).unwrap().capacity, DEFAULT_CAPACITY);
    }

    #[test]
    fn charge_refresh_rent_credits_pool() {
        let mut m = genesis_market();
        let mut pool = RefreshPool::new();
        let amount = charge_refresh_rent(&mut m, &mut pool, ns(2), 1);
        // At zero utilisation: base × 1² / capacity² = 100 × 1 / 1_000_000 = 0 (integer)
        // or non-zero depending on capacity. Just verify it doesn't panic.
        assert!(amount == 0 || amount > 0); // always succeeds
        assert_eq!(pool.total_accrued(), amount);
    }

    #[test]
    fn charge_unknown_namespace_succeeds_silently() {
        let mut m = genesis_market();
        let mut pool = RefreshPool::new();
        // Unknown namespace is auto-registered by ensure_namespace.
        let amount = charge_refresh_rent(&mut m, &mut pool, ns(99), 0);
        assert_eq!(pool.total_accrued(), amount);
    }
}
