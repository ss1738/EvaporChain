//! `SinghPool` — the decay-aware xy=k pool.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::share::{HolderId, LpShare};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PoolError {
    #[error("pool empty — initial mint required first")]
    Empty,
    #[error("amount must be > 0")]
    ZeroAmount,
    #[error("not enough output liquidity for swap (k-invariant would be violated)")]
    InsufficientOutput,
    #[error("holder {0:?} has no shares")]
    NoShares(HolderId),
    #[error("holder {0:?} share energy below floor — cannot withdraw")]
    EnergyBelowFloor(HolderId),
    #[error("withdraw exceeds holder's share balance")]
    WithdrawExceedsBalance,
    #[error("arithmetic overflow")]
    Overflow,
    #[error("fee_bp must be ≤ 10_000")]
    InvalidFee,
}

/// xy=k constant-product AMM with energy-tagged LP shares.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SinghPool {
    /// Reserve of token X.
    reserve_x: u128,
    /// Reserve of token Y.
    reserve_y: u128,
    /// Total LP shares outstanding.
    total_shares: u128,
    /// Per-holder LP records.
    shares: HashMap<HolderId, LpShare>,
    /// Trading fee in basis points (1 bp = 0.01%).
    pub fee_bp: u64,
    /// Energy floor for withdrawals. Holders with energy ≤ floor
    /// cannot withdraw. Set to 0 to disable the gate (legacy
    /// AMM behaviour).
    pub energy_floor: u64,
}

impl SinghPool {
    pub fn new(fee_bp: u64, energy_floor: u64) -> Result<Self, PoolError> {
        if fee_bp > 10_000 {
            return Err(PoolError::InvalidFee);
        }
        Ok(Self {
            reserve_x: 0,
            reserve_y: 0,
            total_shares: 0,
            shares: HashMap::new(),
            fee_bp,
            energy_floor,
        })
    }

    pub fn reserve_x(&self) -> u128 {
        self.reserve_x
    }
    pub fn reserve_y(&self) -> u128 {
        self.reserve_y
    }
    pub fn total_shares(&self) -> u128 {
        self.total_shares
    }
    pub fn k(&self) -> u128 {
        self.reserve_x.saturating_mul(self.reserve_y)
    }

    pub fn shares_of(&self, h: HolderId) -> u128 {
        self.shares.get(&h).map(|s| s.shares).unwrap_or(0)
    }

    pub fn share_record(&self, h: HolderId) -> Option<&LpShare> {
        self.shares.get(&h)
    }

    /// Initial mint: bootstrap the pool. Sets the holder's shares
    /// to `sqrt(amount_x · amount_y)` (rounded down — standard
    /// Uniswap-v2 convention).
    pub fn mint_initial(
        &mut self,
        holder: HolderId,
        amount_x: u128,
        amount_y: u128,
        anchor_energy: u64,
        epoch: u64,
    ) -> Result<u128, PoolError> {
        if amount_x == 0 || amount_y == 0 {
            return Err(PoolError::ZeroAmount);
        }
        if self.total_shares != 0 {
            // Subsequent mints use proportional formula.
            return self.mint_proportional(holder, amount_x, amount_y, anchor_energy, epoch);
        }
        let initial = isqrt_u128(amount_x.checked_mul(amount_y).ok_or(PoolError::Overflow)?);
        self.reserve_x = amount_x;
        self.reserve_y = amount_y;
        self.total_shares = initial;
        self.shares
            .insert(holder, LpShare::new(holder, initial, anchor_energy, epoch));
        Ok(initial)
    }

