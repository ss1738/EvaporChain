//! Payout settlement.
//!
//! Given a pool, an advertiser's `(max_cpm, freshness_floor)` bid,
//! and an attestation `epoch_now`, computes the total earnable
//! energy across all AQs in the pool whose decayed value at
//! `epoch_now` ≥ `freshness_floor`.
//!
//! The advertiser's `max_cpm` (cost per mille — pool currency unit
//! per 1000 AQs) caps the *aggregate* settlement. SAP's role is to
//! filter AQs by freshness and report the earnable total; the
//! actual transfer of energy happens at a higher transaction layer.
//!
//! **The doctrine clearing claim** ("a 5-min-old AQ is worth more
//! than a 25-min one") falls out of the math automatically: each
//! AQ's payout-eligible value is its current decayed value. We
//! report the per-AQ values plus the aggregate.

use evaporchain_types::{AccountAddress, Energy, Epoch};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::pool::AttentionPool;
use crate::quantum::AqId;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PayoutError {
    #[error("max_cpm must be > 0")]
    ZeroMaxCpm,
    #[error("freshness_floor must be > 0 (else every AQ pays out)")]
    ZeroFreshnessFloor,
}

/// Per-AQ payout entry — what the wallet/explorer renders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayoutEntry {
    pub aq_id: AqId,
    pub holder: AccountAddress,
    pub decayed_value: Energy,
}

/// Aggregate result of a payout query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayoutSummary {
    pub epoch: Epoch,
    pub max_cpm: Energy,
    pub freshness_floor: Energy,
    pub eligible: Vec<PayoutEntry>,
    /// Sum of `decayed_value` across `eligible`. Caller-side
    /// settlement uses this to compute the final payout — capped at
    /// `max_cpm × eligible.len() / 1000`.
    pub total_earnable: Energy,
}

