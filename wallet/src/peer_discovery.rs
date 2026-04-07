//! Peer Discovery — node peer scoring and RPC failover system.
//!
//! Tracks RPC nodes, scores them by reliability and latency,
//! auto-selects the best available peer, and handles failover
//! when nodes degrade or go down.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Clone, thiserror::Error)]
pub enum PeerDiscoveryError {
    #[error("peer already exists: {0}")]
    AlreadyExists(String),
    #[error("peer not found: {0}")]
    NotFound(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json parse error: {0}")]
    Parse(String),
}

impl From<std::io::Error> for PeerDiscoveryError {
    fn from(e: std::io::Error) -> Self {
        PeerDiscoveryError::Io(e.to_string())
    }
}

impl From<serde_json::Error> for PeerDiscoveryError {
    fn from(e: serde_json::Error) -> Self {
        PeerDiscoveryError::Parse(e.to_string())
    }
}

// ──────────────────────────── Enums ────────────────────────────────────

/// Peer operational status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeerStatus {
    Active,
    Degraded,
    Down,
    Banned,
}

/// Peer geographic region classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeerRegion {
    Local,
    NearBy,
    Remote,
    Unknown,
}

// ──────────────────────────── Peer ─────────────────────────────────────

/// A single RPC peer node with scoring metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Peer {
    pub url: String,
    pub name: String,
    pub status: PeerStatus,
    pub region: PeerRegion,
    pub latency_ms: u64,
    pub success_count: u64,
    pub failure_count: u64,
    pub consecutive_failures: u32,
    pub max_consecutive_failures: u32,
    pub last_success: Option<String>,
    pub last_failure: Option<String>,
    pub added_at: String,
    pub chain_id: String,
    pub block_height: u64,
    pub version: String,
    pub features: Vec<String>,
    pub weight: u32,
    pub banned_until: Option<String>,
}

impl Peer {
    /// Create a new peer with sensible defaults.
    pub fn new(url: &str, name: &str) -> Self {
        Self {
            url: url.to_string(),
            name: name.to_string(),
            status: PeerStatus::Active,
            region: PeerRegion::Unknown,
            latency_ms: 0,
            success_count: 0,
            failure_count: 0,
            consecutive_failures: 0,
            max_consecutive_failures: 5,
            last_success: None,
            last_failure: None,
            added_at: chrono::Utc::now().to_rfc3339(),
            chain_id: String::new(),
            block_height: 0,
            version: String::new(),
            features: Vec::new(),
            weight: 100,
            banned_until: None,
        }
    }

    /// Builder: set region.
    pub fn with_region(mut self, r: PeerRegion) -> Self {
        self.region = r;
        self
    }

    /// Builder: set manual priority weight.
    pub fn with_weight(mut self, w: u32) -> Self {
        self.weight = w;
        self
    }

    /// Reliability ratio: success / (success + failure). Returns 1.0 if no calls.
    pub fn reliability(&self) -> f64 {
        let total = self.success_count + self.failure_count;
        if total == 0 {
            return 1.0;
        }
        self.success_count as f64 / total as f64
    }

    /// Composite score: higher is better.
    /// `reliability * (1000 / max(latency_ms, 1)) * (weight / 100)`
    pub fn score(&self) -> f64 {
        self.reliability()
            * (1000.0 / (self.latency_ms.max(1) as f64))
            * (self.weight as f64 / 100.0)
    }

    /// Record a successful RPC call with measured latency.
    pub fn record_success(&mut self, latency_ms: u64) {
        if self.success_count == 0 && self.latency_ms == 0 {
            self.latency_ms = latency_ms;
        } else {
            self.latency_ms =
                (self.latency_ms as f64 * 0.7 + latency_ms as f64 * 0.3) as u64;
        }
        self.success_count += 1;
        self.consecutive_failures = 0;
        self.last_success = Some(chrono::Utc::now().to_rfc3339());
        self.status = PeerStatus::Active;
    }

