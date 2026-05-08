//! Refresh pool — protocol-owned bucket that funds keep-alive of namespace
//! roots (chain history, beacon, light-cone proofs) per INVENTION_STACK.md
//! §1.2.
//!
//! Inflows: `RedirectKind::SlashSettle` + `RedirectKind::MevBurn` +
//! `RedirectKind::Demurrage`. Outflows: `RedirectKind::RefreshPayout`.
//!
//! This module owns the *credit ledger* — who can claim against the pool
//! and for how much. The actual energy bytes live in
//! `EnergyAccumulator[RefreshPool]`; the pool here mirrors that with a
//! per-namespace claim ledger so we can distribute payouts deterministically.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

use evaporchain_types::Energy;

/// A namespace identifier — opaque to the kernel. Production will use the
/// 32-byte namespace IDs from `evaporchain-da`; the kernel keeps it as a
/// generic `Vec<u8>` so we don't introduce a cross-crate cycle. The
/// pool only ever compares-by-equality.
pub type NamespaceId = Vec<u8>;

/// A claim against the refresh pool — one namespace root's accrued
/// keep-alive entitlement, decaying like everything else when not paid out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshCredit {
    pub namespace: NamespaceId,
    pub accrued: Energy,
    /// The epoch the credit was last touched (for λ-scoped decay of the
    /// claim itself — kernel exposes this; consumers apply `energy_at_epoch`).
    pub last_touched_epoch: u64,
}

/// In-memory refresh pool ledger.
///
/// Total `accrued` across all credits should track the
/// `EnergyAccumulator[RefreshPool]` bucket up to the conservation
/// invariant — the pool is a credit-ledger view of the same bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshPool {
    credits: BTreeMap<NamespaceId, RefreshCredit>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RefreshPoolError {
    #[error("namespace {0:?} has no credit ledger entry")]
    UnknownNamespace(NamespaceId),
    #[error("requested {requested} from namespace {namespace:?} but only {available} accrued")]
    InsufficientCredit {
        namespace: NamespaceId,
        requested: Energy,
        available: Energy,
    },
}

impl RefreshPool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Increase a namespace's accrued credit. If the namespace is new, it
    /// is created at this epoch. Idempotent on amount=0.
    pub fn accrue(&mut self, namespace: NamespaceId, amount: Energy, epoch: u64) {
        if amount == 0 {
            return;
        }
        let entry = self
            .credits
            .entry(namespace.clone())
            .or_insert(RefreshCredit {
                namespace,
                accrued: 0,
                last_touched_epoch: epoch,
            });
        entry.accrued = entry.accrued.saturating_add(amount);
        entry.last_touched_epoch = epoch;
    }

    /// Pay out (and remove) up to `amount` from a namespace's credits.
    /// Returns the actual amount paid (capped at the available balance).
    /// If the namespace has no entry, returns `Err`.
    pub fn payout(
        &mut self,
        namespace: &NamespaceId,
        amount: Energy,
        epoch: u64,
    ) -> Result<Energy, RefreshPoolError> {
        let entry = self
            .credits
            .get_mut(namespace)
            .ok_or_else(|| RefreshPoolError::UnknownNamespace(namespace.clone()))?;
        if entry.accrued < amount {
            return Err(RefreshPoolError::InsufficientCredit {
                namespace: namespace.clone(),
                requested: amount,
                available: entry.accrued,
            });
        }
        entry.accrued -= amount;
        entry.last_touched_epoch = epoch;
        // Garbage-collect zeroed entries so an iterator over all-active
        // namespaces stays tight.
        if entry.accrued == 0 {
            self.credits.remove(namespace);
        }
        Ok(amount)
    }

    /// Total accrued across every namespace. Should track
    /// `EnergyAccumulator[RefreshPool]`.
    pub fn total_accrued(&self) -> Energy {
        self.credits
            .values()
            .map(|c| c.accrued)
            .fold(0u64, |a, b| a.saturating_add(b))
    }

    /// Total accrued for a single namespace, or 0 if the namespace has
    /// no credit entry. Used by the `/api/four_act` surface to expose
    /// per-namespace accruals (e.g. dead-producer redirect vs.
    /// rent-exhaustion residual) independently.
    pub fn accrued_for(&self, namespace: &NamespaceId) -> Energy {
        self.credits.get(namespace).map(|c| c.accrued).unwrap_or(0)
    }

    /// Read-only iteration for audit / payout-scheduling consumers.
    pub fn credits(&self) -> impl Iterator<Item = &RefreshCredit> {
        self.credits.values()
    }

    pub fn is_empty(&self) -> bool {
        self.credits.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns(b: u8) -> NamespaceId {
        vec![b; 4]
    }

    #[test]
    fn accrue_creates_entry() {
        let mut p = RefreshPool::new();
        p.accrue(ns(1), 100, 5);
        assert_eq!(p.total_accrued(), 100);
        let c = p.credits().next().unwrap();
        assert_eq!(c.accrued, 100);
        assert_eq!(c.last_touched_epoch, 5);
    }

    #[test]
    fn accrue_zero_is_noop() {
        let mut p = RefreshPool::new();
        p.accrue(ns(1), 0, 5);
        assert!(p.is_empty());
    }

    #[test]
    fn accrued_for_returns_per_namespace_total() {
        // Two namespaces — accrued_for must isolate them (used by
        // /api/four_act to report the dead-producer-redirect slice
        // separately from the rent-exhaustion slice, even though
        // both flow into the same RefreshPool aggregate).
        let mut p = RefreshPool::new();
        p.accrue(ns(1), 100, 0);
        p.accrue(ns(2), 250, 0);

        assert_eq!(p.accrued_for(&ns(1)), 100);
        assert_eq!(p.accrued_for(&ns(2)), 250);
        assert_eq!(p.accrued_for(&ns(99)), 0, "unknown namespace must return 0");
        assert_eq!(p.total_accrued(), 350);
    }

    #[test]
    fn second_accrue_sums_and_updates_epoch() {
        let mut p = RefreshPool::new();
        p.accrue(ns(1), 100, 5);
        p.accrue(ns(1), 50, 9);
        let c = p.credits().next().unwrap();
        assert_eq!(c.accrued, 150);
        assert_eq!(c.last_touched_epoch, 9);
    }

    #[test]
    fn payout_reduces_accrued() {
        let mut p = RefreshPool::new();
        p.accrue(ns(1), 100, 5);
        let paid = p.payout(&ns(1), 30, 7).unwrap();
        assert_eq!(paid, 30);
        assert_eq!(p.total_accrued(), 70);
    }

    #[test]
    fn payout_to_zero_garbage_collects() {
        let mut p = RefreshPool::new();
        p.accrue(ns(1), 100, 5);
        p.payout(&ns(1), 100, 7).unwrap();
        assert!(p.is_empty());
    }

    #[test]
    fn payout_missing_namespace_errs() {
        let mut p = RefreshPool::new();
        let err = p.payout(&ns(99), 1, 0).unwrap_err();
        assert!(matches!(err, RefreshPoolError::UnknownNamespace(_)));
    }

    #[test]
    fn payout_insufficient_credit_errs_and_state_unchanged() {
        let mut p = RefreshPool::new();
        p.accrue(ns(1), 50, 5);
        let err = p.payout(&ns(1), 100, 7).unwrap_err();
        assert!(matches!(err, RefreshPoolError::InsufficientCredit { .. }));
        assert_eq!(p.total_accrued(), 50);
    }
}