    /// Proportional mint after the pool is bootstrapped.
    pub fn mint_proportional(
        &mut self,
        holder: HolderId,
        amount_x: u128,
        amount_y: u128,
        anchor_energy: u64,
        epoch: u64,
    ) -> Result<u128, PoolError> {
        if amount_x == 0 || amount_y == 0 {
            return Err(PoolError::ZeroAmount);
        }
        if self.total_shares == 0 || self.reserve_x == 0 || self.reserve_y == 0 {
            return Err(PoolError::Empty);
        }
        // Take the lesser proportion to enforce balance.
        let shares_x = amount_x
            .checked_mul(self.total_shares)
            .ok_or(PoolError::Overflow)?
            / self.reserve_x;
        let shares_y = amount_y
            .checked_mul(self.total_shares)
            .ok_or(PoolError::Overflow)?
            / self.reserve_y;
        let new_shares = shares_x.min(shares_y);
        if new_shares == 0 {
            return Err(PoolError::ZeroAmount);
        }
        self.reserve_x = self
            .reserve_x
            .checked_add(amount_x)
            .ok_or(PoolError::Overflow)?;
        self.reserve_y = self
            .reserve_y
            .checked_add(amount_y)
            .ok_or(PoolError::Overflow)?;
        self.total_shares = self
            .total_shares
            .checked_add(new_shares)
            .ok_or(PoolError::Overflow)?;
        let entry = self
            .shares
            .entry(holder)
            .or_insert_with(|| LpShare::new(holder, 0, 0, epoch));
        entry.shares = entry
            .shares
            .checked_add(new_shares)
            .ok_or(PoolError::Overflow)?;
        // Re-anchor: minting refreshes the energy tag.
        entry.energy = anchor_energy;
        entry.anchored_at_epoch = epoch;
        Ok(new_shares)
    }

    /// Decay a holder's share energy. Called by the chain's tick.
    pub fn decay_holder(&mut self, holder: HolderId, new_energy: u64) -> Result<(), PoolError> {
        let entry = self
            .shares
            .get_mut(&holder)
            .ok_or(PoolError::NoShares(holder))?;
        entry.energy = new_energy;
        Ok(())
    }

    /// Withdraw — burn shares for proportional reserves. Energy
    /// floor MUST be honoured: holders below floor cannot
    /// withdraw, even if they have positive shares.
    pub fn withdraw(
        &mut self,
        holder: HolderId,
        shares_to_burn: u128,
    ) -> Result<(u128, u128), PoolError> {
        if shares_to_burn == 0 {
            return Err(PoolError::ZeroAmount);
        }
        let entry = self
            .shares
            .get_mut(&holder)
            .ok_or(PoolError::NoShares(holder))?;
        if entry.energy <= self.energy_floor {
            return Err(PoolError::EnergyBelowFloor(holder));
        }
        if shares_to_burn > entry.shares {
            return Err(PoolError::WithdrawExceedsBalance);
        }
        // Proportional withdrawal: holder gets (shares/total) of each reserve.
        let out_x = self
            .reserve_x
            .checked_mul(shares_to_burn)
            .ok_or(PoolError::Overflow)?
            / self.total_shares;
        let out_y = self
            .reserve_y
            .checked_mul(shares_to_burn)
            .ok_or(PoolError::Overflow)?
            / self.total_shares;
        if out_x == 0 || out_y == 0 {
            return Err(PoolError::ZeroAmount);
        }
        entry.shares -= shares_to_burn;
        if entry.shares == 0 {
            self.shares.remove(&holder);
        }
        self.total_shares -= shares_to_burn;
        self.reserve_x -= out_x;
        self.reserve_y -= out_y;
        Ok((out_x, out_y))
    }

    /// Re-anchor: top up a holder's energy back to `anchor_energy`.
    /// In production, this is what providers must do periodically
    /// to keep their LP shares withdraw-able.
    pub fn reanchor(
        &mut self,
        holder: HolderId,
        anchor_energy: u64,
        epoch: u64,
    ) -> Result<(), PoolError> {
        let entry = self
            .shares
            .get_mut(&holder)
            .ok_or(PoolError::NoShares(holder))?;
        entry.energy = anchor_energy;
        entry.anchored_at_epoch = epoch;
        Ok(())
    }