    /// Record a failed RPC call.
    pub fn record_failure(&mut self) {
        self.failure_count += 1;
        self.consecutive_failures += 1;
        self.last_failure = Some(chrono::Utc::now().to_rfc3339());
        if self.consecutive_failures >= self.max_consecutive_failures {
            self.status = PeerStatus::Banned;
            let ban_until = chrono::Utc::now() + chrono::Duration::minutes(5);
            self.banned_until = Some(ban_until.to_rfc3339());
        }
    }

    /// Whether this peer can be selected for requests.
    pub fn is_available(&self) -> bool {
        match self.status {
            PeerStatus::Active | PeerStatus::Degraded => {
                if let Some(ref until) = self.banned_until {
                    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(until) {
                        return chrono::Utc::now() > t;
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// Clear ban, reset to Degraded.
    pub fn unban(&mut self) {
        self.banned_until = None;
        self.status = PeerStatus::Degraded;
        self.consecutive_failures = 0;
    }
}

// ──────────────────────────── PeerStats ────────────────────────────────

/// Aggregate statistics about the peer registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerStats {
    pub total: u64,
    pub active: u64,
    pub degraded: u64,
    pub down: u64,
    pub banned: u64,
    pub avg_latency_ms: u64,
    pub failover_count: u64,
}

// ──────────────────────────── PeerRegistry ─────────────────────────────

/// Registry of RPC peers with scoring and failover.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerRegistry {
    pub peers: HashMap<String, Peer>,
    pub preferred_region: PeerRegion,
    pub last_failover: Option<String>,
    pub failover_count: u64,
    #[serde(skip)]
    last_selected: Option<String>,
}

impl Default for PeerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PeerRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            peers: HashMap::new(),
            preferred_region: PeerRegion::Unknown,
            last_failover: None,
            failover_count: 0,
            last_selected: None,
        }
    }

    /// Add a peer. Fails if the URL is already registered.
    pub fn add_peer(&mut self, peer: Peer) -> Result<(), PeerDiscoveryError> {
        if self.peers.contains_key(&peer.url) {
            return Err(PeerDiscoveryError::AlreadyExists(peer.url.clone()));
        }
        self.peers.insert(peer.url.clone(), peer);
        Ok(())
    }

    /// Remove a peer by URL.
    pub fn remove_peer(&mut self, url: &str) -> Result<Peer, PeerDiscoveryError> {
        self.peers
            .remove(url)
            .ok_or_else(|| PeerDiscoveryError::NotFound(url.to_string()))
    }

    /// Get an immutable reference to a peer.
    pub fn get(&self, url: &str) -> Option<&Peer> {
        self.peers.get(url)
    }

    /// Get a mutable reference to a peer.
    pub fn get_mut(&mut self, url: &str) -> Option<&mut Peer> {
        self.peers.get_mut(url)
    }

    /// List all peers.
    pub fn list(&self) -> Vec<&Peer> {
        self.peers.values().collect()
    }

    /// List only available peers.
    pub fn available(&self) -> Vec<&Peer> {
        self.peers.values().filter(|p| p.is_available()).collect()
    }

    /// Best peer by score among available.
    pub fn best_peer(&self) -> Option<&Peer> {
        self.available()
            .into_iter()
            .max_by(|a, b| a.score().partial_cmp(&b.score()).unwrap_or(std::cmp::Ordering::Equal))
    }

    /// Select the best peer URL. If it differs from the last selection,
    /// increment failover_count and record timestamp.
    pub fn select_peer(&mut self) -> Option<String> {
        let best = self
            .available()
            .into_iter()
            .max_by(|a, b| a.score().partial_cmp(&b.score()).unwrap_or(std::cmp::Ordering::Equal))
            .map(|p| p.url.clone());

        if let Some(ref url) = best {
            if self.last_selected.as_ref() != Some(url) {
                if self.last_selected.is_some() {
                    self.failover_count += 1;
                    self.last_failover = Some(chrono::Utc::now().to_rfc3339());
                }
                self.last_selected = Some(url.clone());
            }
        }
        best
    }

    /// Peers in a specific region.
    pub fn by_region(&self, region: &PeerRegion) -> Vec<&Peer> {
        self.peers
            .values()
            .filter(|p| &p.region == region)
            .collect()
    }

