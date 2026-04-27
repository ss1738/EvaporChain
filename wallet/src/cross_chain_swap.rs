//! Cross-chain atomic swap orchestrator.
//!
//! Manages swap routes across supported chains, orchestrates atomic swaps
//! with hashlock/timelock semantics, tracks swap lifecycle, and computes
//! slippage and volume statistics.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum CrossChainError {
    #[error("route not found: {0}")]
    RouteNotFound(String),
    #[error("swap not found: {0}")]
    SwapNotFound(String),
    #[error("duplicate route: {0}")]
    DuplicateRoute(String),
    #[error("invalid status transition for swap {0}")]
    InvalidTransition(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ──────────────────────────── Types ──────────────────────────────────────

/// Supported chain identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChainId {
    EvaporChain,
    Ethereum,
    Solana,
    Bitcoin,
    Polygon,
    Arbitrum,
    Custom(String),
}

/// Status of a cross-chain swap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SwapStatus {
    Pending,
    Locked,
    Completed,
    Refunded,
    Expired,
    Failed,
}

/// A swap route describing how to move tokens between two chains.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapRoute {
    pub id: String,
    pub source_chain: ChainId,
    pub dest_chain: ChainId,
    pub source_token: String,
    pub dest_token: String,
    pub exchange_rate: f64,
    pub fee_bps: u32,
    pub estimated_time_secs: u64,
    pub provider: String,
    pub max_slippage_bps: u32,
}

/// A cross-chain atomic swap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossChainSwap {
    pub id: String,
    pub route_id: String,
    pub source_amount: u64,
    pub expected_output: u64,
    pub actual_output: Option<u64>,
    pub status: SwapStatus,
    pub hashlock: String,
    pub timelock_expiry: String,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub source_tx: Option<String>,
    pub dest_tx: Option<String>,
    pub slippage_bps: Option<u32>,
}

/// Aggregate swap statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapStats {
    pub total_swaps: usize,
    pub completed: usize,
    pub pending: usize,
    pub failed: usize,
    pub refunded: usize,
    pub total_volume_in: u64,
    pub total_volume_out: u64,
    pub total_routes: usize,
    pub avg_slippage_bps: u32,
}

// ──────────────────────────── Manager ──────────────────────────────────

/// Orchestrates cross-chain atomic swaps.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CrossChainManager {
    pub routes: HashMap<String, SwapRoute>,
    pub swaps: HashMap<String, CrossChainSwap>,
}

impl CrossChainManager {
    /// Create a new empty manager.
    pub fn new() -> Self {
        Self {
            routes: HashMap::new(),
            swaps: HashMap::new(),
        }
    }

    // ── Route management ────────────────────────────────────────────

    /// Register a new swap route. Errors if the route ID already exists.
    pub fn add_route(&mut self, route: SwapRoute) -> Result<(), CrossChainError> {
        if self.routes.contains_key(&route.id) {
            return Err(CrossChainError::DuplicateRoute(route.id.clone()));
        }
        self.routes.insert(route.id.clone(), route);
        Ok(())
    }

    /// Remove a route by ID.
    pub fn remove_route(&mut self, id: &str) -> Result<SwapRoute, CrossChainError> {
        self.routes
            .remove(id)
            .ok_or_else(|| CrossChainError::RouteNotFound(id.to_string()))
    }

    /// Look up a route by ID.
    pub fn get_route(&self, id: &str) -> Option<&SwapRoute> {
        self.routes.get(id)
    }

    /// List all registered routes.
    pub fn list_routes(&self) -> Vec<&SwapRoute> {
        self.routes.values().collect()
    }

    /// Find routes matching a source and destination chain.
    pub fn find_routes(&self, source_chain: &ChainId, dest_chain: &ChainId) -> Vec<&SwapRoute> {
        self.routes
            .values()
            .filter(|r| r.source_chain == *source_chain && r.dest_chain == *dest_chain)
            .collect()
    }

    /// Return the best (lowest fee) route for a given chain pair.
    pub fn best_route(&self, source_chain: &ChainId, dest_chain: &ChainId) -> Option<&SwapRoute> {
        self.find_routes(source_chain, dest_chain)
            .into_iter()
            .min_by_key(|r| r.fee_bps)
    }

    // ── Swap lifecycle ──────────────────────────────────────────────