    /// Swap `amount_in` of X for Y. Returns the Y output. Applies
    /// `fee_bp` on the input side (Uniswap-v2 convention).
    pub fn swap_x_for_y(&mut self, amount_in: u128) -> Result<u128, PoolError> {
        if amount_in == 0 {
            return Err(PoolError::ZeroAmount);
        }
        if self.reserve_x == 0 || self.reserve_y == 0 {
            return Err(PoolError::Empty);
        }
        let amount_in_after_fee = amount_in
            .checked_mul(10_000u128 - self.fee_bp as u128)
            .ok_or(PoolError::Overflow)?
            / 10_000;
        if amount_in_after_fee == 0 {
            return Err(PoolError::ZeroAmount);
        }
        // y_out = y · Δx / (x + Δx)
        let new_x = self
            .reserve_x
            .checked_add(amount_in_after_fee)
            .ok_or(PoolError::Overflow)?;
        let y_out = self
            .reserve_y
            .checked_mul(amount_in_after_fee)
            .ok_or(PoolError::Overflow)?
            / new_x;
        if y_out == 0 || y_out >= self.reserve_y {
            return Err(PoolError::InsufficientOutput);
        }
        // Apply: X gets the FULL amount_in (fee accrues to LPs);
        // Y reserve drops by y_out.
        self.reserve_x = self
            .reserve_x
            .checked_add(amount_in)
            .ok_or(PoolError::Overflow)?;
        self.reserve_y -= y_out;
        Ok(y_out)
    }

    /// Symmetric swap Y → X.
    pub fn swap_y_for_x(&mut self, amount_in: u128) -> Result<u128, PoolError> {
        if amount_in == 0 {
            return Err(PoolError::ZeroAmount);
        }
        if self.reserve_x == 0 || self.reserve_y == 0 {
            return Err(PoolError::Empty);
        }
        let amount_in_after_fee = amount_in
            .checked_mul(10_000u128 - self.fee_bp as u128)
            .ok_or(PoolError::Overflow)?
            / 10_000;
        if amount_in_after_fee == 0 {
            return Err(PoolError::ZeroAmount);
        }
        let new_y = self
            .reserve_y
            .checked_add(amount_in_after_fee)
            .ok_or(PoolError::Overflow)?;
        let x_out = self
            .reserve_x
            .checked_mul(amount_in_after_fee)
            .ok_or(PoolError::Overflow)?
            / new_y;
        if x_out == 0 || x_out >= self.reserve_x {
            return Err(PoolError::InsufficientOutput);
        }
        self.reserve_y = self
            .reserve_y
            .checked_add(amount_in)
            .ok_or(PoolError::Overflow)?;
        self.reserve_x -= x_out;
        Ok(x_out)
    }

    /// Sum of all per-holder shares — for the invariant test.
    pub fn sum_holder_shares(&self) -> u128 {
        self.shares.values().map(|s| s.shares).sum()
    }
}