    /// Peers with a specific status.
    pub fn by_status(&self, status: &PeerStatus) -> Vec<&Peer> {
        self.peers
            .values()
            .filter(|p| &p.status == status)
            .collect()
    }

    /// All currently banned peers.
    pub fn banned_peers(&self) -> Vec<&Peer> {
        self.by_status(&PeerStatus::Banned)
    }

    /// Unban every banned peer. Returns the number unbanned.
    pub fn unban_all(&mut self) -> usize {
        let urls: Vec<String> = self
            .peers
            .values()
            .filter(|p| p.status == PeerStatus::Banned)
            .map(|p| p.url.clone())
            .collect();
        let count = urls.len();
        for url in urls {
            if let Some(p) = self.peers.get_mut(&url) {
                p.unban();
            }
        }
        count
    }

    /// Record a successful call against a peer.
    pub fn record_success(
        &mut self,
        url: &str,
        latency_ms: u64,
    ) -> Result<(), PeerDiscoveryError> {
        self.peers
            .get_mut(url)
            .ok_or_else(|| PeerDiscoveryError::NotFound(url.to_string()))?
            .record_success(latency_ms);
        Ok(())
    }

    /// Record a failed call against a peer.
    pub fn record_failure(&mut self, url: &str) -> Result<(), PeerDiscoveryError> {
        self.peers
            .get_mut(url)
            .ok_or_else(|| PeerDiscoveryError::NotFound(url.to_string()))?
            .record_failure();
        Ok(())
    }

    /// Aggregate stats across all peers.
    pub fn stats(&self) -> PeerStats {
        let total = self.peers.len() as u64;
        let active = self.by_status(&PeerStatus::Active).len() as u64;
        let degraded = self.by_status(&PeerStatus::Degraded).len() as u64;
        let down = self.by_status(&PeerStatus::Down).len() as u64;
        let banned = self.by_status(&PeerStatus::Banned).len() as u64;

        let latencies: Vec<u64> = self
            .peers
            .values()
            .filter(|p| p.latency_ms > 0)
            .map(|p| p.latency_ms)
            .collect();
        let avg_latency_ms = if latencies.is_empty() {
            0
        } else {
            latencies.iter().sum::<u64>() / latencies.len() as u64
        };

        PeerStats {
            total,
            active,
            degraded,
            down,
            banned,
            avg_latency_ms,
            failover_count: self.failover_count,
        }
    }

    /// Top N peers by score (available only).
    pub fn top_n(&self, n: usize) -> Vec<&Peer> {
        let mut avail = self.available();
        avail.sort_by(|a, b| {
            b.score()
                .partial_cmp(&a.score())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        avail.into_iter().take(n).collect()
    }

    /// Remove all peers with status Down. Returns the count removed.
    pub fn prune_down(&mut self) -> usize {
        let urls: Vec<String> = self
            .peers
            .values()
            .filter(|p| p.status == PeerStatus::Down)
            .map(|p| p.url.clone())
            .collect();
        let count = urls.len();
        for url in urls {
            self.peers.remove(&url);
        }
        count
    }

    /// Count of peers grouped by status name.
    pub fn health_summary(&self) -> HashMap<String, usize> {
        let mut map = HashMap::new();
        map.insert("Active".to_string(), 0);
        map.insert("Degraded".to_string(), 0);
        map.insert("Down".to_string(), 0);
        map.insert("Banned".to_string(), 0);
        for peer in self.peers.values() {
            let key = match peer.status {
                PeerStatus::Active => "Active",
                PeerStatus::Degraded => "Degraded",
                PeerStatus::Down => "Down",
                PeerStatus::Banned => "Banned",
            };
            *map.get_mut(key).unwrap() += 1;
        }
        map
    }

    // ── Persistence ───────────────────────────────────────────────────

    /// Save registry to disk as pretty JSON.
    pub fn save(&self, path: &Path) -> Result<(), PeerDiscoveryError> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| PeerDiscoveryError::Parse(e.to_string()))?;
        std::fs::write(path, json).map_err(|e| PeerDiscoveryError::Io(e.to_string()))
    }

