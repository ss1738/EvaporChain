//! `RefreshMarket` — the book of namespaces and the operations against
//! the kernel's `RefreshPool`.
//!
//! Two flows:
//!
//! - [`reserve_slot`]: increment a namespace's `used` count after
//!   confirming the renter can pay one epoch of rent at the current
//!   AMM rate. Pay through `RefreshPool::payout` against the renter's
//!   own namespace credit.
//! - [`pay_rent`]: pay `n` epochs of rent for an already-reserved slot,
//!   without changing `used`. Same payout flow.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

use evaporchain_energy_kernel::{
    refresh_pool::RefreshPoolError, EnergyAccumulator, RefreshPool,
};
use evaporchain_types::Energy;

use crate::namespace::{Namespace, NamespaceId};
use crate::pricing::{rent_rate, PricingError};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RefreshMarket {
    namespaces: BTreeMap<NamespaceId, Namespace>,
    /// AMM base coefficient — chain-set governance parameter.
    pub base: Energy,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MarketError {
    #[error("namespace {0:?} is not registered")]
    UnknownNamespace(NamespaceId),
    #[error("namespace is at full capacity ({used}/{capacity})")]
    NoCapacity { used: u64, capacity: u64 },
    #[error("pricing error: {0}")]
    Pricing(#[from] PricingError),
    #[error("refresh-pool error: {0}")]
    Pool(#[from] RefreshPoolError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReservationOutcome {
    pub epochs_funded: u64,
    pub paid: Energy,
}

impl RefreshMarket {
    pub fn new(base: Energy) -> Self {
        Self {
            namespaces: BTreeMap::new(),
            base,
        }
    }

    pub fn register(&mut self, id: NamespaceId, capacity: u64) -> &mut Namespace {
        self.namespaces
            .entry(id.clone())
            .or_insert_with(|| Namespace::new(id, capacity))
    }

    pub fn get(&self, id: &NamespaceId) -> Option<&Namespace> {
        self.namespaces.get(id)
    }

    pub fn get_mut(&mut self, id: &NamespaceId) -> Option<&mut Namespace> {
        self.namespaces.get_mut(id)
    }
}

/// Reserve one slot in `namespace`, paying one epoch of rent at the
/// current AMM rate from `pool`'s credit ledger for that namespace.
/// Increments `namespace.used`.
///
/// Note: this draws against the namespace's *own* refresh-pool credit
/// (the operator pre-funded keep-alive). Per-renter funding is a
/// separate per-tx flow handled by the consumer.
pub fn reserve_slot(
    market: &mut RefreshMarket,
    pool: &mut RefreshPool,
    _acc: &mut EnergyAccumulator,
    namespace_id: &NamespaceId,
    epoch: u64,
) -> Result<ReservationOutcome, MarketError> {
    let base = market.base;
    let ns = market
        .get_mut(namespace_id)
        .ok_or_else(|| MarketError::UnknownNamespace(namespace_id.clone()))?;
    if ns.is_full() {
        return Err(MarketError::NoCapacity {
            used: ns.used,
            capacity: ns.capacity,
        });
    }
    let rate = rent_rate(ns.used, ns.capacity, base)?;
    pool.payout(namespace_id, rate, epoch)?;
    ns.used = ns.used.saturating_add(1);
    Ok(ReservationOutcome {
        epochs_funded: 1,
        paid: rate,
    })
}

/// Pay `epochs` × current rent rate from the namespace's pool credit,
/// without changing `used`. For renewing existing reservations.
pub fn pay_rent(
    market: &RefreshMarket,
    pool: &mut RefreshPool,
    _acc: &mut EnergyAccumulator,
    namespace_id: &NamespaceId,
    epochs: u64,
    epoch: u64,
) -> Result<ReservationOutcome, MarketError> {
    let ns = market
        .get(namespace_id)
        .ok_or_else(|| MarketError::UnknownNamespace(namespace_id.clone()))?;
    if epochs == 0 {
        return Ok(ReservationOutcome { epochs_funded: 0, paid: 0 });
    }
    let rate = rent_rate(ns.used, ns.capacity, market.base)?;
    let total = rate.saturating_mul(epochs);
    pool.payout(namespace_id, total, epoch)?;
    Ok(ReservationOutcome {
        epochs_funded: epochs,
        paid: total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns_id(b: u8) -> NamespaceId {
        vec![b; 4]
    }

    fn fresh_pool_with(ns: NamespaceId, amount: Energy) -> RefreshPool {
        let mut p = RefreshPool::new();
        p.accrue(ns, amount, 0);
        p
    }

    #[test]
    fn register_and_get() {
        let mut m = RefreshMarket::new(1_000);
        m.register(ns_id(1), 100);
        assert_eq!(m.get(&ns_id(1)).unwrap().capacity, 100);
    }

    #[test]
    fn reserve_increments_used_and_drains_credit() {
        let mut m = RefreshMarket::new(1_000);
        m.register(ns_id(1), 100);
        let mut pool = fresh_pool_with(ns_id(1), 1_000_000);
        let mut acc = EnergyAccumulator::default();
        let pre = pool.total_accrued();
        let out = reserve_slot(&mut m, &mut pool, &mut acc, &ns_id(1), 5).unwrap();
        assert_eq!(out.epochs_funded, 1);
        assert!(out.paid > 0);
        assert_eq!(m.get(&ns_id(1)).unwrap().used, 1);
        assert!(pool.total_accrued() < pre);
    }

    #[test]
    fn reserve_full_namespace_rejected() {
        let mut m = RefreshMarket::new(1_000);
        let n = m.register(ns_id(1), 1);
        n.used = 1;
        let mut pool = fresh_pool_with(ns_id(1), 1_000_000);
        let mut acc = EnergyAccumulator::default();
        let err = reserve_slot(&mut m, &mut pool, &mut acc, &ns_id(1), 5).unwrap_err();
        assert!(matches!(err, MarketError::NoCapacity { .. }));
    }

    #[test]
    fn reserve_unknown_namespace_rejected() {
        let mut m = RefreshMarket::new(1_000);
        let mut pool = RefreshPool::new();
        let mut acc = EnergyAccumulator::default();
        let err = reserve_slot(&mut m, &mut pool, &mut acc, &ns_id(99), 5).unwrap_err();
        assert!(matches!(err, MarketError::UnknownNamespace(_)));
    }

    #[test]
    fn reserve_with_insufficient_pool_credit_rejected() {
        let mut m = RefreshMarket::new(1_000_000);
        m.register(ns_id(1), 10); // base/cap² ≈ 10_000 per epoch (used=0 → 10_000/100=100? recompute)
        // base=1_000_000, cap=10, used=0 → rate = 1_000_000 × 1 / 100 = 10_000.
        let mut pool = fresh_pool_with(ns_id(1), 5_000); // < rate
        let mut acc = EnergyAccumulator::default();
        let err = reserve_slot(&mut m, &mut pool, &mut acc, &ns_id(1), 1).unwrap_err();
        assert!(matches!(err, MarketError::Pool(_)));
    }

    #[test]
    fn pay_rent_for_zero_epochs_no_op() {
        let mut m = RefreshMarket::new(1_000);
        m.register(ns_id(1), 100);
        let mut pool = fresh_pool_with(ns_id(1), 1_000_000);
        let mut acc = EnergyAccumulator::default();
        let out = pay_rent(&m, &mut pool, &mut acc, &ns_id(1), 0, 5).unwrap();
        assert_eq!(out.epochs_funded, 0);
        assert_eq!(out.paid, 0);
    }
}
