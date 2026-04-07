// wallet/src/gas_station.rs — Gas station network for meta-transactions
//
// Enables gasless transactions via relay network:
//   - Relay management (add/remove/rank relays)
//   - Gas sponsorship (sponsor accounts pay gas for users)
//   - Meta-transaction wrapping
//   - Relay health tracking

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GasStationError {
    #[error("relay not found: {0}")]
    RelayNotFound(String),
    #[error("relay already exists: {0}")]
    RelayExists(String),
    #[error("no available relays")]
    NoRelays,
    #[error("sponsor budget exhausted")]
    BudgetExhausted,
    #[error("invalid config: {0}")]
    InvalidConfig(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
}

// ── Relay ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelayStatus {
    Active,
    Degraded,
    Down,
    Banned,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relay {
    pub url: String,
    pub name: String,
    pub status: RelayStatus,
    pub success_count: u64,
    pub failure_count: u64,
    pub avg_latency_ms: u64,
    pub last_seen: Option<String>,
    pub added_at: String,
    pub fee_percent: f64,
    pub max_gas: u64,
    pub supported_tx_types: Vec<String>,
}

impl Relay {
    pub fn new(url: &str, name: &str) -> Self {
        Self {
            url: url.to_string(),
            name: name.to_string(),
            status: RelayStatus::Active,
            success_count: 0,
            failure_count: 0,
            avg_latency_ms: 0,
            last_seen: None,
            added_at: chrono::Utc::now().to_rfc3339(),
            fee_percent: 0.0,
            max_gas: 1_000_000,
            supported_tx_types: vec!["transfer".into(), "refresh".into(), "contract_call".into()],
        }
    }

    pub fn with_fee(mut self, fee_percent: f64) -> Self {
        self.fee_percent = fee_percent;
        self
    }

    pub fn with_max_gas(mut self, max_gas: u64) -> Self {
        self.max_gas = max_gas;
        self
    }

    pub fn reliability(&self) -> f64 {
        let total = self.success_count + self.failure_count;
        if total == 0 {
            return 1.0;
        }
        self.success_count as f64 / total as f64
    }

    pub fn record_success(&mut self, latency_ms: u64) {
        self.success_count += 1;
        self.last_seen = Some(chrono::Utc::now().to_rfc3339());
        // Rolling average
        let total = self.success_count + self.failure_count;
        self.avg_latency_ms =
            (self.avg_latency_ms * (total - 1) + latency_ms) / total;
    }

    pub fn record_failure(&mut self) {
        self.failure_count += 1;
        if self.reliability() < 0.5 {
            self.status = RelayStatus::Degraded;
        }
        if self.reliability() < 0.2 {
            self.status = RelayStatus::Down;
        }
    }

    pub fn ban(&mut self) {
        self.status = RelayStatus::Banned;
    }

    pub fn is_available(&self) -> bool {
        self.status == RelayStatus::Active || self.status == RelayStatus::Degraded
    }

    /// Score for relay selection (higher = better)
    pub fn score(&self) -> f64 {
        let reliability = self.reliability();
        let latency_factor = if self.avg_latency_ms > 0 {
            1000.0 / self.avg_latency_ms as f64
        } else {
            1.0
        };
        let fee_factor = 1.0 / (1.0 + self.fee_percent);
        reliability * latency_factor * fee_factor * 100.0
    }
}

// ── Sponsor ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasSponsor {
    pub address: String,
    pub name: String,
    pub budget: u64,
    pub spent: u64,
    pub per_tx_limit: u64,
    pub allowed_senders: Vec<String>,
    pub allowed_tx_types: Vec<String>,
    pub active: bool,
    pub created_at: String,
    pub tx_count: u64,
}

impl GasSponsor {
    pub fn new(address: &str, name: &str, budget: u64) -> Self {
        Self {
            address: address.to_string(),
            name: name.to_string(),
            budget,
            spent: 0,
            per_tx_limit: budget / 10, // Default 10% per tx
            allowed_senders: Vec::new(), // Empty = all allowed
            allowed_tx_types: Vec::new(), // Empty = all allowed
            active: true,
            created_at: chrono::Utc::now().to_rfc3339(),
            tx_count: 0,
        }
    }