    /// Initiate a new swap. Returns the swap ID.
    pub fn initiate_swap(
        &mut self,
        route_id: &str,
        source_amount: u64,
    ) -> Result<String, CrossChainError> {
        let route = self
            .routes
            .get(route_id)
            .ok_or_else(|| CrossChainError::RouteNotFound(route_id.to_string()))?;

        // Compute expected output: apply exchange rate then subtract fee.
        let gross = (source_amount as f64 * route.exchange_rate) as u64;
        let fee = gross * route.fee_bps as u64 / 10_000;
        let expected_output = gross - fee;

        // Generate hashlock from random-ish data using blake3.
        let now = chrono::Utc::now();
        let preimage = format!("{}-{}-{}-{}", route_id, source_amount, now.timestamp_nanos_opt().unwrap_or(0), expected_output);
        let hashlock = blake3::hash(preimage.as_bytes()).to_hex().to_string();

        let swap_id = format!("swap-{}", &hashlock[..16]);
        let timelock_expiry = (now + chrono::Duration::seconds(route.estimated_time_secs as i64 * 3))
            .to_rfc3339();

        let swap = CrossChainSwap {
            id: swap_id.clone(),
            route_id: route_id.to_string(),
            source_amount,
            expected_output,
            actual_output: None,
            status: SwapStatus::Pending,
            hashlock,
            timelock_expiry,
            created_at: now.to_rfc3339(),
            completed_at: None,
            source_tx: None,
            dest_tx: None,
            slippage_bps: None,
        };

        self.swaps.insert(swap_id.clone(), swap);
        Ok(swap_id)
    }

    /// Lock a pending swap.
    pub fn lock_swap(&mut self, swap_id: &str) -> Result<(), CrossChainError> {
        let swap = self
            .swaps
            .get_mut(swap_id)
            .ok_or_else(|| CrossChainError::SwapNotFound(swap_id.to_string()))?;
        if swap.status != SwapStatus::Pending {
            return Err(CrossChainError::InvalidTransition(swap_id.to_string()));
        }
        swap.status = SwapStatus::Locked;
        Ok(())
    }

    /// Complete a locked swap with the actual output amount and destination tx hash.
    pub fn complete_swap(
        &mut self,
        swap_id: &str,
        actual_output: u64,
        dest_tx: &str,
    ) -> Result<(), CrossChainError> {
        let swap = self
            .swaps
            .get_mut(swap_id)
            .ok_or_else(|| CrossChainError::SwapNotFound(swap_id.to_string()))?;
        if swap.status != SwapStatus::Locked {
            return Err(CrossChainError::InvalidTransition(swap_id.to_string()));
        }
        swap.status = SwapStatus::Completed;
        swap.actual_output = Some(actual_output);
        swap.dest_tx = Some(dest_tx.to_string());
        swap.completed_at = Some(chrono::Utc::now().to_rfc3339());
        swap.slippage_bps = Some(Self::calculate_slippage(swap.expected_output, actual_output));
        Ok(())
    }

    /// Refund a swap (must be Pending or Locked).
    pub fn refund_swap(&mut self, swap_id: &str) -> Result<(), CrossChainError> {
        let swap = self
            .swaps
            .get_mut(swap_id)
            .ok_or_else(|| CrossChainError::SwapNotFound(swap_id.to_string()))?;
        if swap.status != SwapStatus::Pending && swap.status != SwapStatus::Locked {
            return Err(CrossChainError::InvalidTransition(swap_id.to_string()));
        }
        swap.status = SwapStatus::Refunded;
        Ok(())
    }

    /// Mark a swap as failed (must be Pending or Locked).
    pub fn fail_swap(&mut self, swap_id: &str) -> Result<(), CrossChainError> {
        let swap = self
            .swaps
            .get_mut(swap_id)
            .ok_or_else(|| CrossChainError::SwapNotFound(swap_id.to_string()))?;
        if swap.status != SwapStatus::Pending && swap.status != SwapStatus::Locked {
            return Err(CrossChainError::InvalidTransition(swap_id.to_string()));
        }
        swap.status = SwapStatus::Failed;
        Ok(())
    }

    /// Look up a swap by ID.
    pub fn get_swap(&self, id: &str) -> Option<&CrossChainSwap> {
        self.swaps.get(id)
    }

    /// Return all active (Pending or Locked) swaps.
    pub fn active_swaps(&self) -> Vec<&CrossChainSwap> {
        self.swaps
            .values()
            .filter(|s| s.status == SwapStatus::Pending || s.status == SwapStatus::Locked)
            .collect()
    }

    /// Return all swaps sorted by created_at descending.
    pub fn swap_history(&self) -> Vec<&CrossChainSwap> {
        let mut swaps: Vec<&CrossChainSwap> = self.swaps.values().collect();
        swaps.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        swaps
    }

