// wallet/src/liquidity_pool.rs — AMM liquidity pool tracker
//
// Track liquidity pools, LP positions, impermanent loss,
// and swap output estimation using constant-product AMM math.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ── Errors ───────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum LiquidityPoolError {
    #[error("pool already exists: {0}")]
    PoolAlreadyExists(String),
    #[error("pool not found: {0}")]
    PoolNotFound(String),
    #[error("position already exists: {0}")]
    PositionAlreadyExists(String),
    #[error("position not found: {0}")]
    PositionNotFound(String),
    #[error("position not active: {0}")]
    PositionNotActive(String),
    #[error("no pending rewards for position: {0}")]
    NoPendingRewards(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
}

// ── Enums ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PoolType {
    ConstantProduct,
    StableSwap,
    Concentrated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PositionStatus {
    Active,
    Withdrawn,
    Pending,
}

// ── Structs ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidityPool {
    pub id: String,
    pub token_a: String,
    pub token_b: String,
    pub pool_type: PoolType,
    pub reserve_a: u64,
    pub reserve_b: u64,
    pub total_lp_tokens: u64,
    pub fee_bps: u32,
    pub created_at: String,
    pub volume_24h: u64,
    pub apy_estimate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LpPosition {
    pub id: String,
    pub pool_id: String,
    pub lp_tokens: u64,
    pub deposited_a: u64,
    pub deposited_b: u64,
    pub deposit_time: String,
    pub status: PositionStatus,
    pub rewards_claimed: u64,
    pub pending_rewards: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpermanentLoss {
    pub pool_id: String,
    pub position_id: String,
    pub initial_value: f64,
    pub current_value: f64,
    pub hold_value: f64,
    pub il_percentage: f64,
    pub net_with_fees: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolStats {
    pub total_pools: usize,
    pub total_positions: usize,
    pub active_positions: usize,
    pub total_deposited_value: u64,
    pub total_rewards_claimed: u64,
    pub total_pending_rewards: u64,
}

// ── Manager ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LiquidityPoolManager {
    pub pools: HashMap<String, LiquidityPool>,
    pub positions: HashMap<String, LpPosition>,
}

impl LiquidityPoolManager {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Pool CRUD ────────────────────────────────────────────

    pub fn add_pool(&mut self, pool: LiquidityPool) -> Result<(), LiquidityPoolError> {
        if self.pools.contains_key(&pool.id) {
            return Err(LiquidityPoolError::PoolAlreadyExists(pool.id));
        }
        self.pools.insert(pool.id.clone(), pool);
        Ok(())
    }

    pub fn remove_pool(&mut self, id: &str) -> Result<LiquidityPool, LiquidityPoolError> {
        self.pools
            .remove(id)
            .ok_or_else(|| LiquidityPoolError::PoolNotFound(id.to_string()))
    }

    pub fn get_pool(&self, id: &str) -> Option<&LiquidityPool> {
        self.pools.get(id)
    }

    pub fn list_pools(&self) -> Vec<&LiquidityPool> {
        self.pools.values().collect()
    }

    // ── Position CRUD ────────────────────────────────────────

    pub fn add_position(&mut self, pos: LpPosition) -> Result<(), LiquidityPoolError> {
        if self.positions.contains_key(&pos.id) {
            return Err(LiquidityPoolError::PositionAlreadyExists(pos.id));
        }
        if !self.pools.contains_key(&pos.pool_id) {
            return Err(LiquidityPoolError::PoolNotFound(pos.pool_id));
        }
        self.positions.insert(pos.id.clone(), pos);
        Ok(())
    }

    pub fn withdraw_position(&mut self, id: &str) -> Result<&LpPosition, LiquidityPoolError> {
        let pos = self
            .positions
            .get_mut(id)
            .ok_or_else(|| LiquidityPoolError::PositionNotFound(id.to_string()))?;
        if pos.status != PositionStatus::Active {
            return Err(LiquidityPoolError::PositionNotActive(id.to_string()));
        }
        pos.status = PositionStatus::Withdrawn;
        Ok(pos)
    }

    pub fn get_position(&self, id: &str) -> Option<&LpPosition> {
        self.positions.get(id)
    }

    pub fn positions_for_pool(&self, pool_id: &str) -> Vec<&LpPosition> {
        self.positions
            .values()
            .filter(|p| p.pool_id == pool_id)
            .collect()
    }

    pub fn active_positions(&self) -> Vec<&LpPosition> {
        self.positions
            .values()
            .filter(|p| p.status == PositionStatus::Active)
            .collect()
    }

    // ── Rewards ──────────────────────────────────────────────

    pub fn claim_rewards(&mut self, position_id: &str) -> Result<u64, LiquidityPoolError> {
        let pos = self
            .positions
            .get_mut(position_id)
            .ok_or_else(|| LiquidityPoolError::PositionNotFound(position_id.to_string()))?;
        if pos.status != PositionStatus::Active {
            return Err(LiquidityPoolError::PositionNotActive(
                position_id.to_string(),
            ));
        }
        if pos.pending_rewards == 0 {
            return Err(LiquidityPoolError::NoPendingRewards(
                position_id.to_string(),
            ));
        }
        let claimed = pos.pending_rewards;
        pos.rewards_claimed += claimed;
        pos.pending_rewards = 0;
        Ok(claimed)
    }

    // ── IL Calculation ───────────────────────────────────────

    /// Calculate impermanent loss for a position given the current price ratio.
    ///
    /// Uses the standard AMM IL formula:
    ///   IL = 2 * sqrt(r) / (1 + r) - 1
    /// where r = current_price_ratio (price_now / price_at_deposit).
    pub fn calculate_il(
        &self,
        position_id: &str,
        current_price_ratio: f64,
    ) -> Result<ImpermanentLoss, LiquidityPoolError> {
        let pos = self
            .positions
            .get(position_id)
            .ok_or_else(|| LiquidityPoolError::PositionNotFound(position_id.to_string()))?;

        let initial_value = (pos.deposited_a + pos.deposited_b) as f64;

        // Standard AMM IL formula
        let r = current_price_ratio;
        let sqrt_r = r.sqrt();
        let il_ratio = 2.0 * sqrt_r / (1.0 + r) - 1.0;
        let il_percentage = il_ratio.abs() * 100.0;

        // Current LP value after IL
        let current_value = initial_value * (2.0 * sqrt_r / (1.0 + r));

        // Value if just held the original tokens
        // Token A stays same, Token B changes by price ratio
        let hold_value = pos.deposited_a as f64 + pos.deposited_b as f64 * current_price_ratio;

        // Net value including earned fees (rewards)
        let net_with_fees = current_value + (pos.rewards_claimed + pos.pending_rewards) as f64;

        Ok(ImpermanentLoss {
            pool_id: pos.pool_id.clone(),
            position_id: pos.id.clone(),
            initial_value,
            current_value,
            hold_value,
            il_percentage,
            net_with_fees,
        })
    }

    // ── Swap estimation ──────────────────────────────────────

    /// Estimate output using constant product formula: output = (input * (10000 - fee_bps) * reserve_out) / (reserve_in * 10000 + input * (10000 - fee_bps))
    pub fn estimate_output(
        &self,
        pool_id: &str,
        input_amount: u64,
        is_a_to_b: bool,
    ) -> Result<u64, LiquidityPoolError> {
        let pool = self
            .pools
            .get(pool_id)
            .ok_or_else(|| LiquidityPoolError::PoolNotFound(pool_id.to_string()))?;

        let (reserve_in, reserve_out) = if is_a_to_b {
            (pool.reserve_a, pool.reserve_b)
        } else {
            (pool.reserve_b, pool.reserve_a)
        };

        let fee_factor = 10_000u128 - pool.fee_bps as u128;
        let input = input_amount as u128;
        let r_in = reserve_in as u128;
        let r_out = reserve_out as u128;

        let numerator = input * fee_factor * r_out;
        let denominator = r_in * 10_000u128 + input * fee_factor;

        Ok((numerator / denominator) as u64)
    }

    // ── Analytics ─────────────────────────────────────────────

    pub fn pools_by_apy(&self) -> Vec<&LiquidityPool> {
        let mut pools: Vec<&LiquidityPool> = self.pools.values().collect();
        pools.sort_by(|a, b| {
            b.apy_estimate
                .partial_cmp(&a.apy_estimate)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        pools
    }

    pub fn total_value_locked(&self) -> u64 {
        self.pools
            .values()
            .map(|p| p.reserve_a + p.reserve_b)
            .sum()
    }

    pub fn stats(&self) -> PoolStats {
        let active = self
            .positions
            .values()
            .filter(|p| p.status == PositionStatus::Active)
            .count();
        let total_deposited: u64 = self
            .positions
            .values()
            .map(|p| p.deposited_a + p.deposited_b)
            .sum();
        let total_claimed: u64 = self.positions.values().map(|p| p.rewards_claimed).sum();
        let total_pending: u64 = self.positions.values().map(|p| p.pending_rewards).sum();

        PoolStats {
            total_pools: self.pools.len(),
            total_positions: self.positions.len(),
            active_positions: active,
            total_deposited_value: total_deposited,
            total_rewards_claimed: total_claimed,
            total_pending_rewards: total_pending,
        }
    }

    // ── Persistence ──────────────────────────────────────────

    pub fn save(&self, path: &Path) -> Result<(), LiquidityPoolError> {
        let json =
            serde_json::to_string_pretty(self).map_err(|e| LiquidityPoolError::Json(e.to_string()))?;
        std::fs::write(path, json).map_err(|e| LiquidityPoolError::Io(e.to_string()))
    }

    pub fn load(path: &Path) -> Result<Self, LiquidityPoolError> {
        let data =
            std::fs::read_to_string(path).map_err(|e| LiquidityPoolError::Io(e.to_string()))?;
        serde_json::from_str(&data).map_err(|e| LiquidityPoolError::Json(e.to_string()))
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_pool(id: &str, reserve_a: u64, reserve_b: u64, fee_bps: u32, apy: f64) -> LiquidityPool {
        LiquidityPool {
            id: id.to_string(),
            token_a: "EVAP".to_string(),
            token_b: "USDC".to_string(),
            pool_type: PoolType::ConstantProduct,
            reserve_a,
            reserve_b,
            total_lp_tokens: 1000,
            fee_bps,
            created_at: chrono::Utc::now().to_rfc3339(),
            volume_24h: 50000,
            apy_estimate: apy,
        }
    }

    fn make_position(id: &str, pool_id: &str, pending: u64) -> LpPosition {
        LpPosition {
            id: id.to_string(),
            pool_id: pool_id.to_string(),
            lp_tokens: 100,
            deposited_a: 500,
            deposited_b: 500,
            deposit_time: chrono::Utc::now().to_rfc3339(),
            status: PositionStatus::Active,
            rewards_claimed: 0,
            pending_rewards: pending,
        }
    }

    #[test]
    fn test_add_pool() {
        let mut mgr = LiquidityPoolManager::new();
        let pool = make_pool("pool-1", 10000, 10000, 30, 12.5);
        assert!(mgr.add_pool(pool).is_ok());
        assert!(mgr.get_pool("pool-1").is_some());
    }

    #[test]
    fn test_add_pool_duplicate_error() {
        let mut mgr = LiquidityPoolManager::new();
        mgr.add_pool(make_pool("pool-1", 10000, 10000, 30, 12.5))
            .unwrap();
        let result = mgr.add_pool(make_pool("pool-1", 5000, 5000, 20, 8.0));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            LiquidityPoolError::PoolAlreadyExists(_)
        ));
    }

    #[test]
    fn test_remove_pool() {
        let mut mgr = LiquidityPoolManager::new();
        mgr.add_pool(make_pool("pool-1", 10000, 10000, 30, 12.5))
            .unwrap();
        let removed = mgr.remove_pool("pool-1").unwrap();
        assert_eq!(removed.id, "pool-1");
        assert!(mgr.get_pool("pool-1").is_none());
    }

    #[test]
    fn test_remove_pool_not_found() {
        let mut mgr = LiquidityPoolManager::new();
        let result = mgr.remove_pool("nonexistent");
        assert!(matches!(
            result.unwrap_err(),
            LiquidityPoolError::PoolNotFound(_)
        ));
    }

    #[test]
    fn test_list_pools() {
        let mut mgr = LiquidityPoolManager::new();
        mgr.add_pool(make_pool("p1", 1000, 1000, 30, 10.0)).unwrap();
        mgr.add_pool(make_pool("p2", 2000, 2000, 30, 20.0)).unwrap();
        assert_eq!(mgr.list_pools().len(), 2);
    }

    #[test]
    fn test_add_position() {
        let mut mgr = LiquidityPoolManager::new();
        mgr.add_pool(make_pool("pool-1", 10000, 10000, 30, 12.5))
            .unwrap();
        let pos = make_position("pos-1", "pool-1", 50);
        assert!(mgr.add_position(pos).is_ok());
        assert!(mgr.get_position("pos-1").is_some());
    }

    #[test]
    fn test_add_position_duplicate_error() {
        let mut mgr = LiquidityPoolManager::new();
        mgr.add_pool(make_pool("pool-1", 10000, 10000, 30, 12.5))
            .unwrap();
        mgr.add_position(make_position("pos-1", "pool-1", 50))
            .unwrap();
        let result = mgr.add_position(make_position("pos-1", "pool-1", 100));
        assert!(matches!(
            result.unwrap_err(),
            LiquidityPoolError::PositionAlreadyExists(_)
        ));
    }

    #[test]
    fn test_add_position_pool_not_found() {
        let mut mgr = LiquidityPoolManager::new();
        let result = mgr.add_position(make_position("pos-1", "nonexistent", 50));
        assert!(matches!(
            result.unwrap_err(),
            LiquidityPoolError::PoolNotFound(_)
        ));
    }

    #[test]
    fn test_withdraw_position() {
        let mut mgr = LiquidityPoolManager::new();
        mgr.add_pool(make_pool("pool-1", 10000, 10000, 30, 12.5))
            .unwrap();
        mgr.add_position(make_position("pos-1", "pool-1", 50))
            .unwrap();
        let pos = mgr.withdraw_position("pos-1").unwrap();
        assert_eq!(pos.status, PositionStatus::Withdrawn);
    }

    #[test]
    fn test_withdraw_position_already_withdrawn() {
        let mut mgr = LiquidityPoolManager::new();
        mgr.add_pool(make_pool("pool-1", 10000, 10000, 30, 12.5))
            .unwrap();
        mgr.add_position(make_position("pos-1", "pool-1", 50))
            .unwrap();
        mgr.withdraw_position("pos-1").unwrap();
        let result = mgr.withdraw_position("pos-1");
        assert!(matches!(
            result.unwrap_err(),
            LiquidityPoolError::PositionNotActive(_)
        ));
    }

    #[test]
    fn test_positions_for_pool() {
        let mut mgr = LiquidityPoolManager::new();
        mgr.add_pool(make_pool("pool-1", 10000, 10000, 30, 12.5))
            .unwrap();
        mgr.add_pool(make_pool("pool-2", 5000, 5000, 30, 8.0))
            .unwrap();
        mgr.add_position(make_position("pos-1", "pool-1", 50))
            .unwrap();
        mgr.add_position(make_position("pos-2", "pool-1", 30))
            .unwrap();
        mgr.add_position(make_position("pos-3", "pool-2", 20))
            .unwrap();
        assert_eq!(mgr.positions_for_pool("pool-1").len(), 2);
        assert_eq!(mgr.positions_for_pool("pool-2").len(), 1);
    }

    #[test]
    fn test_active_positions() {
        let mut mgr = LiquidityPoolManager::new();
        mgr.add_pool(make_pool("pool-1", 10000, 10000, 30, 12.5))
            .unwrap();
        mgr.add_position(make_position("pos-1", "pool-1", 50))
            .unwrap();
        mgr.add_position(make_position("pos-2", "pool-1", 30))
            .unwrap();
        mgr.withdraw_position("pos-2").unwrap();
        let active = mgr.active_positions();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "pos-1");
    }

    #[test]
    fn test_claim_rewards() {
        let mut mgr = LiquidityPoolManager::new();
        mgr.add_pool(make_pool("pool-1", 10000, 10000, 30, 12.5))
            .unwrap();
        mgr.add_position(make_position("pos-1", "pool-1", 200))
            .unwrap();
        let claimed = mgr.claim_rewards("pos-1").unwrap();
        assert_eq!(claimed, 200);
        let pos = mgr.get_position("pos-1").unwrap();
        assert_eq!(pos.rewards_claimed, 200);
        assert_eq!(pos.pending_rewards, 0);
    }

    #[test]
    fn test_claim_rewards_no_pending() {
        let mut mgr = LiquidityPoolManager::new();
        mgr.add_pool(make_pool("pool-1", 10000, 10000, 30, 12.5))
            .unwrap();
        mgr.add_position(make_position("pos-1", "pool-1", 0))
            .unwrap();
        let result = mgr.claim_rewards("pos-1");
        assert!(matches!(
            result.unwrap_err(),
            LiquidityPoolError::NoPendingRewards(_)
        ));
    }

    #[test]
    fn test_calculate_il() {
        let mut mgr = LiquidityPoolManager::new();
        mgr.add_pool(make_pool("pool-1", 10000, 10000, 30, 12.5))
            .unwrap();
        mgr.add_position(make_position("pos-1", "pool-1", 50))
            .unwrap();

        // Price doubles: ratio = 2.0
        let il = mgr.calculate_il("pos-1", 2.0).unwrap();
        assert_eq!(il.pool_id, "pool-1");
        assert_eq!(il.position_id, "pos-1");
        assert_eq!(il.initial_value, 1000.0);

        // IL at 2x should be ~5.72%
        assert!((il.il_percentage - 5.72).abs() < 0.1);
        assert!(il.current_value < il.initial_value);
    }

    #[test]
    fn test_calculate_il_no_change() {
        let mut mgr = LiquidityPoolManager::new();
        mgr.add_pool(make_pool("pool-1", 10000, 10000, 30, 12.5))
            .unwrap();
        mgr.add_position(make_position("pos-1", "pool-1", 0))
            .unwrap();

        // Price unchanged: ratio = 1.0 -> IL = 0%
        let il = mgr.calculate_il("pos-1", 1.0).unwrap();
        assert!((il.il_percentage).abs() < 0.01);
    }

    #[test]
    fn test_estimate_output_a_to_b() {
        let mut mgr = LiquidityPoolManager::new();
        // 30 bps fee, equal reserves
        mgr.add_pool(make_pool("pool-1", 1_000_000, 1_000_000, 30, 10.0))
            .unwrap();

        let output = mgr.estimate_output("pool-1", 1000, true).unwrap();
        // With 30bps fee on 1000 input into 1M/1M pool, output ~ 997 (slightly less due to price impact)
        assert!(output > 0);
        assert!(output < 1000); // Must be less than input (fees + slippage)
    }

    #[test]
    fn test_estimate_output_b_to_a() {
        let mut mgr = LiquidityPoolManager::new();
        mgr.add_pool(make_pool("pool-1", 1_000_000, 500_000, 30, 10.0))
            .unwrap();

        let output = mgr.estimate_output("pool-1", 1000, false).unwrap();
        // B->A with more reserve_a than reserve_b, output should be > input value-wise
        assert!(output > 0);
    }

    #[test]
    fn test_estimate_output_pool_not_found() {
        let mgr = LiquidityPoolManager::new();
        let result = mgr.estimate_output("nonexistent", 1000, true);
        assert!(matches!(
            result.unwrap_err(),
            LiquidityPoolError::PoolNotFound(_)
        ));
    }

    #[test]
    fn test_pools_by_apy() {
        let mut mgr = LiquidityPoolManager::new();
        mgr.add_pool(make_pool("low", 1000, 1000, 30, 5.0)).unwrap();
        mgr.add_pool(make_pool("high", 1000, 1000, 30, 25.0)).unwrap();
        mgr.add_pool(make_pool("mid", 1000, 1000, 30, 12.0)).unwrap();

        let sorted = mgr.pools_by_apy();
        assert_eq!(sorted[0].id, "high");
        assert_eq!(sorted[1].id, "mid");
        assert_eq!(sorted[2].id, "low");
    }

    #[test]
    fn test_total_value_locked() {
        let mut mgr = LiquidityPoolManager::new();
        mgr.add_pool(make_pool("p1", 1000, 2000, 30, 10.0)).unwrap();
        mgr.add_pool(make_pool("p2", 3000, 4000, 30, 15.0)).unwrap();
        assert_eq!(mgr.total_value_locked(), 10000);
    }

    #[test]
    fn test_stats() {
        let mut mgr = LiquidityPoolManager::new();
        mgr.add_pool(make_pool("pool-1", 10000, 10000, 30, 12.5))
            .unwrap();
        mgr.add_pool(make_pool("pool-2", 5000, 5000, 30, 8.0))
            .unwrap();
        mgr.add_position(make_position("pos-1", "pool-1", 100))
            .unwrap();
        mgr.add_position(make_position("pos-2", "pool-1", 200))
            .unwrap();
        mgr.add_position(make_position("pos-3", "pool-2", 50))
            .unwrap();
        mgr.withdraw_position("pos-3").unwrap();

        let stats = mgr.stats();
        assert_eq!(stats.total_pools, 2);
        assert_eq!(stats.total_positions, 3);
        assert_eq!(stats.active_positions, 2);
        assert_eq!(stats.total_deposited_value, 3000); // 3 positions * 1000 each
        assert_eq!(stats.total_pending_rewards, 350); // 100 + 200 + 50
        assert_eq!(stats.total_rewards_claimed, 0);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let mut mgr = LiquidityPoolManager::new();
        mgr.add_pool(make_pool("pool-1", 10000, 10000, 30, 12.5))
            .unwrap();
        mgr.add_position(make_position("pos-1", "pool-1", 50))
            .unwrap();

        let dir = std::env::temp_dir();
        let path = dir.join(format!("lp_data_{}.json", std::process::id()));

        mgr.save(&path).unwrap();

        let loaded = LiquidityPoolManager::load(&path).unwrap();
        assert_eq!(loaded.pools.len(), 1);
        assert_eq!(loaded.positions.len(), 1);
        assert!(loaded.get_pool("pool-1").is_some());
        assert!(loaded.get_position("pos-1").is_some());
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let mgr = LiquidityPoolManager::load_or_default(Path::new("/tmp/nonexistent_lp.json"));
        assert_eq!(mgr.pools.len(), 0);
        assert_eq!(mgr.positions.len(), 0);
    }
}