    pub fn remaining(&self) -> u64 {
        self.budget.saturating_sub(self.spent)
    }

    pub fn can_sponsor(&self, sender: &str, tx_type: &str, gas_cost: u64) -> bool {
        if !self.active {
            return false;
        }
        if gas_cost > self.remaining() {
            return false;
        }
        if gas_cost > self.per_tx_limit {
            return false;
        }
        if !self.allowed_senders.is_empty() && !self.allowed_senders.contains(&sender.to_string()) {
            return false;
        }
        if !self.allowed_tx_types.is_empty()
            && !self.allowed_tx_types.contains(&tx_type.to_string())
        {
            return false;
        }
        true
    }

    pub fn spend(&mut self, amount: u64) -> Result<(), GasStationError> {
        if amount > self.remaining() {
            return Err(GasStationError::BudgetExhausted);
        }
        self.spent += amount;
        self.tx_count += 1;
        Ok(())
    }

    pub fn add_budget(&mut self, amount: u64) {
        self.budget += amount;
    }

    pub fn utilization_percent(&self) -> f64 {
        if self.budget == 0 {
            return 100.0;
        }
        (self.spent as f64 / self.budget as f64) * 100.0
    }
}

// ── Meta-transaction ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaTx {
    pub id: String,
    pub inner_tx_type: String,
    pub sender: String,
    pub relay_url: String,
    pub sponsor_address: Option<String>,
    pub gas_estimate: u64,
    pub status: MetaTxStatus,
    pub created_at: String,
    pub submitted_at: Option<String>,
    pub confirmed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetaTxStatus {
    Pending,
    Submitted,
    Confirmed,
    Failed,
}

// ── Gas station ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GasStation {
    pub relays: HashMap<String, Relay>,
    pub sponsors: HashMap<String, GasSponsor>,
    pub meta_txs: Vec<MetaTx>,
}

impl GasStation {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Relay management ──────────────────────────────────────

    pub fn add_relay(&mut self, relay: Relay) -> Result<(), GasStationError> {
        if self.relays.contains_key(&relay.url) {
            return Err(GasStationError::RelayExists(relay.url.clone()));
        }
        self.relays.insert(relay.url.clone(), relay);
        Ok(())
    }

    pub fn remove_relay(&mut self, url: &str) -> Result<Relay, GasStationError> {
        self.relays
            .remove(url)
            .ok_or_else(|| GasStationError::RelayNotFound(url.into()))
    }

    pub fn get_relay(&self, url: &str) -> Option<&Relay> {
        self.relays.get(url)
    }

    pub fn get_relay_mut(&mut self, url: &str) -> Option<&mut Relay> {
        self.relays.get_mut(url)
    }

    pub fn list_relays(&self) -> Vec<&Relay> {
        self.relays.values().collect()
    }

    pub fn available_relays(&self) -> Vec<&Relay> {
        self.relays.values().filter(|r| r.is_available()).collect()
    }

    /// Select best relay by score
    pub fn best_relay(&self) -> Option<&Relay> {
        self.available_relays()
            .into_iter()
            .max_by(|a, b| a.score().partial_cmp(&b.score()).unwrap_or(std::cmp::Ordering::Equal))
    }

    // ── Sponsor management ────────────────────────────────────

    pub fn add_sponsor(&mut self, sponsor: GasSponsor) {
        self.sponsors.insert(sponsor.address.clone(), sponsor);
    }

    pub fn get_sponsor(&self, address: &str) -> Option<&GasSponsor> {
        self.sponsors.get(address)
    }

    pub fn get_sponsor_mut(&mut self, address: &str) -> Option<&mut GasSponsor> {
        self.sponsors.get_mut(address)
    }

    pub fn list_sponsors(&self) -> Vec<&GasSponsor> {
        self.sponsors.values().collect()
    }

    /// Find a sponsor willing to pay for a transaction
    pub fn find_sponsor(&self, sender: &str, tx_type: &str, gas_cost: u64) -> Option<&GasSponsor> {
        self.sponsors
            .values()
            .find(|s| s.can_sponsor(sender, tx_type, gas_cost))
    }

    // ── Meta-tx tracking ──────────────────────────────────────