/// Compute the eligible AQs and their payouts for the given bid at
/// `epoch_now`. Pure; deterministic; integer-only.
pub fn payout_for(
    pool: &AttentionPool,
    max_cpm: Energy,
    freshness_floor: Energy,
    epoch_now: Epoch,
) -> Result<PayoutSummary, PayoutError> {
    if max_cpm == 0 {
        return Err(PayoutError::ZeroMaxCpm);
    }
    if freshness_floor == 0 {
        return Err(PayoutError::ZeroFreshnessFloor);
    }
    let mut eligible: Vec<PayoutEntry> = Vec::new();
    let mut total: u128 = 0;
    // Iterate in canonical order — sorted by AQ id — so validators
    // agree on the exact entry list (HashMap iteration would be
    // non-deterministic across processes/builds).
    let mut ids: Vec<&AqId> = pool.quanta.keys().collect();
    ids.sort();
    for id in ids {
        let aq = pool.quanta.get(id).expect("just iterated");
        let value = aq.value_at(epoch_now);
        if value >= freshness_floor {
            total = total.saturating_add(value as u128);
            eligible.push(PayoutEntry {
                aq_id: aq.id,
                holder: aq.holder,
                decayed_value: value,
            });
        }
    }
    let total_earnable = total.min(Energy::MAX as u128) as Energy;
    Ok(PayoutSummary {
        epoch: epoch_now,
        max_cpm,
        freshness_floor,
        eligible,
        total_earnable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> AqId {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    fn addr(b: u8) -> AccountAddress {
        id(b)
    }

    fn pool_with_aqs() -> AttentionPool {
        let mut pool = AttentionPool::new(10, 60).unwrap();
        pool.mint_aq(id(1), addr(0xAA), 1000, 45, 0).unwrap();
        pool.mint_aq(id(2), addr(0xBB), 1000, 45, 0).unwrap();
        pool
    }

    #[test]
    fn rejects_zero_max_cpm() {
        let pool = pool_with_aqs();
        let err = payout_for(&pool, 0, 100, 5).unwrap_err();
        assert_eq!(err, PayoutError::ZeroMaxCpm);
    }

    #[test]
    fn rejects_zero_freshness_floor() {
        let pool = pool_with_aqs();
        let err = payout_for(&pool, 100, 0, 5).unwrap_err();
        assert_eq!(err, PayoutError::ZeroFreshnessFloor);
    }

    #[test]
    fn fresh_aqs_above_floor_are_eligible() {
        let pool = pool_with_aqs();
        // At epoch 0, both AQs are at full value 1000. Floor 500 is
        // below — both eligible.
        let summary = payout_for(&pool, 1000, 500, 0).unwrap();
        assert_eq!(summary.eligible.len(), 2);
        assert_eq!(summary.total_earnable, 2000);
    }

    #[test]
    fn stale_aqs_below_floor_are_filtered() {
        // After many half-lives, AQs decay below any positive floor.
        let pool = pool_with_aqs();
        let summary = payout_for(&pool, 1000, 500, 10_000).unwrap();
        assert!(summary.eligible.is_empty());
        assert_eq!(summary.total_earnable, 0);
    }

    #[test]
    fn five_minute_aq_pays_more_than_25_minute() {
        // Doctrine claim: "A 5-minute-old AQ is worth more than a
        // 25-min one." Two AQs minted at epochs 0 and 0; query at
        // 5 vs at 25.
        let pool = pool_with_aqs();
        let early = payout_for(&pool, 1000, 1, 5).unwrap();
        let late = payout_for(&pool, 1000, 1, 25).unwrap();
        assert!(
            early.total_earnable > late.total_earnable,
            "early total {} should be > late {}",
            early.total_earnable,
            late.total_earnable
        );
    }

    #[test]
    fn payout_summary_is_deterministic_across_runs() {
        // HashMap iteration is non-deterministic, but `payout_for`
        // sorts AQ ids canonically. Two queries with identical inputs
        // must produce byte-identical summaries.
        let pool = pool_with_aqs();
        let s1 = payout_for(&pool, 1000, 100, 5).unwrap();
        let s2 = payout_for(&pool, 1000, 100, 5).unwrap();
        assert_eq!(s1, s2);
        let bytes1 = serde_json::to_vec(&s1).unwrap();
        let bytes2 = serde_json::to_vec(&s2).unwrap();
        assert_eq!(bytes1, bytes2);
    }

    #[test]
    fn freshness_floor_partitions_eligible_set() {
        // High floor admits only fresh AQs; low floor admits the rest.
        let pool = pool_with_aqs();
        let high = payout_for(&pool, 1000, 800, 0).unwrap();
        let low = payout_for(&pool, 1000, 100, 0).unwrap();
        // Both AQs at epoch 0 are at full value 1000 → both pass any
        // floor up to 1000. Test the floor ordering instead: query
        // mid-decay where the floors actually partition.
        let high_mid = payout_for(&pool, 1000, 800, 50).unwrap();
        let low_mid = payout_for(&pool, 1000, 100, 50).unwrap();
        assert!(low_mid.eligible.len() >= high_mid.eligible.len());
        // At epoch 0, both are full value so both floors admit the same set.
        assert_eq!(high.eligible.len(), low.eligible.len());
    }

    #[test]
    fn total_earnable_is_sum_of_eligible_values() {
        // Pure arithmetic check: aggregate equals sum of per-entry
        // decayed values.
        let mut pool = AttentionPool::new(10, 60).unwrap();
        pool.mint_aq(id(1), addr(0xAA), 1000, 1_000_000, 0).unwrap();
        pool.mint_aq(id(2), addr(0xBB), 500, 1_000_000, 0).unwrap();
        let summary = payout_for(&pool, 100, 1, 0).unwrap();
        let sum: u64 = summary.eligible.iter().map(|e| e.decayed_value).sum();
        assert_eq!(sum, summary.total_earnable);
    }

    #[test]
    fn round_trip_serde() {
        let pool = pool_with_aqs();
        let summary = payout_for(&pool, 1000, 100, 5).unwrap();
        let s = serde_json::to_string(&summary).unwrap();
        let back: PayoutSummary = serde_json::from_str(&s).unwrap();
        assert_eq!(summary, back);
    }
}