    /// Load registry from disk.
    pub fn load(path: &Path) -> Result<Self, PeerDiscoveryError> {
        let data =
            std::fs::read_to_string(path).map_err(|e| PeerDiscoveryError::Io(e.to_string()))?;
        serde_json::from_str(&data).map_err(|e| PeerDiscoveryError::Parse(e.to_string()))
    }

    /// Load from disk or return default if file is missing / corrupt.
    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ──────────────────────────── Tests ────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "peer_disc_test_{}_{name}",
            std::process::id()
        ))
    }

    fn make_peer(url: &str, name: &str) -> Peer {
        Peer::new(url, name)
    }

    #[test]
    fn test_add_and_get_peer() {
        let mut reg = PeerRegistry::new();
        let p = make_peer("https://rpc1.example.com", "rpc1");
        reg.add_peer(p).unwrap();
        let got = reg.get("https://rpc1.example.com").unwrap();
        assert_eq!(got.name, "rpc1");
        assert_eq!(got.status, PeerStatus::Active);
    }

    #[test]
    fn test_add_duplicate_rejected() {
        let mut reg = PeerRegistry::new();
        reg.add_peer(make_peer("https://rpc1.example.com", "rpc1"))
            .unwrap();
        let err = reg
            .add_peer(make_peer("https://rpc1.example.com", "rpc1-dup"))
            .unwrap_err();
        assert!(matches!(err, PeerDiscoveryError::AlreadyExists(_)));
    }

    #[test]
    fn test_remove_peer() {
        let mut reg = PeerRegistry::new();
        reg.add_peer(make_peer("https://rpc1.example.com", "rpc1"))
            .unwrap();
        let removed = reg.remove_peer("https://rpc1.example.com").unwrap();
        assert_eq!(removed.name, "rpc1");
        assert!(reg.get("https://rpc1.example.com").is_none());
    }

    #[test]
    fn test_remove_not_found() {
        let mut reg = PeerRegistry::new();
        let err = reg.remove_peer("https://nonexistent.com").unwrap_err();
        assert!(matches!(err, PeerDiscoveryError::NotFound(_)));
    }

    #[test]
    fn test_record_success_updates_latency() {
        let mut p = make_peer("https://rpc1.example.com", "rpc1");
        // First success sets latency directly.
        p.record_success(100);
        assert_eq!(p.latency_ms, 100);
        assert_eq!(p.success_count, 1);
        // Second success uses weighted average: 100*0.7 + 200*0.3 = 130
        p.record_success(200);
        assert_eq!(p.latency_ms, 130);
        assert_eq!(p.success_count, 2);
    }

    #[test]
    fn test_record_failure_increments() {
        let mut p = make_peer("https://rpc1.example.com", "rpc1");
        p.record_failure();
        assert_eq!(p.failure_count, 1);
        assert_eq!(p.consecutive_failures, 1);
        assert!(p.last_failure.is_some());
    }

    #[test]
    fn test_auto_ban_on_consecutive_failures() {
        let mut p = make_peer("https://rpc1.example.com", "rpc1");
        assert_eq!(p.max_consecutive_failures, 5);
        for _ in 0..5 {
            p.record_failure();
        }
        assert_eq!(p.status, PeerStatus::Banned);
        assert!(p.banned_until.is_some());
    }

    #[test]
    fn test_is_available_when_banned() {
        let mut p = make_peer("https://rpc1.example.com", "rpc1");
        for _ in 0..5 {
            p.record_failure();
        }
        assert!(!p.is_available());
    }

    #[test]
    fn test_unban() {
        let mut p = make_peer("https://rpc1.example.com", "rpc1");
        for _ in 0..5 {
            p.record_failure();
        }
        assert_eq!(p.status, PeerStatus::Banned);
        p.unban();
        assert_eq!(p.status, PeerStatus::Degraded);
        assert!(p.banned_until.is_none());
        assert_eq!(p.consecutive_failures, 0);
    }

    #[test]
    fn test_unban_all() {
        let mut reg = PeerRegistry::new();
        let mut p1 = make_peer("https://rpc1.example.com", "rpc1");
        let mut p2 = make_peer("https://rpc2.example.com", "rpc2");
        for _ in 0..5 {
            p1.record_failure();
            p2.record_failure();
        }
        reg.add_peer(p1).unwrap();
        reg.add_peer(p2).unwrap();
        assert_eq!(reg.banned_peers().len(), 2);
        let count = reg.unban_all();
        assert_eq!(count, 2);
        assert_eq!(reg.banned_peers().len(), 0);
    }

    #[test]
    fn test_best_peer() {
        let mut reg = PeerRegistry::new();
        let mut p1 = make_peer("https://rpc1.example.com", "rpc1");
        let mut p2 = make_peer("https://rpc2.example.com", "rpc2");
        p1.record_success(100);
        p2.record_success(200);
        reg.add_peer(p1).unwrap();
        reg.add_peer(p2).unwrap();
        let best = reg.best_peer().unwrap();
        // p1 has lower latency -> higher score
        assert_eq!(best.url, "https://rpc1.example.com");
    }

    #[test]
    fn test_best_peer_prefers_low_latency() {
        let mut reg = PeerRegistry::new();
        let mut p1 = make_peer("https://fast.example.com", "fast");
        let mut p2 = make_peer("https://slow.example.com", "slow");
        p1.record_success(10);
        p2.record_success(500);
        reg.add_peer(p1).unwrap();
        reg.add_peer(p2).unwrap();
        let best = reg.best_peer().unwrap();
        assert_eq!(best.url, "https://fast.example.com");
    }

    #[test]
    fn test_best_peer_considers_weight() {
        let mut reg = PeerRegistry::new();
        let mut p1 = make_peer("https://rpc1.example.com", "rpc1");
        let mut p2 = Peer::new("https://rpc2.example.com", "rpc2").with_weight(300);
        // Same latency
        p1.record_success(100);
        p2.record_success(100);
        reg.add_peer(p1).unwrap();
        reg.add_peer(p2).unwrap();
        let best = reg.best_peer().unwrap();
        assert_eq!(best.url, "https://rpc2.example.com");
    }

    #[test]
    fn test_available_excludes_down_and_banned() {
        let mut reg = PeerRegistry::new();
        let mut p1 = make_peer("https://rpc1.example.com", "active");
        p1.record_success(50);
        let mut p2 = make_peer("https://rpc2.example.com", "down");
        p2.status = PeerStatus::Down;
        let mut p3 = make_peer("https://rpc3.example.com", "banned");
        for _ in 0..5 {
            p3.record_failure();
        }
        reg.add_peer(p1).unwrap();
        reg.add_peer(p2).unwrap();
        reg.add_peer(p3).unwrap();
        let avail = reg.available();
        assert_eq!(avail.len(), 1);
        assert_eq!(avail[0].url, "https://rpc1.example.com");
    }

    #[test]
    fn test_by_region() {
        let mut reg = PeerRegistry::new();
        reg.add_peer(make_peer("https://local.example.com", "local").with_region(PeerRegion::Local))
            .unwrap();
        reg.add_peer(
            make_peer("https://remote.example.com", "remote").with_region(PeerRegion::Remote),
        )
        .unwrap();
        assert_eq!(reg.by_region(&PeerRegion::Local).len(), 1);
        assert_eq!(reg.by_region(&PeerRegion::Remote).len(), 1);
        assert_eq!(reg.by_region(&PeerRegion::NearBy).len(), 0);
    }

    #[test]
    fn test_by_status() {
        let mut reg = PeerRegistry::new();
        let mut p1 = make_peer("https://rpc1.example.com", "active");
        p1.record_success(50);
        let mut p2 = make_peer("https://rpc2.example.com", "degraded");
        p2.status = PeerStatus::Degraded;
        reg.add_peer(p1).unwrap();
        reg.add_peer(p2).unwrap();
        assert_eq!(reg.by_status(&PeerStatus::Active).len(), 1);
        assert_eq!(reg.by_status(&PeerStatus::Degraded).len(), 1);
    }

    #[test]
    fn test_top_n() {
        let mut reg = PeerRegistry::new();
        for i in 0..5 {
            let mut p = make_peer(&format!("https://rpc{i}.example.com"), &format!("rpc{i}"));
            p.record_success((i as u64 + 1) * 50);
            reg.add_peer(p).unwrap();
        }
        let top = reg.top_n(3);
        assert_eq!(top.len(), 3);
        // Sorted by score descending — lowest latency first.
        assert!(top[0].score() >= top[1].score());
        assert!(top[1].score() >= top[2].score());
    }

    #[test]
    fn test_prune_down() {
        let mut reg = PeerRegistry::new();
        let mut p1 = make_peer("https://rpc1.example.com", "active");
        p1.record_success(50);
        let mut p2 = make_peer("https://rpc2.example.com", "down");
        p2.status = PeerStatus::Down;
        let mut p3 = make_peer("https://rpc3.example.com", "down2");
        p3.status = PeerStatus::Down;
        reg.add_peer(p1).unwrap();
        reg.add_peer(p2).unwrap();
        reg.add_peer(p3).unwrap();
        let pruned = reg.prune_down();
        assert_eq!(pruned, 2);
        assert_eq!(reg.peers.len(), 1);
    }

    #[test]
    fn test_health_summary() {
        let mut reg = PeerRegistry::new();
        let mut p1 = make_peer("https://rpc1.example.com", "active");
        p1.record_success(50);
        let mut p2 = make_peer("https://rpc2.example.com", "degraded");
        p2.status = PeerStatus::Degraded;
        let mut p3 = make_peer("https://rpc3.example.com", "down");
        p3.status = PeerStatus::Down;
        reg.add_peer(p1).unwrap();
        reg.add_peer(p2).unwrap();
        reg.add_peer(p3).unwrap();
        let summary = reg.health_summary();
        assert_eq!(summary["Active"], 1);
        assert_eq!(summary["Degraded"], 1);
        assert_eq!(summary["Down"], 1);
        assert_eq!(summary["Banned"], 0);
    }

    #[test]
    fn test_stats() {
        let mut reg = PeerRegistry::new();
        let mut p1 = make_peer("https://rpc1.example.com", "rpc1");
        p1.record_success(100);
        let mut p2 = make_peer("https://rpc2.example.com", "rpc2");
        p2.record_success(200);
        reg.add_peer(p1).unwrap();
        reg.add_peer(p2).unwrap();
        let stats = reg.stats();
        assert_eq!(stats.total, 2);
        assert_eq!(stats.active, 2);
        assert_eq!(stats.avg_latency_ms, 150);
    }

    #[test]
    fn test_select_peer_failover_count() {
        let mut reg = PeerRegistry::new();
        let mut p1 = make_peer("https://rpc1.example.com", "rpc1");
        let mut p2 = make_peer("https://rpc2.example.com", "rpc2");
        p1.record_success(100);
        p2.record_success(200);
        reg.add_peer(p1).unwrap();
        reg.add_peer(p2).unwrap();

        // First select — no failover yet (no previous selection).
        let first = reg.select_peer().unwrap();
        assert_eq!(first, "https://rpc1.example.com");
        assert_eq!(reg.failover_count, 0);

        // Ban p1 to force failover to p2.
        for _ in 0..5 {
            reg.record_failure("https://rpc1.example.com").unwrap();
        }
        let second = reg.select_peer().unwrap();
        assert_eq!(second, "https://rpc2.example.com");
        assert_eq!(reg.failover_count, 1);
        assert!(reg.last_failover.is_some());
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path("roundtrip.json");
        let mut reg = PeerRegistry::new();
        let mut p = make_peer("https://rpc1.example.com", "rpc1");
        p.record_success(120);
        reg.add_peer(p).unwrap();
        reg.save(&path).unwrap();

        let loaded = PeerRegistry::load(&path).unwrap();
        assert_eq!(loaded.peers.len(), 1);
        let peer = loaded.get("https://rpc1.example.com").unwrap();
        assert_eq!(peer.latency_ms, 120);
        assert_eq!(peer.success_count, 1);

        // Cleanup
        let _ = std::fs::remove_file(&path);
    }
}
