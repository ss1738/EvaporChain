use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum WhaleTrackerError {
    #[error("Whale already tracked: {0}")]
    AlreadyTracked(String),

    #[error("Whale not found: {0}")]
    NotFound(String),

    #[error("Cluster already exists: {0}")]
    ClusterExists(String),

    #[error("Cluster not found: {0}")]
    ClusterNotFound(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WhaleActivity {
    Accumulating,
    Distributing,
    Holding,
    Dormant,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MovementType {
    Inflow,
    Outflow,
    InternalTransfer,
    ExchangeDeposit,
    ExchangeWithdrawal,
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhaleAccount {
    pub address: String,
    pub label: Option<String>,
    pub balance: u64,
    pub first_seen: String,
    pub last_active: String,
    pub activity: WhaleActivity,
    pub cluster_id: Option<String>,
    pub is_exchange: bool,
    pub total_inflow: u64,
    pub total_outflow: u64,
    pub tx_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhaleMovement {
    pub id: String,
    pub from: String,
    pub to: String,
    pub amount: u64,
    pub token: String,
    pub movement_type: MovementType,
    pub timestamp: String,
    pub tx_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletCluster {
    pub id: String,
    pub addresses: Vec<String>,
    pub label: Option<String>,
    pub total_balance: u64,
    pub activity: WhaleActivity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhaleAlert {
    pub address: String,
    pub amount: u64,
    pub movement_type: MovementType,
    pub timestamp: String,
    pub significance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhaleStats {
    pub tracked_whales: usize,
    pub total_movements: usize,
    pub clusters: usize,
    pub accumulating: usize,
    pub distributing: usize,
    pub dormant: usize,
    pub total_whale_balance: u64,
}

// ---------------------------------------------------------------------------
// Main Store
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhaleTracker {
    pub whales: HashMap<String, WhaleAccount>,
    pub movements: Vec<WhaleMovement>,
    pub clusters: HashMap<String, WalletCluster>,
    pub min_whale_balance: u64,
}

impl Default for WhaleTracker {
    fn default() -> Self {
        Self {
            whales: HashMap::new(),
            movements: Vec::new(),
            clusters: HashMap::new(),
            min_whale_balance: 1_000_000,
        }
    }
}

impl WhaleTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_min_balance(min: u64) -> Self {
        Self {
            min_whale_balance: min,
            ..Self::default()
        }
    }

    // -- Whale management ---------------------------------------------------

    pub fn track_whale(&mut self, account: WhaleAccount) -> Result<(), WhaleTrackerError> {
        if self.whales.contains_key(&account.address) {
            return Err(WhaleTrackerError::AlreadyTracked(account.address));
        }
        self.whales.insert(account.address.clone(), account);
        Ok(())
    }

    pub fn untrack_whale(&mut self, address: &str) -> Result<WhaleAccount, WhaleTrackerError> {
        self.whales
            .remove(address)
            .ok_or_else(|| WhaleTrackerError::NotFound(address.to_string()))
    }

    pub fn update_balance(
        &mut self,
        address: &str,
        new_balance: u64,
    ) -> Result<(), WhaleTrackerError> {
        let whale = self
            .whales
            .get_mut(address)
            .ok_or_else(|| WhaleTrackerError::NotFound(address.to_string()))?;

        let old_balance = whale.balance;
        whale.balance = new_balance;
        whale.last_active = chrono::Utc::now().to_rfc3339();

        whale.activity = if new_balance > old_balance {
            WhaleActivity::Accumulating
        } else if new_balance < old_balance {
            WhaleActivity::Distributing
        } else {
            WhaleActivity::Holding
        };

        Ok(())
    }

    // -- Movements ----------------------------------------------------------

    pub fn record_movement(&mut self, movement: WhaleMovement) {
        // Update sender
        if let Some(sender) = self.whales.get_mut(&movement.from) {
            sender.total_outflow = sender.total_outflow.saturating_add(movement.amount);
            sender.balance = sender.balance.saturating_sub(movement.amount);
            sender.tx_count += 1;
            sender.last_active = chrono::Utc::now().to_rfc3339();
            sender.activity = if sender.total_inflow > sender.total_outflow {
                WhaleActivity::Accumulating
            } else {
                WhaleActivity::Distributing
            };
        }

        // Update receiver
        if let Some(receiver) = self.whales.get_mut(&movement.to) {
            receiver.total_inflow = receiver.total_inflow.saturating_add(movement.amount);
            receiver.balance = receiver.balance.saturating_add(movement.amount);
            receiver.tx_count += 1;
            receiver.last_active = chrono::Utc::now().to_rfc3339();
            receiver.activity = if receiver.total_inflow > receiver.total_outflow {
                WhaleActivity::Accumulating
            } else {
                WhaleActivity::Distributing
            };
        }

        self.movements.push(movement);
    }

    pub fn detect_activity(&self, address: &str) -> Result<WhaleActivity, WhaleTrackerError> {
        let whale = self
            .whales
            .get(address)
            .ok_or_else(|| WhaleTrackerError::NotFound(address.to_string()))?;

        if whale.tx_count == 0 {
            return Ok(WhaleActivity::Dormant);
        }

        if whale.total_inflow > whale.total_outflow {
            Ok(WhaleActivity::Accumulating)
        } else if whale.total_outflow > whale.total_inflow {
            Ok(WhaleActivity::Distributing)
        } else {
            Ok(WhaleActivity::Holding)
        }
    }

    // -- Clusters -----------------------------------------------------------

    pub fn create_cluster(
        &mut self,
        id: &str,
        addresses: Vec<String>,
        label: Option<String>,
    ) -> Result<(), WhaleTrackerError> {
        if self.clusters.contains_key(id) {
            return Err(WhaleTrackerError::ClusterExists(id.to_string()));
        }

        let total_balance: u64 = addresses
            .iter()
            .filter_map(|a| self.whales.get(a))
            .map(|w| w.balance)
            .sum();

        let cluster = WalletCluster {
            id: id.to_string(),
            addresses,
            label,
            total_balance,
            activity: WhaleActivity::Holding,
        };
        self.clusters.insert(id.to_string(), cluster);
        Ok(())
    }

    pub fn get_cluster(&self, id: &str) -> Option<&WalletCluster> {
        self.clusters.get(id)
    }

    pub fn find_cluster_for_address(&self, address: &str) -> Option<&WalletCluster> {
        self.clusters
            .values()
            .find(|c| c.addresses.iter().any(|a| a == address))
    }

    // -- Queries ------------------------------------------------------------

    pub fn top_whales(&self, n: usize) -> Vec<&WhaleAccount> {
        let mut whales: Vec<&WhaleAccount> = self.whales.values().collect();
        whales.sort_by_key(|a| std::cmp::Reverse(a.balance));
        whales.truncate(n);
        whales
    }

    pub fn recent_movements(&self, n: usize) -> Vec<&WhaleMovement> {
        self.movements.iter().rev().take(n).collect()
    }

    pub fn movements_for_address(&self, address: &str) -> Vec<&WhaleMovement> {
        self.movements
            .iter()
            .filter(|m| m.from == address || m.to == address)
            .collect()
    }

    pub fn generate_alerts(&self, threshold: u64) -> Vec<WhaleAlert> {
        self.movements
            .iter()
            .filter(|m| m.amount >= threshold)
            .map(|m| {
                let significance = if m.amount >= threshold * 10 {
                    "critical".to_string()
                } else if m.amount >= threshold * 5 {
                    "high".to_string()
                } else {
                    "medium".to_string()
                };
                WhaleAlert {
                    address: m.from.clone(),
                    amount: m.amount,
                    movement_type: m.movement_type.clone(),
                    timestamp: m.timestamp.clone(),
                    significance,
                }
            })
            .collect()
    }

    pub fn accumulation_score(&self, address: &str) -> Result<f64, WhaleTrackerError> {
        let whale = self
            .whales
            .get(address)
            .ok_or_else(|| WhaleTrackerError::NotFound(address.to_string()))?;

        let total = whale.total_inflow + whale.total_outflow;
        if total == 0 {
            return Ok(0.0);
        }
        Ok(whale.total_inflow as f64 / total as f64)
    }

    pub fn stats(&self) -> WhaleStats {
        let accumulating = self
            .whales
            .values()
            .filter(|w| w.activity == WhaleActivity::Accumulating)
            .count();
        let distributing = self
            .whales
            .values()
            .filter(|w| w.activity == WhaleActivity::Distributing)
            .count();
        let dormant = self
            .whales
            .values()
            .filter(|w| w.activity == WhaleActivity::Dormant)
            .count();
        let total_whale_balance: u64 = self.whales.values().map(|w| w.balance).sum();

        WhaleStats {
            tracked_whales: self.whales.len(),
            total_movements: self.movements.len(),
            clusters: self.clusters.len(),
            accumulating,
            distributing,
            dormant,
            total_whale_balance,
        }
    }

    // -- Persistence --------------------------------------------------------

    pub fn load(path: &Path) -> Result<Self, WhaleTrackerError> {
        let data = std::fs::read_to_string(path)?;
        let tracker: Self = serde_json::from_str(&data)?;
        Ok(tracker)
    }

    pub fn save(&self, path: &Path) -> Result<(), WhaleTrackerError> {
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
    use std::env::temp_dir;
    use std::process::id;

    fn temp_path(name: &str) -> std::path::PathBuf {
        temp_dir().join(format!("whale_tracker_{}_{}", id(), name))
    }

    fn make_whale(address: &str, balance: u64) -> WhaleAccount {
        WhaleAccount {
            address: address.to_string(),
            label: None,
            balance,
            first_seen: chrono::Utc::now().to_rfc3339(),
            last_active: chrono::Utc::now().to_rfc3339(),
            activity: WhaleActivity::Holding,
            cluster_id: None,
            is_exchange: false,
            total_inflow: 0,
            total_outflow: 0,
            tx_count: 0,
        }
    }

    fn make_movement(id: &str, from: &str, to: &str, amount: u64) -> WhaleMovement {
        WhaleMovement {
            id: id.to_string(),
            from: from.to_string(),
            to: to.to_string(),
            amount,
            token: "EVAP".to_string(),
            movement_type: MovementType::Outflow,
            timestamp: chrono::Utc::now().to_rfc3339(),
            tx_hash: None,
        }
    }

    #[test]
    fn test_new_default() {
        let tracker = WhaleTracker::new();
        assert_eq!(tracker.min_whale_balance, 1_000_000);
        assert!(tracker.whales.is_empty());
    }

    #[test]
    fn test_with_min_balance() {
        let tracker = WhaleTracker::with_min_balance(500_000);
        assert_eq!(tracker.min_whale_balance, 500_000);
    }

    #[test]
    fn test_track_whale() {
        let mut tracker = WhaleTracker::new();
        let whale = make_whale("addr1", 5_000_000);
        assert!(tracker.track_whale(whale).is_ok());
        assert_eq!(tracker.whales.len(), 1);
    }

    #[test]
    fn test_track_whale_duplicate() {
        let mut tracker = WhaleTracker::new();
        tracker.track_whale(make_whale("addr1", 5_000_000)).unwrap();
        let result = tracker.track_whale(make_whale("addr1", 3_000_000));
        assert!(result.is_err());
    }

    #[test]
    fn test_untrack_whale() {
        let mut tracker = WhaleTracker::new();
        tracker.track_whale(make_whale("addr1", 5_000_000)).unwrap();
        let removed = tracker.untrack_whale("addr1").unwrap();
        assert_eq!(removed.address, "addr1");
        assert!(tracker.whales.is_empty());
    }

    #[test]
    fn test_untrack_whale_not_found() {
        let mut tracker = WhaleTracker::new();
        assert!(tracker.untrack_whale("nonexistent").is_err());
    }

    #[test]
    fn test_update_balance_accumulating() {
        let mut tracker = WhaleTracker::new();
        tracker.track_whale(make_whale("addr1", 1_000_000)).unwrap();
        tracker.update_balance("addr1", 2_000_000).unwrap();
        let whale = tracker.whales.get("addr1").unwrap();
        assert_eq!(whale.balance, 2_000_000);
        assert_eq!(whale.activity, WhaleActivity::Accumulating);
    }

    #[test]
    fn test_update_balance_distributing() {
        let mut tracker = WhaleTracker::new();
        tracker.track_whale(make_whale("addr1", 5_000_000)).unwrap();
        tracker.update_balance("addr1", 2_000_000).unwrap();
        let whale = tracker.whales.get("addr1").unwrap();
        assert_eq!(whale.activity, WhaleActivity::Distributing);
    }

    #[test]
    fn test_update_balance_not_found() {
        let mut tracker = WhaleTracker::new();
        assert!(tracker.update_balance("nonexistent", 100).is_err());
    }

    #[test]
    fn test_record_movement() {
        let mut tracker = WhaleTracker::new();
        tracker
            .track_whale(make_whale("sender", 10_000_000))
            .unwrap();
        tracker
            .track_whale(make_whale("receiver", 1_000_000))
            .unwrap();

        let movement = make_movement("m1", "sender", "receiver", 500_000);
        tracker.record_movement(movement);

        assert_eq!(tracker.movements.len(), 1);
        let sender = tracker.whales.get("sender").unwrap();
        assert_eq!(sender.total_outflow, 500_000);
        assert_eq!(sender.balance, 9_500_000);

        let receiver = tracker.whales.get("receiver").unwrap();
        assert_eq!(receiver.total_inflow, 500_000);
        assert_eq!(receiver.balance, 1_500_000);
    }

    #[test]
    fn test_detect_activity_dormant() {
        let mut tracker = WhaleTracker::new();
        tracker.track_whale(make_whale("addr1", 5_000_000)).unwrap();
        let activity = tracker.detect_activity("addr1").unwrap();
        assert_eq!(activity, WhaleActivity::Dormant);
    }

    #[test]
    fn test_detect_activity_accumulating() {
        let mut tracker = WhaleTracker::new();
        let mut whale = make_whale("addr1", 5_000_000);
        whale.total_inflow = 1_000_000;
        whale.total_outflow = 200_000;
        whale.tx_count = 5;
        tracker.track_whale(whale).unwrap();
        let activity = tracker.detect_activity("addr1").unwrap();
        assert_eq!(activity, WhaleActivity::Accumulating);
    }

    #[test]
    fn test_create_cluster() {
        let mut tracker = WhaleTracker::new();
        tracker.track_whale(make_whale("a1", 3_000_000)).unwrap();
        tracker.track_whale(make_whale("a2", 2_000_000)).unwrap();

        tracker
            .create_cluster("c1", vec!["a1".into(), "a2".into()], Some("Group A".into()))
            .unwrap();

        let cluster = tracker.get_cluster("c1").unwrap();
        assert_eq!(cluster.total_balance, 5_000_000);
        assert_eq!(cluster.addresses.len(), 2);
    }

    #[test]
    fn test_create_cluster_duplicate() {
        let mut tracker = WhaleTracker::new();
        tracker.create_cluster("c1", vec![], None).unwrap();
        assert!(tracker.create_cluster("c1", vec![], None).is_err());
    }

    #[test]
    fn test_find_cluster_for_address() {
        let mut tracker = WhaleTracker::new();
        tracker
            .create_cluster("c1", vec!["a1".into(), "a2".into()], None)
            .unwrap();
        let cluster = tracker.find_cluster_for_address("a2").unwrap();
        assert_eq!(cluster.id, "c1");
        assert!(tracker.find_cluster_for_address("a99").is_none());
    }

    #[test]
    fn test_top_whales() {
        let mut tracker = WhaleTracker::new();
        tracker.track_whale(make_whale("a1", 1_000_000)).unwrap();
        tracker.track_whale(make_whale("a2", 9_000_000)).unwrap();
        tracker.track_whale(make_whale("a3", 5_000_000)).unwrap();

        let top = tracker.top_whales(2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].address, "a2");
        assert_eq!(top[1].address, "a3");
    }

    #[test]
    fn test_recent_movements() {
        let mut tracker = WhaleTracker::new();
        tracker.record_movement(make_movement("m1", "a", "b", 100));
        tracker.record_movement(make_movement("m2", "a", "b", 200));
        tracker.record_movement(make_movement("m3", "a", "b", 300));

        let recent = tracker.recent_movements(2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].id, "m3");
        assert_eq!(recent[1].id, "m2");
    }

    #[test]
    fn test_movements_for_address() {
        let mut tracker = WhaleTracker::new();
        tracker.record_movement(make_movement("m1", "a1", "a2", 100));
        tracker.record_movement(make_movement("m2", "a3", "a1", 200));
        tracker.record_movement(make_movement("m3", "a2", "a3", 300));

        let moves = tracker.movements_for_address("a1");
        assert_eq!(moves.len(), 2);
    }

    #[test]
    fn test_generate_alerts() {
        let mut tracker = WhaleTracker::new();
        tracker.record_movement(make_movement("m1", "a", "b", 500));
        tracker.record_movement(make_movement("m2", "a", "b", 5_000));
        tracker.record_movement(make_movement("m3", "a", "b", 50_000));

        let alerts = tracker.generate_alerts(1_000);
        assert_eq!(alerts.len(), 2);
    }

    #[test]
    fn test_accumulation_score() {
        let mut tracker = WhaleTracker::new();
        let mut whale = make_whale("addr1", 5_000_000);
        whale.total_inflow = 800_000;
        whale.total_outflow = 200_000;
        tracker.track_whale(whale).unwrap();

        let score = tracker.accumulation_score("addr1").unwrap();
        assert!((score - 0.8).abs() < 1e-9);
    }

    #[test]
    fn test_accumulation_score_zero() {
        let mut tracker = WhaleTracker::new();
        tracker.track_whale(make_whale("addr1", 5_000_000)).unwrap();
        let score = tracker.accumulation_score("addr1").unwrap();
        assert!((score - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_stats() {
        let mut tracker = WhaleTracker::new();
        let mut w1 = make_whale("a1", 3_000_000);
        w1.activity = WhaleActivity::Accumulating;
        let mut w2 = make_whale("a2", 2_000_000);
        w2.activity = WhaleActivity::Distributing;
        let mut w3 = make_whale("a3", 1_000_000);
        w3.activity = WhaleActivity::Dormant;

        tracker.track_whale(w1).unwrap();
        tracker.track_whale(w2).unwrap();
        tracker.track_whale(w3).unwrap();
        tracker.record_movement(make_movement("m1", "a1", "a2", 100));

        let stats = tracker.stats();
        assert_eq!(stats.tracked_whales, 3);
        assert_eq!(stats.total_movements, 1);
        assert_eq!(stats.total_whale_balance, 6_000_000);
    }

    #[test]
    fn test_save_and_load() {
        let path = temp_path("save_load.json");
        let mut tracker = WhaleTracker::new();
        tracker.track_whale(make_whale("addr1", 5_000_000)).unwrap();
        tracker.save(&path).unwrap();

        let loaded = WhaleTracker::load(&path).unwrap();
        assert_eq!(loaded.whales.len(), 1);
        assert_eq!(loaded.whales.get("addr1").unwrap().balance, 5_000_000);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = temp_path("nonexistent_file.json");
        let tracker = WhaleTracker::load_or_default(&path);
        assert_eq!(tracker.min_whale_balance, 1_000_000);
        assert!(tracker.whales.is_empty());
    }
}