    pub fn record_meta_tx(&mut self, meta_tx: MetaTx) {
        self.meta_txs.push(meta_tx);
    }

    pub fn pending_meta_txs(&self) -> Vec<&MetaTx> {
        self.meta_txs
            .iter()
            .filter(|t| t.status == MetaTxStatus::Pending || t.status == MetaTxStatus::Submitted)
            .collect()
    }

    pub fn meta_tx_count(&self) -> usize {
        self.meta_txs.len()
    }

    // ── Persistence ───────────────────────────────────────────

    pub fn save(&self, path: &Path) -> Result<(), GasStationError> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| GasStationError::Json(e.to_string()))?;
        std::fs::write(path, json).map_err(|e| GasStationError::Io(e.to_string()))
    }

    pub fn load(path: &Path) -> Result<Self, GasStationError> {
        let data =
            std::fs::read_to_string(path).map_err(|e| GasStationError::Io(e.to_string()))?;
        serde_json::from_str(&data).map_err(|e| GasStationError::Json(e.to_string()))
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_relay(url: &str) -> Relay {
        Relay::new(url, &format!("relay-{}", url))
    }

    #[test]
    fn test_relay_new() {
        let r = make_relay("http://relay1.evap.io");
        assert_eq!(r.status, RelayStatus::Active);
        assert_eq!(r.reliability(), 1.0);
    }

    #[test]
    fn test_relay_with_fee() {
        let r = make_relay("http://r1").with_fee(1.5);
        assert_eq!(r.fee_percent, 1.5);
    }

    #[test]
    fn test_relay_record_success() {
        let mut r = make_relay("http://r1");
        r.record_success(100);
        r.record_success(200);
        assert_eq!(r.success_count, 2);
        assert!(r.last_seen.is_some());
        assert!(r.avg_latency_ms > 0);
    }

    #[test]
    fn test_relay_record_failure() {
        let mut r = make_relay("http://r1");
        r.record_success(100);
        r.record_failure();
        assert_eq!(r.failure_count, 1);
        assert_eq!(r.reliability(), 0.5);
    }

    #[test]
    fn test_relay_degraded() {
        let mut r = make_relay("http://r1");
        r.record_success(100);
        for _ in 0..3 {
            r.record_failure();
        }
        assert_eq!(r.status, RelayStatus::Degraded);
    }

    #[test]
    fn test_relay_down() {
        let mut r = make_relay("http://r1");
        for _ in 0..10 {
            r.record_failure();
        }
        assert_eq!(r.status, RelayStatus::Down);
        assert!(!r.is_available());
    }

    #[test]
    fn test_relay_ban() {
        let mut r = make_relay("http://r1");
        r.ban();
        assert_eq!(r.status, RelayStatus::Banned);
        assert!(!r.is_available());
    }

    #[test]
    fn test_relay_score() {
        let mut r1 = make_relay("http://r1");
        r1.record_success(100);
        let mut r2 = make_relay("http://r2").with_fee(5.0);
        r2.record_success(500);
        // r1 should score higher (lower latency, no fee)
        assert!(r1.score() > r2.score());
    }

    #[test]
    fn test_sponsor_new() {
        let s = GasSponsor::new("evap1sponsor", "My Sponsor", 10000);
        assert_eq!(s.budget, 10000);
        assert_eq!(s.remaining(), 10000);
        assert!(s.active);
    }

    #[test]
    fn test_sponsor_can_sponsor() {
        let s = GasSponsor::new("evap1s", "S", 10000);
        assert!(s.can_sponsor("anyone", "transfer", 500));
        assert!(!s.can_sponsor("anyone", "transfer", 20000)); // Over budget
    }

    #[test]
    fn test_sponsor_per_tx_limit() {
        let s = GasSponsor::new("evap1s", "S", 10000);
        // Default per_tx_limit = 10000/10 = 1000
        assert!(s.can_sponsor("a", "transfer", 1000));
        assert!(!s.can_sponsor("a", "transfer", 1001));
    }

    #[test]
    fn test_sponsor_allowed_senders() {
        let mut s = GasSponsor::new("evap1s", "S", 10000);
        s.allowed_senders = vec!["evap1alice".into()];
        assert!(s.can_sponsor("evap1alice", "transfer", 100));
        assert!(!s.can_sponsor("evap1bob", "transfer", 100));
    }

    #[test]
    fn test_sponsor_spend() {
        let mut s = GasSponsor::new("evap1s", "S", 10000);
        s.spend(500).unwrap();
        assert_eq!(s.spent, 500);
        assert_eq!(s.remaining(), 9500);
        assert_eq!(s.tx_count, 1);
    }

    #[test]
    fn test_sponsor_budget_exhausted() {
        let mut s = GasSponsor::new("evap1s", "S", 100);
        assert!(s.spend(200).is_err());
    }

    #[test]
    fn test_sponsor_add_budget() {
        let mut s = GasSponsor::new("evap1s", "S", 100);
        s.spend(100).unwrap();
        s.add_budget(500);
        assert_eq!(s.budget, 600);
        assert_eq!(s.remaining(), 500);
    }

    #[test]
    fn test_sponsor_utilization() {
        let mut s = GasSponsor::new("evap1s", "S", 1000);
        s.spend(250).unwrap();
        assert_eq!(s.utilization_percent(), 25.0);
    }

    #[test]
    fn test_gas_station_add_relay() {
        let mut gs = GasStation::new();
        gs.add_relay(make_relay("http://r1")).unwrap();
        assert_eq!(gs.list_relays().len(), 1);
    }

    #[test]
    fn test_gas_station_add_duplicate_relay() {
        let mut gs = GasStation::new();
        gs.add_relay(make_relay("http://r1")).unwrap();
        assert!(gs.add_relay(make_relay("http://r1")).is_err());
    }

    #[test]
    fn test_gas_station_remove_relay() {
        let mut gs = GasStation::new();
        gs.add_relay(make_relay("http://r1")).unwrap();
        gs.remove_relay("http://r1").unwrap();
        assert!(gs.list_relays().is_empty());
    }

    #[test]
    fn test_gas_station_best_relay() {
        let mut gs = GasStation::new();
        let mut r1 = make_relay("http://r1");
        r1.record_success(50);
        let mut r2 = make_relay("http://r2").with_fee(5.0);
        r2.record_success(500);
        gs.add_relay(r1).unwrap();
        gs.add_relay(r2).unwrap();
        let best = gs.best_relay().unwrap();
        assert_eq!(best.url, "http://r1");
    }

    #[test]
    fn test_gas_station_no_relays() {
        let gs = GasStation::new();
        assert!(gs.best_relay().is_none());
    }

    #[test]
    fn test_gas_station_find_sponsor() {
        let mut gs = GasStation::new();
        gs.add_sponsor(GasSponsor::new("evap1s", "S", 10000));
        let s = gs.find_sponsor("anyone", "transfer", 100);
        assert!(s.is_some());
    }

    #[test]
    fn test_gas_station_meta_tx() {
        let mut gs = GasStation::new();
        gs.record_meta_tx(MetaTx {
            id: "mt1".into(),
            inner_tx_type: "transfer".into(),
            sender: "evap1a".into(),
            relay_url: "http://r1".into(),
            sponsor_address: None,
            gas_estimate: 21000,
            status: MetaTxStatus::Pending,
            created_at: chrono::Utc::now().to_rfc3339(),
            submitted_at: None,
            confirmed_at: None,
        });
        assert_eq!(gs.meta_tx_count(), 1);
        assert_eq!(gs.pending_meta_txs().len(), 1);
    }

    #[test]
    fn test_gas_station_save_load() {
        let path = std::env::temp_dir().join(format!(
            "evap_gasstation_{}.json", std::process::id()
        ));
        let mut gs = GasStation::new();
        gs.add_relay(make_relay("http://r1")).unwrap();
        gs.add_sponsor(GasSponsor::new("evap1s", "S", 5000));
        gs.save(&path).unwrap();
        let loaded = GasStation::load(&path).unwrap();
        assert_eq!(loaded.list_relays().len(), 1);
        assert_eq!(loaded.list_sponsors().len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_gas_station_load_or_default() {
        let gs = GasStation::load_or_default(Path::new("/tmp/noexist_gs.json"));
        assert!(gs.relays.is_empty());
    }
}