/// Integer square root of a u128 — Newton's method.
fn isqrt_u128(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(b: u8) -> HolderId {
        HolderId([b; 32])
    }

    fn fresh_pool(fee_bp: u64, floor: u64) -> SinghPool {
        SinghPool::new(fee_bp, floor).unwrap()
    }

    fn bootstrap(p: &mut SinghPool, holder: HolderId, x: u128, y: u128, energy: u64) -> u128 {
        p.mint_initial(holder, x, y, energy, 0).unwrap()
    }

    // ── isqrt sanity ─────────────────────────────────────────────

    #[test]
    fn isqrt_sanity() {
        assert_eq!(isqrt_u128(0), 0);
        assert_eq!(isqrt_u128(1), 1);
        assert_eq!(isqrt_u128(4), 2);
        assert_eq!(isqrt_u128(9), 3);
        assert_eq!(isqrt_u128(99), 9);
        assert_eq!(isqrt_u128(100), 10);
        assert_eq!(isqrt_u128(1_000_000), 1000);
    }

    // ── mint ─────────────────────────────────────────────────────

    #[test]
    fn initial_mint_sets_reserves_and_shares() {
        let mut p = fresh_pool(30, 0);
        let s = bootstrap(&mut p, h(1), 1_000_000, 4_000_000, 1000);
        assert_eq!(p.reserve_x(), 1_000_000);
        assert_eq!(p.reserve_y(), 4_000_000);
        assert_eq!(s, isqrt_u128(4_000_000_000_000));
        assert_eq!(p.shares_of(h(1)), s);
        assert_eq!(p.total_shares(), s);
    }

    #[test]
    fn proportional_mint_extends_reserves() {
        let mut p = fresh_pool(30, 0);
        bootstrap(&mut p, h(1), 1_000_000, 4_000_000, 1000);
        // Add same ratio.
        let s2 = p
            .mint_proportional(h(2), 500_000, 2_000_000, 1000, 1)
            .unwrap();
        assert_eq!(p.reserve_x(), 1_500_000);
        assert_eq!(p.reserve_y(), 6_000_000);
        assert_eq!(p.shares_of(h(2)), s2);
        // h(1)'s shares unchanged.
        assert!(p.shares_of(h(1)) > 0);
    }

    #[test]
    fn proportional_mint_takes_lesser_ratio() {
        // Provider over-deposits one side; only the lesser ratio
        // earns shares (the surplus contributes to reserves but
        // not to share count — same as Uniswap).
        let mut p = fresh_pool(30, 0);
        bootstrap(&mut p, h(1), 1_000_000, 4_000_000, 1000);
        // Ratio mismatch: x is 5% but y is 10%.
        let s2 = p.mint_proportional(h(2), 50_000, 400_000, 1000, 1).unwrap();
        // shares = min(50_000·initial/1M, 400_000·initial/4M).
        // initial = isqrt(1M·4M) = 2_000_000.
        // shares_x = 50_000 · 2M / 1M = 100_000.
        // shares_y = 400_000 · 2M / 4M = 200_000.
        // min = 100_000.
        // total after = 2_000_000 + 100_000 = 2_100_000.
        // Reserves after: x = 1_050_000 (full deposit goes in even though
        //  the share count uses the lesser ratio); y = 4_400_000.
        assert_eq!(s2, 100_000);
        assert_eq!(p.total_shares(), 2_100_000);
        assert_eq!(p.reserve_x(), 1_050_000);
        assert_eq!(p.reserve_y(), 4_400_000);
    }

    // ── invariant: total = sum of holders ─────────────────────────

    #[test]
    fn total_shares_equals_sum_of_holders() {
        let mut p = fresh_pool(30, 0);
        bootstrap(&mut p, h(1), 1_000_000, 4_000_000, 1000);
        p.mint_proportional(h(2), 500_000, 2_000_000, 1000, 1)
            .unwrap();
        p.mint_proportional(h(3), 250_000, 1_000_000, 1000, 2)
            .unwrap();
        assert_eq!(p.total_shares(), p.sum_holder_shares());
    }

    // ── swap: k invariant ────────────────────────────────────────

    #[test]
    fn swap_preserves_or_grows_k() {
        let mut p = fresh_pool(30, 0); // 0.3% fee
        bootstrap(&mut p, h(1), 1_000_000, 4_000_000, 1000);
        let k_before = p.k();
        let _y_out = p.swap_x_for_y(10_000).unwrap();
        let k_after = p.k();
        assert!(
            k_after >= k_before,
            "k must not decrease: before={k_before}, after={k_after}"
        );
    }

    #[test]
    fn swap_x_for_y_returns_positive_output() {
        let mut p = fresh_pool(30, 0);
        bootstrap(&mut p, h(1), 1_000_000, 4_000_000, 1000);
        let y_out = p.swap_x_for_y(10_000).unwrap();
        assert!(y_out > 0);
    }

    #[test]
    fn swap_round_trip_loses_to_fees() {
        // X → Y → X gets back less than the original X (slippage + fees).
        let mut p = fresh_pool(30, 0);
        bootstrap(&mut p, h(1), 1_000_000, 4_000_000, 1000);
        let original_x = 10_000;
        let y_received = p.swap_x_for_y(original_x).unwrap();
        let x_back = p.swap_y_for_x(y_received).unwrap();
        assert!(x_back < original_x);
    }

    #[test]
    fn swap_zero_input_rejected() {
        let mut p = fresh_pool(30, 0);
        bootstrap(&mut p, h(1), 1_000_000, 4_000_000, 1000);
        assert_eq!(p.swap_x_for_y(0).unwrap_err(), PoolError::ZeroAmount);
    }

    #[test]
    fn swap_against_empty_pool_errors() {
        let mut p = fresh_pool(30, 0);
        let err = p.swap_x_for_y(100).unwrap_err();
        assert_eq!(err, PoolError::Empty);
    }

    // ── decay-floor withdraw gate (LOAD-BEARING) ──────────────────

    #[test]
    fn withdraw_above_floor_succeeds() {
        let mut p = fresh_pool(30, 100);
        bootstrap(&mut p, h(1), 1_000_000, 4_000_000, 1000);
        let s = p.shares_of(h(1));
        let (x, y) = p.withdraw(h(1), s / 2).unwrap();
        assert!(x > 0 && y > 0);
    }

    #[test]
    fn withdraw_at_or_below_floor_rejected() {
        let mut p = fresh_pool(30, 100);
        bootstrap(&mut p, h(1), 1_000_000, 4_000_000, 1000);
        let s = p.shares_of(h(1));
        // Decay holder to exactly the floor.
        p.decay_holder(h(1), 100).unwrap();
        let err = p.withdraw(h(1), s / 2).unwrap_err();
        assert_eq!(err, PoolError::EnergyBelowFloor(h(1)));
    }

    #[test]
    fn withdraw_below_floor_rejected() {
        let mut p = fresh_pool(30, 100);
        bootstrap(&mut p, h(1), 1_000_000, 4_000_000, 1000);
        let s = p.shares_of(h(1));
        p.decay_holder(h(1), 50).unwrap();
        assert!(p.withdraw(h(1), s / 2).is_err());
    }

    #[test]
    fn withdraw_above_balance_rejected() {
        let mut p = fresh_pool(30, 0);
        bootstrap(&mut p, h(1), 1_000_000, 4_000_000, 1000);
        let s = p.shares_of(h(1));
        let err = p.withdraw(h(1), s + 1).unwrap_err();
        assert_eq!(err, PoolError::WithdrawExceedsBalance);
    }

    #[test]
    fn reanchor_restores_withdraw() {
        let mut p = fresh_pool(30, 100);
        bootstrap(&mut p, h(1), 1_000_000, 4_000_000, 1000);
        p.decay_holder(h(1), 50).unwrap();
        // Below floor — withdraw fails.
        let s = p.shares_of(h(1));
        assert!(p.withdraw(h(1), s / 2).is_err());
        // Re-anchor restores energy.
        p.reanchor(h(1), 1000, 100).unwrap();
        let (x, y) = p.withdraw(h(1), s / 2).unwrap();
        assert!(x > 0 && y > 0);
    }

    // ── doctrine claim ────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Singh Pool eliminates mercenary capital. A
        // provider who deposits to farm subsidies and disappears
        // cannot withdraw because their LP shares' energy decays
        // below floor before they return. Re-anchoring restores
        // the gate — but requires the provider to be active on
        // the chain. Sticky liquidity is structurally cheaper
        // than mercenary."
        let mut p = fresh_pool(30, 100);

        // Provider seed bootstraps the pool (sticky).
        bootstrap(&mut p, h(0), 1_000_000, 4_000_000, 1000);

        // Mercenary deposits — high energy at deposit.
        let merc_shares = p
            .mint_proportional(h(1), 1_000_000, 4_000_000, 1000, 0)
            .unwrap();
        assert!(merc_shares > 0);

        // Time passes; mercenary disappeared. Chain's tick decays
        // their energy below floor.
        p.decay_holder(h(1), 50).unwrap();

        // Mercenary tries to extract: cannot.
        let err = p.withdraw(h(1), merc_shares).unwrap_err();
        assert_eq!(err, PoolError::EnergyBelowFloor(h(1)));

        // Sticky provider remained engaged — their re-anchor
        // routine kept their energy fresh.
        p.reanchor(h(0), 1000, 100).unwrap();
        let s0 = p.shares_of(h(0));
        let (x, y) = p.withdraw(h(0), s0 / 2).unwrap();
        assert!(x > 0 && y > 0);
    }

    proptest::proptest! {
        #[test]
        fn property_sum_holder_shares_equals_total(
            n in 1usize..10usize,
            seed in 1u64..1000u64,
        ) {
            // For any sequence of mints, sum of per-holder
            // shares equals total_shares.
            let mut p = fresh_pool(30, 0);
            let bx = 100_000 + seed * 1000;
            let by = 4 * bx;
            bootstrap(&mut p, h(0), bx as u128, by as u128, 1000);
            for i in 1..n {
                let dx = (50_000 + (seed.wrapping_mul(i as u64)) % 50_000) as u128;
                let dy = 4 * dx;
                p.mint_proportional(h(i as u8), dx, dy, 1000, i as u64).unwrap();
            }
            proptest::prop_assert_eq!(p.total_shares(), p.sum_holder_shares());
        }

        #[test]
        fn property_swap_never_decreases_k(
            seed in 1u128..100u128,
            fee_bp in 0u64..1_000u64,
        ) {
            let mut p = fresh_pool(fee_bp, 0);
            bootstrap(&mut p, h(0), 1_000_000, 4_000_000, 1000);
            let k_before = p.k();
            let _ = p.swap_x_for_y(seed * 100);
            let k_after = p.k();
            proptest::prop_assert!(k_after >= k_before);
        }
    }

    /// T1.20 — SinghPool::new rejects fee_bp > 10_000 (line 52).
    #[test]
    fn t1_20_pool_new_rejects_bad_fee() {
        let r = SinghPool::new(10_001, 0);
        assert!(matches!(r, Err(PoolError::InvalidFee)));

        let ok = SinghPool::new(0, 0);
        assert!(ok.is_ok());
        let ok_max = SinghPool::new(10_000, 0);
        assert!(ok_max.is_ok());
    }

    /// T1.20 — accessors: reserve_x, reserve_y, total_shares, k,
    /// shares_of, share_record. Previously the trivial getters
    /// weren't called from tests.
    #[test]
    fn t1_20_pool_accessors() {
        let mut pool = SinghPool::new(30, 0).unwrap();
        // Fresh pool — all zeros.
        assert_eq!(pool.reserve_x(), 0);
        assert_eq!(pool.reserve_y(), 0);
        assert_eq!(pool.total_shares(), 0);
        assert_eq!(pool.k(), 0);
        let holder = HolderId([1u8; 32]);
        assert_eq!(pool.shares_of(holder), 0);
        assert!(pool.share_record(holder).is_none());

        // After initial mint.
        let _ = pool.mint_initial(holder, 1_000, 1_000, 100, 1).unwrap();
        assert_eq!(pool.reserve_x(), 1_000);
        assert_eq!(pool.reserve_y(), 1_000);
        assert_eq!(pool.k(), 1_000_000);
        assert!(pool.shares_of(holder) > 0);
        assert!(pool.share_record(holder).is_some());
    }

    /// T1.20 — mint_initial rejects zero amounts (line 97).
    #[test]
    fn t1_20_mint_initial_rejects_zero() {
        let mut pool = SinghPool::new(30, 0).unwrap();
        let h = HolderId([2u8; 32]);
        assert!(matches!(
            pool.mint_initial(h, 0, 100, 100, 1),
            Err(PoolError::ZeroAmount)
        ));
        assert!(matches!(
            pool.mint_initial(h, 100, 0, 100, 1),
            Err(PoolError::ZeroAmount)
        ));
    }

    /// T1.20 — mint_initial called twice routes the second call
    /// through mint_proportional (line 101).
    #[test]
    fn t1_20_mint_initial_second_call_routes_to_proportional() {
        let mut pool = SinghPool::new(30, 0).unwrap();
        let h = HolderId([3u8; 32]);
        pool.mint_initial(h, 1_000, 1_000, 100, 1).unwrap();
        // Second mint_initial: pool is non-empty so it delegates.
        let r = pool.mint_initial(h, 100, 100, 100, 2);
        // Should succeed via proportional formula.
        assert!(r.is_ok(), "expected delegation to mint_proportional");
    }
}
