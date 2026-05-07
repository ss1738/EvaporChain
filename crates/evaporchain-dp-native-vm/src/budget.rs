//! Privacy-budget tracker — the decay-native ε/δ reservoir.
//!
//! Two structural decisions:
//!
//! 1. **Fixed-point integers, not floats.** ε is tracked in *micros*
//!    (1 ε = 1_000_000 micros); δ is tracked in *ppb* (1 δ = 10⁹).
//!    Validators agree byte-for-byte; no f64 anywhere on the
//!    consensus path.
//!
//! 2. **Spend-only.** Once consumed, ε / δ cannot be returned. The
//!    budget is decay-native: same as any energy quantity on the
//!    chain. There is no "refund-on-error" path.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 32-byte chain-wide handle for a sensitive dataset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct DatasetId(pub [u8; 32]);

/// Privacy budget for one dataset. Tracked as fixed-point integers.
///
/// ε is in *micros*: 1 ε = 1_000_000 micros. So `epsilon_micros = 1_000_000`
/// means "ε = 1.0", a moderate budget. ε = 0.1 → 100_000 micros.
///
/// δ is in *ppb* (parts per billion): 10⁻⁹ = 1 ppb. δ = 10⁻⁶ → 1_000 ppb.
/// Cryptographic δ targets (10⁻⁹ to 10⁻¹²) fit comfortably in u64.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivacyBudget {
    pub initial_epsilon_micros: u64,
    pub initial_delta_ppb: u64,
    pub consumed_epsilon_micros: u64,
    pub consumed_delta_ppb: u64,
}

impl PrivacyBudget {
    pub fn new(initial_epsilon_micros: u64, initial_delta_ppb: u64) -> Self {
        Self {
            initial_epsilon_micros,
            initial_delta_ppb,
            consumed_epsilon_micros: 0,
            consumed_delta_ppb: 0,
        }
    }

    pub fn remaining_epsilon_micros(&self) -> u64 {
        self.initial_epsilon_micros
            .saturating_sub(self.consumed_epsilon_micros)
    }

    pub fn remaining_delta_ppb(&self) -> u64 {
        self.initial_delta_ppb
            .saturating_sub(self.consumed_delta_ppb)
    }

    /// True iff a query asking for `(ε_q, δ_q)` would be admissible
    /// under the current consumed totals (basic composition: sum).
    pub fn admits(&self, query_epsilon_micros: u64, query_delta_ppb: u64) -> bool {
        let new_eps = self
            .consumed_epsilon_micros
            .checked_add(query_epsilon_micros);
        let new_delta = self.consumed_delta_ppb.checked_add(query_delta_ppb);
        match (new_eps, new_delta) {
            (Some(e), Some(d)) => e <= self.initial_epsilon_micros && d <= self.initial_delta_ppb,
            _ => false,
        }
    }

    /// True iff the budget is fully exhausted on either axis.
    pub fn is_exhausted(&self) -> bool {
        self.remaining_epsilon_micros() == 0 || self.remaining_delta_ppb() == 0
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BudgetError {
    #[error("dataset {0:?} not registered")]
    UnknownDataset(DatasetId),
    #[error("dataset {0:?} already registered (re-registering is forbidden — budget is monotone)")]
    AlreadyRegistered(DatasetId),
    #[error(
        "query exceeds remaining budget: requested ε={req_eps_micros} micros / δ={req_delta_ppb} ppb, remaining ε={rem_eps_micros} / δ={rem_delta_ppb}"
    )]
    BudgetExhausted {
        req_eps_micros: u64,
        req_delta_ppb: u64,
        rem_eps_micros: u64,
        rem_delta_ppb: u64,
    },
    #[error("query parameters cause budget arithmetic overflow")]
    Overflow,
}

/// Per-chain registry of dataset budgets. The chain wraps this with
/// whatever durability layer it uses.
///
/// **Audit fix H2**: backed by `BTreeMap` (not `HashMap`) so serde
/// serialisation is canonical (key-sorted) — required for
/// validator-determinism when the registry is hashed into chain state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BudgetRegistry {
    budgets: BTreeMap<DatasetId, PrivacyBudget>,
}