    /// Calculate slippage in basis points between expected and actual output.
    pub fn calculate_slippage(expected: u64, actual: u64) -> u32 {
        if expected == 0 {
            return 0;
        }
        let diff = actual.abs_diff(expected);
        ((diff as f64 / expected as f64) * 10_000.0).round() as u32
    }

    /// Compute aggregate statistics across all swaps and routes.
    pub fn stats(&self) -> SwapStats {
        let mut completed = 0usize;
        let mut pending = 0usize;
        let mut failed = 0usize;
        let mut refunded = 0usize;
        let mut total_volume_in = 0u64;
        let mut total_volume_out = 0u64;
        let mut slippage_sum = 0u64;
        let mut slippage_count = 0u32;

        for swap in self.swaps.values() {
            match swap.status {
                SwapStatus::Completed => {
                    completed += 1;
                    total_volume_in += swap.source_amount;
                    if let Some(out) = swap.actual_output {
                        total_volume_out += out;
                    }
                    if let Some(slip) = swap.slippage_bps {
                        slippage_sum += slip as u64;
                        slippage_count += 1;
                    }
                }
                SwapStatus::Pending => pending += 1,
                SwapStatus::Locked => pending += 1,
                SwapStatus::Failed => failed += 1,
                SwapStatus::Refunded => refunded += 1,
                SwapStatus::Expired => failed += 1,
            }
        }

        let avg_slippage_bps = if slippage_count > 0 {
            (slippage_sum / slippage_count as u64) as u32
        } else {
            0
        };

        SwapStats {
            total_swaps: self.swaps.len(),
            completed,
            pending,
            failed,
            refunded,
            total_volume_in,
            total_volume_out,
            total_routes: self.routes.len(),
            avg_slippage_bps,
        }
    }

    // ── Persistence ─────────────────────────────────────────────────

    /// Save the manager state to a JSON file.
    pub fn save(&self, path: &Path) -> Result<(), CrossChainError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load the manager state from a JSON file.
    pub fn load(path: &Path) -> Result<Self, CrossChainError> {
        let data = std::fs::read_to_string(path)?;
        let manager: Self = serde_json::from_str(&data)?;
        Ok(manager)
    }

