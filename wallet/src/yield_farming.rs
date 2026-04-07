use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum YieldFarmingError {
    #[error("Farm already exists: {0}")]
    FarmAlreadyExists(String),
    #[error("Farm not found: {0}")]
    FarmNotFound(String),
    #[error("Position already exists: {0}")]
    PositionAlreadyExists(String),
    #[error("Position not found: {0}")]
    PositionNotFound(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FarmStatus {
    Active,
    Paused,
    Ended,
    Harvesting,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CompoundStrategy {
    Manual,
    AutoDaily,
    AutoWeekly,
    Threshold(u64),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RewardType {
    Token,
    Lp,
    Nft,
    Points,
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YieldFarm {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub stake_token: String,
    pub reward_token: String,
    pub reward_type: RewardType,
    pub apy: f64,
    pub tvl: u64,
    pub status: FarmStatus,
    pub start_date: String,
    pub end_date: Option<String>,
    pub min_stake: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FarmPosition {
    pub id: String,
    pub farm_id: String,
    pub staked_amount: u64,
    pub entry_time: String,
    pub last_harvest: Option<String>,
    pub total_harvested: u64,
    pub pending_rewards: u64,
    pub compound_strategy: CompoundStrategy,
    pub auto_compound_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApyHistory {
    pub farm_id: String,
    pub entries: Vec<ApyEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApyEntry {
    pub timestamp: String,
    pub apy: f64,
    pub tvl: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FarmingStats {
    pub total_farms: usize,
    pub active_farms: usize,
    pub total_positions: usize,
    pub total_staked: u64,
    pub total_harvested: u64,
    pub total_pending: u64,
    pub avg_apy: f64,
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct YieldFarmManager {
    pub farms: HashMap<String, YieldFarm>,
    pub positions: HashMap<String, FarmPosition>,
    pub apy_history: HashMap<String, ApyHistory>,
}

impl YieldFarmManager {
    pub fn new() -> Self {
        Self::default()
    }

    // -- Farm CRUD ----------------------------------------------------------

    pub fn add_farm(&mut self, farm: YieldFarm) -> Result<(), YieldFarmingError> {
        if self.farms.contains_key(&farm.id) {
            return Err(YieldFarmingError::FarmAlreadyExists(farm.id));
        }
        self.farms.insert(farm.id.clone(), farm);
        Ok(())
    }

    pub fn remove_farm(&mut self, id: &str) -> Result<YieldFarm, YieldFarmingError> {
        self.farms
            .remove(id)
            .ok_or_else(|| YieldFarmingError::FarmNotFound(id.to_string()))
    }

    pub fn get_farm(&self, id: &str) -> Option<&YieldFarm> {
        self.farms.get(id)
    }

    pub fn list_farms(&self) -> Vec<&YieldFarm> {
        self.farms.values().collect()
    }

    pub fn active_farms(&self) -> Vec<&YieldFarm> {
        self.farms
            .values()
            .filter(|f| f.status == FarmStatus::Active)
            .collect()
    }

    // -- Positions ----------------------------------------------------------

    pub fn stake(&mut self, pos: FarmPosition) -> Result<(), YieldFarmingError> {
        if self.positions.contains_key(&pos.id) {
            return Err(YieldFarmingError::PositionAlreadyExists(pos.id));
        }
        if !self.farms.contains_key(&pos.farm_id) {
            return Err(YieldFarmingError::FarmNotFound(pos.farm_id));
        }
        self.positions.insert(pos.id.clone(), pos);
        Ok(())
    }

    pub fn unstake(&mut self, position_id: &str) -> Result<FarmPosition, YieldFarmingError> {
        self.positions
            .remove(position_id)
            .ok_or_else(|| YieldFarmingError::PositionNotFound(position_id.to_string()))
    }

    pub fn harvest(&mut self, position_id: &str) -> Result<u64, YieldFarmingError> {
        let pos = self
            .positions
            .get_mut(position_id)
            .ok_or_else(|| YieldFarmingError::PositionNotFound(position_id.to_string()))?;
        let harvested = pos.pending_rewards;
        pos.total_harvested += harvested;
        pos.pending_rewards = 0;
        pos.last_harvest = Some(chrono::Utc::now().to_rfc3339());
        Ok(harvested)
    }

    pub fn auto_compound(&mut self, position_id: &str) -> Result<u64, YieldFarmingError> {
        let pos = self
            .positions
            .get_mut(position_id)
            .ok_or_else(|| YieldFarmingError::PositionNotFound(position_id.to_string()))?;
        let compounded = pos.pending_rewards;
        pos.staked_amount += compounded;
        pos.pending_rewards = 0;
        pos.auto_compound_count += 1;
        Ok(compounded)
    }

    // -- APY history --------------------------------------------------------

    pub fn record_apy(&mut self, farm_id: &str, apy: f64, tvl: u64) {
        let entry = ApyEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            apy,
            tvl,
        };
        self.apy_history
            .entry(farm_id.to_string())
            .or_insert_with(|| ApyHistory {
                farm_id: farm_id.to_string(),
                entries: Vec::new(),
            })
            .entries
            .push(entry);
    }

    pub fn get_apy_history(&self, farm_id: &str) -> Option<&ApyHistory> {
        self.apy_history.get(farm_id)
    }

    // -- Analytics ----------------------------------------------------------

    pub fn best_farms(&self, n: usize) -> Vec<&YieldFarm> {
        let mut active: Vec<&YieldFarm> = self.active_farms();
        active.sort_by(|a, b| b.apy.partial_cmp(&a.apy).unwrap_or(std::cmp::Ordering::Equal));
        active.truncate(n);
        active
    }

    pub fn estimate_rewards(
        &self,
        position_id: &str,
        days: u32,
    ) -> Result<u64, YieldFarmingError> {
        let pos = self
            .positions
            .get(position_id)
            .ok_or_else(|| YieldFarmingError::PositionNotFound(position_id.to_string()))?;
        let farm = self
            .farms
            .get(&pos.farm_id)
            .ok_or_else(|| YieldFarmingError::FarmNotFound(pos.farm_id.clone()))?;

        // Simple daily compounding: principal * (1 + daily_rate)^days - principal
        let daily_rate = farm.apy / 100.0 / 365.0;
        let principal = pos.staked_amount as f64;
        let future_value = principal * (1.0 + daily_rate).powi(days as i32);
        let reward = (future_value - principal) as u64;
        Ok(reward)
    }

    pub fn stats(&self) -> FarmingStats {
        let active_farms = self
            .farms
            .values()
            .filter(|f| f.status == FarmStatus::Active)
            .count();

        let total_staked: u64 = self.positions.values().map(|p| p.staked_amount).sum();
        let total_harvested: u64 = self.positions.values().map(|p| p.total_harvested).sum();
        let total_pending: u64 = self.positions.values().map(|p| p.pending_rewards).sum();

        let active_apys: Vec<f64> = self
            .farms
            .values()
            .filter(|f| f.status == FarmStatus::Active)
            .map(|f| f.apy)
            .collect();
        let avg_apy = if active_apys.is_empty() {
            0.0
        } else {
            active_apys.iter().sum::<f64>() / active_apys.len() as f64
        };

        FarmingStats {
            total_farms: self.farms.len(),
            active_farms,
            total_positions: self.positions.len(),
            total_staked,
            total_harvested,
            total_pending,
            avg_apy,
        }
    }

    // -- Persistence --------------------------------------------------------

    pub fn load(path: &Path) -> Result<Self, YieldFarmingError> {
        let data = std::fs::read_to_string(path)?;
        let mgr: Self = serde_json::from_str(&data)?;
        Ok(mgr)
    }

    pub fn save(&self, path: &Path) -> Result<(), YieldFarmingError> {
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample_farm(id: &str, status: FarmStatus, apy: f64) -> YieldFarm {
        YieldFarm {
            id: id.to_string(),
            name: format!("Farm {}", id),
            protocol: "TestProtocol".to_string(),
            stake_token: "EVP".to_string(),
            reward_token: "REVP".to_string(),
            reward_type: RewardType::Token,
            apy,
            tvl: 1_000_000,
            status,
            start_date: "2026-01-01T00:00:00Z".to_string(),
            end_date: None,
            min_stake: 100,
        }
    }

    fn sample_position(id: &str, farm_id: &str) -> FarmPosition {
        FarmPosition {
            id: id.to_string(),
            farm_id: farm_id.to_string(),
            staked_amount: 10_000,
            entry_time: "2026-01-01T00:00:00Z".to_string(),
            last_harvest: None,
            total_harvested: 0,
            pending_rewards: 500,
            compound_strategy: CompoundStrategy::Manual,
            auto_compound_count: 0,
        }
    }

    #[test]
    fn test_add_farm() {
        let mut mgr = YieldFarmManager::new();
        let farm = sample_farm("f1", FarmStatus::Active, 12.0);
        assert!(mgr.add_farm(farm).is_ok());
        assert!(mgr.get_farm("f1").is_some());
    }

    #[test]
    fn test_add_farm_duplicate_error() {
        let mut mgr = YieldFarmManager::new();
        mgr.add_farm(sample_farm("f1", FarmStatus::Active, 12.0)).unwrap();
        let result = mgr.add_farm(sample_farm("f1", FarmStatus::Active, 15.0));
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_farm() {
        let mut mgr = YieldFarmManager::new();
        mgr.add_farm(sample_farm("f1", FarmStatus::Active, 12.0)).unwrap();
        let removed = mgr.remove_farm("f1").unwrap();
        assert_eq!(removed.id, "f1");
        assert!(mgr.get_farm("f1").is_none());
    }

    #[test]
    fn test_remove_farm_not_found() {
        let mut mgr = YieldFarmManager::new();
        assert!(mgr.remove_farm("nonexistent").is_err());
    }

    #[test]
    fn test_list_farms() {
        let mut mgr = YieldFarmManager::new();
        mgr.add_farm(sample_farm("f1", FarmStatus::Active, 10.0)).unwrap();
        mgr.add_farm(sample_farm("f2", FarmStatus::Paused, 20.0)).unwrap();
        assert_eq!(mgr.list_farms().len(), 2);
    }

    #[test]
    fn test_active_farms_filter() {
        let mut mgr = YieldFarmManager::new();
        mgr.add_farm(sample_farm("f1", FarmStatus::Active, 10.0)).unwrap();
        mgr.add_farm(sample_farm("f2", FarmStatus::Paused, 20.0)).unwrap();
        mgr.add_farm(sample_farm("f3", FarmStatus::Active, 30.0)).unwrap();
        assert_eq!(mgr.active_farms().len(), 2);
    }

    #[test]
    fn test_stake() {
        let mut mgr = YieldFarmManager::new();
        mgr.add_farm(sample_farm("f1", FarmStatus::Active, 12.0)).unwrap();
        let pos = sample_position("p1", "f1");
        assert!(mgr.stake(pos).is_ok());
    }

    #[test]
    fn test_stake_duplicate_error() {
        let mut mgr = YieldFarmManager::new();
        mgr.add_farm(sample_farm("f1", FarmStatus::Active, 12.0)).unwrap();
        mgr.stake(sample_position("p1", "f1")).unwrap();
        let result = mgr.stake(sample_position("p1", "f1"));
        assert!(result.is_err());
    }

    #[test]
    fn test_stake_missing_farm_error() {
        let mut mgr = YieldFarmManager::new();
        let pos = sample_position("p1", "no_such_farm");
        assert!(mgr.stake(pos).is_err());
    }

    #[test]
    fn test_unstake() {
        let mut mgr = YieldFarmManager::new();
        mgr.add_farm(sample_farm("f1", FarmStatus::Active, 12.0)).unwrap();
        mgr.stake(sample_position("p1", "f1")).unwrap();
        let pos = mgr.unstake("p1").unwrap();
        assert_eq!(pos.id, "p1");
        assert!(mgr.unstake("p1").is_err());
    }

    #[test]
    fn test_harvest() {
        let mut mgr = YieldFarmManager::new();
        mgr.add_farm(sample_farm("f1", FarmStatus::Active, 12.0)).unwrap();
        mgr.stake(sample_position("p1", "f1")).unwrap();
        let harvested = mgr.harvest("p1").unwrap();
        assert_eq!(harvested, 500);
        let pos = mgr.positions.get("p1").unwrap();
        assert_eq!(pos.pending_rewards, 0);
        assert_eq!(pos.total_harvested, 500);
        assert!(pos.last_harvest.is_some());
    }

    #[test]
    fn test_harvest_missing_position() {
        let mut mgr = YieldFarmManager::new();
        assert!(mgr.harvest("nonexistent").is_err());
    }

    #[test]
    fn test_auto_compound() {
        let mut mgr = YieldFarmManager::new();
        mgr.add_farm(sample_farm("f1", FarmStatus::Active, 12.0)).unwrap();
        mgr.stake(sample_position("p1", "f1")).unwrap();
        let compounded = mgr.auto_compound("p1").unwrap();
        assert_eq!(compounded, 500);
        let pos = mgr.positions.get("p1").unwrap();
        assert_eq!(pos.staked_amount, 10_500);
        assert_eq!(pos.pending_rewards, 0);
        assert_eq!(pos.auto_compound_count, 1);
    }

    #[test]
    fn test_auto_compound_missing_position() {
        let mut mgr = YieldFarmManager::new();
        assert!(mgr.auto_compound("nonexistent").is_err());
    }

    #[test]
    fn test_record_apy_and_get_history() {
        let mut mgr = YieldFarmManager::new();
        mgr.record_apy("f1", 12.5, 1_000_000);
        mgr.record_apy("f1", 13.0, 1_100_000);
        let history = mgr.get_apy_history("f1").unwrap();
        assert_eq!(history.entries.len(), 2);
        assert_eq!(history.entries[0].apy, 12.5);
        assert_eq!(history.entries[1].tvl, 1_100_000);
    }

    #[test]
    fn test_get_apy_history_not_found() {
        let mgr = YieldFarmManager::new();
        assert!(mgr.get_apy_history("nonexistent").is_none());
    }

    #[test]
    fn test_best_farms_sorted() {
        let mut mgr = YieldFarmManager::new();
        mgr.add_farm(sample_farm("f1", FarmStatus::Active, 10.0)).unwrap();
        mgr.add_farm(sample_farm("f2", FarmStatus::Active, 30.0)).unwrap();
        mgr.add_farm(sample_farm("f3", FarmStatus::Active, 20.0)).unwrap();
        mgr.add_farm(sample_farm("f4", FarmStatus::Paused, 50.0)).unwrap();
        let best = mgr.best_farms(2);
        assert_eq!(best.len(), 2);
        assert_eq!(best[0].id, "f2");
        assert_eq!(best[1].id, "f3");
    }

    #[test]
    fn test_estimate_rewards() {
        let mut mgr = YieldFarmManager::new();
        mgr.add_farm(sample_farm("f1", FarmStatus::Active, 36.5)).unwrap();
        mgr.stake(sample_position("p1", "f1")).unwrap();
        let est = mgr.estimate_rewards("p1", 365).unwrap();
        // 36.5% APY on 10_000 staked with daily compounding ~ 4,395
        assert!(est > 4_000 && est < 5_000, "estimated rewards: {}", est);
    }

    #[test]
    fn test_estimate_rewards_missing_position() {
        let mgr = YieldFarmManager::new();
        assert!(mgr.estimate_rewards("nonexistent", 30).is_err());
    }

    #[test]
    fn test_stats() {
        let mut mgr = YieldFarmManager::new();
        mgr.add_farm(sample_farm("f1", FarmStatus::Active, 10.0)).unwrap();
        mgr.add_farm(sample_farm("f2", FarmStatus::Paused, 20.0)).unwrap();
        mgr.stake(sample_position("p1", "f1")).unwrap();
        let stats = mgr.stats();
        assert_eq!(stats.total_farms, 2);
        assert_eq!(stats.active_farms, 1);
        assert_eq!(stats.total_positions, 1);
        assert_eq!(stats.total_staked, 10_000);
        assert_eq!(stats.total_pending, 500);
        assert!((stats.avg_apy - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let mut mgr = YieldFarmManager::new();
        mgr.add_farm(sample_farm("f1", FarmStatus::Active, 12.0)).unwrap();
        mgr.stake(sample_position("p1", "f1")).unwrap();
        mgr.record_apy("f1", 12.0, 1_000_000);

        let dir = std::env::temp_dir().join("evaporchain_yield_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("yield_farming.json");

        mgr.save(&path).unwrap();
        let loaded = YieldFarmManager::load(&path).unwrap();
        assert_eq!(loaded.farms.len(), 1);
        assert_eq!(loaded.positions.len(), 1);
        assert_eq!(loaded.apy_history.get("f1").unwrap().entries.len(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = PathBuf::from("/tmp/evaporchain_yield_nonexistent_987654.json");
        let mgr = YieldFarmManager::load_or_default(&path);
        assert_eq!(mgr.farms.len(), 0);
    }
}