impl BudgetRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.budgets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.budgets.is_empty()
    }

    pub fn get(&self, id: &DatasetId) -> Option<&PrivacyBudget> {
        self.budgets.get(id)
    }

    /// Register a new dataset with a fresh budget. Re-registering an
    /// existing dataset is forbidden — would erase the consumption
    /// history and break the monotone-spend invariant.
    pub fn register(
        &mut self,
        id: DatasetId,
        initial_epsilon_micros: u64,
        initial_delta_ppb: u64,
    ) -> Result<(), BudgetError> {
        if self.budgets.contains_key(&id) {
            return Err(BudgetError::AlreadyRegistered(id));
        }
        self.budgets.insert(
            id,
            PrivacyBudget::new(initial_epsilon_micros, initial_delta_ppb),
        );
        Ok(())
    }

    /// Reserve and consume budget for one query. **Atomic** —
    /// either the full `(ε_q, δ_q)` is consumed and the call
    /// succeeds, or nothing is consumed and the call fails.
    ///
    /// This is the central query gate. The chain calls this BEFORE
    /// running the actual query; if it returns `Ok`, the chain
    /// proceeds; if `Err`, the query is refused.
    pub fn consume(
        &mut self,
        id: DatasetId,
        query_epsilon_micros: u64,
        query_delta_ppb: u64,
    ) -> Result<(), BudgetError> {
        let b = self
            .budgets
            .get_mut(&id)
            .ok_or(BudgetError::UnknownDataset(id))?;
        if !b.admits(query_epsilon_micros, query_delta_ppb) {
            return Err(BudgetError::BudgetExhausted {
                req_eps_micros: query_epsilon_micros,
                req_delta_ppb: query_delta_ppb,
                rem_eps_micros: b.remaining_epsilon_micros(),
                rem_delta_ppb: b.remaining_delta_ppb(),
            });
        }
        // Safe to consume — overflow guarded by `admits`.
        b.consumed_epsilon_micros = b
            .consumed_epsilon_micros
            .checked_add(query_epsilon_micros)
            .ok_or(BudgetError::Overflow)?;
        b.consumed_delta_ppb = b
            .consumed_delta_ppb
            .checked_add(query_delta_ppb)
            .ok_or(BudgetError::Overflow)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ds(byte: u8) -> DatasetId {
        DatasetId([byte; 32])
    }

    #[test]
    fn fresh_budget_admits_within_initial() {
        let b = PrivacyBudget::new(1_000_000, 1000);
        assert!(b.admits(500_000, 500));
        assert!(b.admits(1_000_000, 1000)); // exact match
        assert!(!b.admits(1_000_001, 0));
    }

    #[test]
    fn consume_decrements_remaining() {
        let mut r = BudgetRegistry::new();
        r.register(ds(1), 1_000_000, 1000).unwrap();
        r.consume(ds(1), 300_000, 300).unwrap();
        let b = r.get(&ds(1)).unwrap();
        assert_eq!(b.remaining_epsilon_micros(), 700_000);
        assert_eq!(b.remaining_delta_ppb(), 700);
    }

    #[test]
    fn consume_atomic_under_partial_overflow() {
        // If ε is admissible but δ would overflow, the consume must
        // fail entirely, leaving ε state untouched. Test: register
        // with very tight δ; query that would overshoot δ.
        let mut r = BudgetRegistry::new();
        r.register(ds(1), 1_000_000, 100).unwrap();
        let err = r.consume(ds(1), 100_000, 200).unwrap_err();
        assert!(matches!(err, BudgetError::BudgetExhausted { .. }));
        // No consumption occurred.
        let b = r.get(&ds(1)).unwrap();
        assert_eq!(b.consumed_epsilon_micros, 0);
        assert_eq!(b.consumed_delta_ppb, 0);
    }

    #[test]
    fn budget_exhausts_then_fails_closed() {
        let mut r = BudgetRegistry::new();
        r.register(ds(1), 1_000_000, 1000).unwrap();
        r.consume(ds(1), 1_000_000, 1000).unwrap(); // exhaust
        let b = r.get(&ds(1)).unwrap();
        assert!(b.is_exhausted());
        // Any subsequent query — even ε=0 δ=0 — passes admits
        // (0 ≤ 0 + remaining=0) but a positive query fails.
        assert!(b.admits(0, 0));
        let err = r.consume(ds(1), 1, 0).unwrap_err();
        assert!(matches!(err, BudgetError::BudgetExhausted { .. }));
    }

    #[test]
    fn unknown_dataset_consume_fails() {
        let mut r = BudgetRegistry::new();
        let err = r.consume(ds(99), 100, 0).unwrap_err();
        assert_eq!(err, BudgetError::UnknownDataset(ds(99)));
    }

    #[test]
    fn duplicate_registration_rejected() {
        // Re-registering would erase the consumption history.
        let mut r = BudgetRegistry::new();
        r.register(ds(1), 1_000_000, 1000).unwrap();
        let err = r.register(ds(1), 5_000_000, 0).unwrap_err();
        assert_eq!(err, BudgetError::AlreadyRegistered(ds(1)));
    }

    #[test]
    fn many_small_queries_sum_via_basic_composition() {
        // 10 queries × ε=0.1 each = 1.0 total. Match the budget
        // exactly.
        let mut r = BudgetRegistry::new();
        r.register(ds(1), 1_000_000, 0).unwrap();
        for _ in 0..10 {
            r.consume(ds(1), 100_000, 0).unwrap();
        }
        let err = r.consume(ds(1), 1, 0).unwrap_err();
        assert!(matches!(err, BudgetError::BudgetExhausted { .. }));
        assert!(r.get(&ds(1)).unwrap().is_exhausted());
    }

    #[test]
    fn budget_value_round_trips_serde() {
        // The registry uses HashMap<DatasetId, _> where DatasetId is
        // [u8; 32]. JSON requires string-typed map keys, so the
        // registry itself doesn't round-trip through JSON; the chain
        // is expected to wrap it with binary durability (RocksDB
        // value-by-value, bincode, etc.). The PrivacyBudget VALUE
        // type round-trips fine.
        let b = PrivacyBudget {
            initial_epsilon_micros: 1_000_000,
            initial_delta_ppb: 1000,
            consumed_epsilon_micros: 250_000,
            consumed_delta_ppb: 250,
        };
        let s = serde_json::to_string(&b).unwrap();
        let back: PrivacyBudget = serde_json::from_str(&s).unwrap();
        assert_eq!(b, back);
    }
}