    /// Load from file or return a default instance if the file cannot be read.
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
            "cross_chain_swap_test_{}_{}.json",
            name,
            std::process::id()
        ))
    }

    fn sample_route(id: &str, fee_bps: u32) -> SwapRoute {
        SwapRoute {
            id: id.to_string(),
            source_chain: ChainId::EvaporChain,
            dest_chain: ChainId::Ethereum,
            source_token: "EVAP".to_string(),
            dest_token: "ETH".to_string(),
            exchange_rate: 0.5,
            fee_bps,
            estimated_time_secs: 120,
            provider: "TestProvider".to_string(),
            max_slippage_bps: 100,
        }
    }

    fn sample_route_chains(id: &str, src: ChainId, dst: ChainId) -> SwapRoute {
        SwapRoute {
            id: id.to_string(),
            source_chain: src,
            dest_chain: dst,
            source_token: "TOK_A".to_string(),
            dest_token: "TOK_B".to_string(),
            exchange_rate: 1.0,
            fee_bps: 50,
            estimated_time_secs: 60,
            provider: "Provider".to_string(),
            max_slippage_bps: 200,
        }
    }

    #[test]
    fn test_add_route() {
        let mut mgr = CrossChainManager::new();
        assert!(mgr.add_route(sample_route("r1", 30)).is_ok());
        assert_eq!(mgr.routes.len(), 1);
        assert!(mgr.get_route("r1").is_some());
    }

    #[test]
    fn test_duplicate_route() {
        let mut mgr = CrossChainManager::new();
        mgr.add_route(sample_route("r1", 30)).unwrap();
        let err = mgr.add_route(sample_route("r1", 50));
        assert!(err.is_err());
        assert!(matches!(err.unwrap_err(), CrossChainError::DuplicateRoute(_)));
    }

    #[test]
    fn test_remove_route() {
        let mut mgr = CrossChainManager::new();
        mgr.add_route(sample_route("r1", 30)).unwrap();
        let removed = mgr.remove_route("r1").unwrap();
        assert_eq!(removed.id, "r1");
        assert!(mgr.get_route("r1").is_none());
    }

    #[test]
    fn test_remove_route_missing() {
        let mut mgr = CrossChainManager::new();
        assert!(mgr.remove_route("nonexistent").is_err());
    }

    #[test]
    fn test_find_routes() {
        let mut mgr = CrossChainManager::new();
        mgr.add_route(sample_route("r1", 30)).unwrap();
        mgr.add_route(sample_route_chains("r2", ChainId::Solana, ChainId::Bitcoin)).unwrap();
        mgr.add_route(sample_route("r3", 50)).unwrap();

        let found = mgr.find_routes(&ChainId::EvaporChain, &ChainId::Ethereum);
        assert_eq!(found.len(), 2);

        let found2 = mgr.find_routes(&ChainId::Solana, &ChainId::Bitcoin);
        assert_eq!(found2.len(), 1);

        let found3 = mgr.find_routes(&ChainId::Bitcoin, &ChainId::Polygon);
        assert!(found3.is_empty());
    }

    #[test]
    fn test_best_route() {
        let mut mgr = CrossChainManager::new();
        mgr.add_route(sample_route("r_expensive", 100)).unwrap();
        mgr.add_route(sample_route("r_cheap", 10)).unwrap();
        mgr.add_route(sample_route("r_mid", 50)).unwrap();

        let best = mgr.best_route(&ChainId::EvaporChain, &ChainId::Ethereum).unwrap();
        assert_eq!(best.id, "r_cheap");
    }

    #[test]
    fn test_best_route_no_match() {
        let mgr = CrossChainManager::new();
        assert!(mgr.best_route(&ChainId::Bitcoin, &ChainId::Solana).is_none());
    }

    #[test]
    fn test_initiate_swap() {
        let mut mgr = CrossChainManager::new();
        mgr.add_route(sample_route("r1", 100)).unwrap(); // 1% fee, 0.5 rate

        let swap_id = mgr.initiate_swap("r1", 10_000).unwrap();
        let swap = mgr.get_swap(&swap_id).unwrap();

        assert_eq!(swap.source_amount, 10_000);
        // gross = 10_000 * 0.5 = 5_000; fee = 5_000 * 100 / 10_000 = 50; expected = 4_950
        assert_eq!(swap.expected_output, 4_950);
        assert_eq!(swap.status, SwapStatus::Pending);
        assert!(!swap.hashlock.is_empty());
        assert!(swap.actual_output.is_none());
    }

    #[test]
    fn test_initiate_swap_missing_route() {
        let mut mgr = CrossChainManager::new();
        assert!(mgr.initiate_swap("nonexistent", 1000).is_err());
    }

    #[test]
    fn test_lock_swap() {
        let mut mgr = CrossChainManager::new();
        mgr.add_route(sample_route("r1", 50)).unwrap();
        let swap_id = mgr.initiate_swap("r1", 5_000).unwrap();

        assert!(mgr.lock_swap(&swap_id).is_ok());
        assert_eq!(mgr.get_swap(&swap_id).unwrap().status, SwapStatus::Locked);
    }

    #[test]
    fn test_complete_swap() {
        let mut mgr = CrossChainManager::new();
        mgr.add_route(sample_route("r1", 50)).unwrap();
        let swap_id = mgr.initiate_swap("r1", 5_000).unwrap();
        mgr.lock_swap(&swap_id).unwrap();

        assert!(mgr.complete_swap(&swap_id, 2_400, "0xdest123").is_ok());
        let swap = mgr.get_swap(&swap_id).unwrap();
        assert_eq!(swap.status, SwapStatus::Completed);
        assert_eq!(swap.actual_output, Some(2_400));
        assert_eq!(swap.dest_tx, Some("0xdest123".to_string()));
        assert!(swap.completed_at.is_some());
        assert!(swap.slippage_bps.is_some());
    }

    #[test]
    fn test_refund_swap() {
        let mut mgr = CrossChainManager::new();
        mgr.add_route(sample_route("r1", 50)).unwrap();
        let swap_id = mgr.initiate_swap("r1", 5_000).unwrap();

        assert!(mgr.refund_swap(&swap_id).is_ok());
        assert_eq!(mgr.get_swap(&swap_id).unwrap().status, SwapStatus::Refunded);
    }

    #[test]
    fn test_fail_swap() {
        let mut mgr = CrossChainManager::new();
        mgr.add_route(sample_route("r1", 50)).unwrap();
        let swap_id = mgr.initiate_swap("r1", 5_000).unwrap();
        mgr.lock_swap(&swap_id).unwrap();

        assert!(mgr.fail_swap(&swap_id).is_ok());
        assert_eq!(mgr.get_swap(&swap_id).unwrap().status, SwapStatus::Failed);
    }

    #[test]
    fn test_fail_swap_missing() {
        let mut mgr = CrossChainManager::new();
        assert!(mgr.fail_swap("no_such_swap").is_err());
    }

    #[test]
    fn test_invalid_transition_lock_completed() {
        let mut mgr = CrossChainManager::new();
        mgr.add_route(sample_route("r1", 50)).unwrap();
        let sid = mgr.initiate_swap("r1", 1_000).unwrap();
        mgr.lock_swap(&sid).unwrap();
        mgr.complete_swap(&sid, 490, "0xtx").unwrap();

        // Cannot lock a completed swap.
        assert!(mgr.lock_swap(&sid).is_err());
    }

    #[test]
    fn test_slippage_calculation() {
        assert_eq!(CrossChainManager::calculate_slippage(1_000, 990), 100); // 1%
        assert_eq!(CrossChainManager::calculate_slippage(1_000, 1_000), 0);
        assert_eq!(CrossChainManager::calculate_slippage(1_000, 950), 500); // 5%
        assert_eq!(CrossChainManager::calculate_slippage(0, 100), 0);
        // Actual > expected (positive slippage)
        assert_eq!(CrossChainManager::calculate_slippage(1_000, 1_050), 500);
    }

    #[test]
    fn test_active_swaps() {
        let mut mgr = CrossChainManager::new();
        mgr.add_route(sample_route("r1", 50)).unwrap();

        let s1 = mgr.initiate_swap("r1", 1_000).unwrap();
        let s2 = mgr.initiate_swap("r1", 2_000).unwrap();
        let s3 = mgr.initiate_swap("r1", 3_000).unwrap();
        mgr.lock_swap(&s2).unwrap();
        mgr.lock_swap(&s3).unwrap();
        mgr.complete_swap(&s3, 1_400, "0xtx").unwrap();

        let active = mgr.active_swaps();
        assert_eq!(active.len(), 2); // s1 (Pending), s2 (Locked)
        let ids: Vec<&str> = active.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&s1.as_str()));
        assert!(ids.contains(&s2.as_str()));
    }

    #[test]
    fn test_swap_history_ordering() {
        let mut mgr = CrossChainManager::new();
        mgr.add_route(sample_route("r1", 50)).unwrap();

        let s1 = mgr.initiate_swap("r1", 1_000).unwrap();
        let s2 = mgr.initiate_swap("r1", 2_000).unwrap();
        let s3 = mgr.initiate_swap("r1", 3_000).unwrap();

        let history = mgr.swap_history();
        assert_eq!(history.len(), 3);
        // Most recent first.
        assert_eq!(history[0].id, s3);
        assert_eq!(history[1].id, s2);
        assert_eq!(history[2].id, s1);
    }

    #[test]
    fn test_stats() {
        let mut mgr = CrossChainManager::new();
        mgr.add_route(sample_route("r1", 50)).unwrap();
        mgr.add_route(sample_route("r2", 100)).unwrap();

        let s1 = mgr.initiate_swap("r1", 1_000).unwrap();
        let s2 = mgr.initiate_swap("r1", 2_000).unwrap();
        let s3 = mgr.initiate_swap("r1", 3_000).unwrap();

        mgr.lock_swap(&s1).unwrap();
        mgr.complete_swap(&s1, 480, "0xt1").unwrap();

        mgr.lock_swap(&s2).unwrap();
        mgr.complete_swap(&s2, 960, "0xt2").unwrap();

        mgr.refund_swap(&s3).unwrap();

        let stats = mgr.stats();
        assert_eq!(stats.total_swaps, 3);
        assert_eq!(stats.completed, 2);
        assert_eq!(stats.pending, 0);
        assert_eq!(stats.refunded, 1);
        assert_eq!(stats.total_volume_in, 3_000); // 1000 + 2000 completed
        assert_eq!(stats.total_volume_out, 1_440); // 480 + 960
        assert_eq!(stats.total_routes, 2);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path("roundtrip");
        let mut mgr = CrossChainManager::new();
        mgr.add_route(sample_route("r1", 30)).unwrap();
        let swap_id = mgr.initiate_swap("r1", 5_000).unwrap();
        mgr.lock_swap(&swap_id).unwrap();

        mgr.save(&path).unwrap();
        let loaded = CrossChainManager::load(&path).unwrap();

        assert_eq!(loaded.routes.len(), 1);
        assert_eq!(loaded.swaps.len(), 1);
        assert!(loaded.get_route("r1").is_some());
        assert_eq!(loaded.get_swap(&swap_id).unwrap().status, SwapStatus::Locked);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = test_path("no_such_file");
        let _ = std::fs::remove_file(&path);
        let mgr = CrossChainManager::load_or_default(&path);
        assert!(mgr.routes.is_empty());
        assert!(mgr.swaps.is_empty());
    }
}
