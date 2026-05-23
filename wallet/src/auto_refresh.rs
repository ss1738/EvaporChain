//! Auto-refresh daemon — monitors energy levels and automatically
//! submits refresh transactions when objects drop below a threshold.
//!
//! This is EvaporChain's killer UX feature: users set it and forget it.
//! Their objects stay alive without manual intervention.
//!
//! # Architecture
//!
//! ```text
//! AutoRefresher
//! ├── poll loop (every N seconds)
//! │   ├── fetch portfolio
//! │   ├── check each object against threshold
//! │   └── submit refresh tx for objects below threshold
//! └── config (threshold %, poll interval, max energy per refresh)
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::assets::{AssetManager, Portfolio};
use crate::pipeline::TxPipeline;
use crate::rpc::RpcClient;
use crate::signer::WalletSigner;

// ──────────────────────────── Config ──────────────────────────────────

/// Configuration for the auto-refresh daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoRefreshConfig {
    /// Energy percentage threshold — refresh when below this (0-100).
    pub threshold_pct: f64,
    /// Poll interval in seconds.
    pub poll_interval_secs: u64,
    /// Maximum energy to deposit per refresh.
    pub max_energy_per_refresh: u64,
    /// Minimum energy to deposit (skip if cost would be less).
    pub min_energy_per_refresh: u64,
    /// Object IDs to exclude from auto-refresh.
    #[serde(default)]
    pub exclude_ids: HashSet<String>,
    /// If true, also auto-refresh NFTs.
    pub include_nfts: bool,
}

impl Default for AutoRefreshConfig {
    fn default() -> Self {
        Self {
            threshold_pct: 25.0,
            poll_interval_secs: 60,
            max_energy_per_refresh: 10_000,
            min_energy_per_refresh: 100,
            exclude_ids: HashSet::new(),
            include_nfts: true,
        }
    }
}

// ──────────────────────────── Refresh Action ──────────────────────────

/// A refresh action that was taken or needs to be taken.
#[derive(Debug, Clone)]
pub struct RefreshAction {
    /// Object/NFT identifier.
    pub asset_id: String,
    /// Asset name.
    pub asset_name: String,
    /// Current energy percentage.
    pub current_pct: f64,
    /// Energy to deposit.
    pub energy_to_deposit: u64,
    /// Whether the refresh was submitted.
    pub submitted: bool,
    /// Tx hash if submitted.
    pub tx_hash: Option<String>,
    /// Error message if failed.
    pub error: Option<String>,
}

// ──────────────────────────── AutoRefresher ───────────────────────────

/// Scans a portfolio and identifies objects needing refresh.
pub struct AutoRefresher {
    config: AutoRefreshConfig,
}

impl AutoRefresher {
    pub fn new(config: AutoRefreshConfig) -> Self {
        Self { config }
    }

    pub fn with_default_config() -> Self {
        Self::new(AutoRefreshConfig::default())
    }

    pub fn config(&self) -> &AutoRefreshConfig {
        &self.config
    }

    /// Scan portfolio and return list of objects that need refreshing.
    pub fn scan(&self, portfolio: &Portfolio) -> Vec<RefreshAction> {
        let mut actions = Vec::new();

        for obj in &portfolio.objects {
            if self.config.exclude_ids.contains(&obj.id) {
                continue;
            }
            if obj.state == "Ghost" {
                continue;
            }

            let pct = if obj.max_energy > 0 {
                (obj.current_energy as f64 / obj.max_energy as f64) * 100.0
            } else {
                100.0
            };

            if pct < self.config.threshold_pct {
                let deficit = obj.max_energy.saturating_sub(obj.current_energy);
                let energy = deficit
                    .min(self.config.max_energy_per_refresh)
                    .max(self.config.min_energy_per_refresh);

                actions.push(RefreshAction {
                    asset_id: obj.id.clone(),
                    asset_name: obj.name.clone(),
                    current_pct: pct,
                    energy_to_deposit: energy,
                    submitted: false,
                    tx_hash: None,
                    error: None,
                });
            }
        }

        if self.config.include_nfts {
            for nft in &portfolio.nfts {
                let pct = if nft.max_energy > 0 {
                    (nft.current_energy as f64 / nft.max_energy as f64) * 100.0
                } else {
                    100.0
                };

                if pct < self.config.threshold_pct {
                    let deficit = nft.max_energy.saturating_sub(nft.current_energy);
                    let energy = deficit
                        .min(self.config.max_energy_per_refresh)
                        .max(self.config.min_energy_per_refresh);

                    actions.push(RefreshAction {
                        asset_id: format!("nft:{}", nft.id),
                        asset_name: nft.name.clone(),
                        current_pct: pct,
                        energy_to_deposit: energy,
                        submitted: false,
                        tx_hash: None,
                        error: None,
                    });
                }
            }
        }

        // Sort by most critical first
        actions.sort_by(|a, b| a.current_pct.partial_cmp(&b.current_pct).unwrap());
        actions
    }

    /// Execute a single refresh cycle: scan + submit refresh txs.
    /// Returns the actions taken.
    pub async fn execute_cycle(
        &self,
        rpc: &RpcClient,
        signer: &WalletSigner,
        address: &str,
    ) -> Result<Vec<RefreshAction>, Box<dyn std::error::Error + Send + Sync>> {
        let asset_mgr = AssetManager::new(RpcClient::new(rpc.base_url())?);
        let portfolio = asset_mgr.fetch_portfolio(address).await?;

        let mut actions = self.scan(&portfolio);

        let rpc2 = RpcClient::new(rpc.base_url())?;
        let mut pipeline = TxPipeline::new(rpc2);

        for action in &mut actions {
            if action.asset_id.starts_with("nft:") {
                // NFT refresh via API
                if let Ok(nft_id) = action.asset_id[4..].parse::<u64>() {
                    match pipeline.refresh_nft(nft_id, action.energy_to_deposit).await {
                        Ok(resp) => {
                            action.submitted = true;
                            action.tx_hash = resp.tx_hash;
                        }
                        Err(e) => {
                            action.error = Some(e.to_string());
                        }
                    }
                }
            } else {
                // Object refresh via signed tx
                let obj_id = crate::address::parse_address(&action.asset_id).unwrap_or([0u8; 32]);
                match pipeline
                    .refresh_object(signer, &obj_id, action.energy_to_deposit)
                    .await
                {
                    Ok(resp) => {
                        action.submitted = true;
                        action.tx_hash = resp.tx_hash;
                    }
                    Err(e) => {
                        action.error = Some(e.to_string());
                    }
                }
            }
        }

        Ok(actions)
    }

    /// Run the auto-refresh loop. Runs until cancelled.
    /// Calls `on_cycle` after each scan+refresh cycle with the actions taken.
    pub async fn run_loop<F>(
        &self,
        rpc: &RpcClient,
        signer: &WalletSigner,
        address: &str,
        mut on_cycle: F,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    where
        F: FnMut(&[RefreshAction]),
    {
        loop {
            let actions = self.execute_cycle(rpc, signer, address).await?;
            if !actions.is_empty() {
                on_cycle(&actions);
            }
            tokio::time::sleep(std::time::Duration::from_secs(
                self.config.poll_interval_secs,
            ))
            .await;
        }
    }

    /// Run the auto-refresh loop with graceful shutdown on Ctrl+C.
    /// Returns a summary: (total_cycles, total_refreshes).
    pub async fn run_loop_graceful<F>(
        &self,
        rpc: &RpcClient,
        signer: &WalletSigner,
        address: &str,
        mut on_cycle: F,
    ) -> Result<DaemonSummary, Box<dyn std::error::Error + Send + Sync>>
    where
        F: FnMut(&[RefreshAction]),
    {
        let mut summary = DaemonSummary::default();

        loop {
            tokio::select! {
                result = self.execute_cycle(rpc, signer, address) => {
                    let actions = result?;
                    summary.cycles += 1;
                    let refreshed = actions.iter().filter(|a| a.submitted).count();
                    let failed = actions.iter().filter(|a| a.error.is_some()).count();
                    summary.refreshes += refreshed as u64;
                    summary.failures += failed as u64;
                    if !actions.is_empty() {
                        on_cycle(&actions);
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    summary.shutdown_reason = "Ctrl+C".to_string();
                    return Ok(summary);
                }
            }

            // Interruptible sleep
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(
                    self.config.poll_interval_secs,
                )) => {}
                _ = tokio::signal::ctrl_c() => {
                    summary.shutdown_reason = "Ctrl+C".to_string();
                    return Ok(summary);
                }
            }
        }
    }
}

/// Summary of a daemon run.
#[derive(Debug, Clone, Default)]
pub struct DaemonSummary {
    /// Number of scan cycles completed.
    pub cycles: u64,
    /// Number of successful refreshes.
    pub refreshes: u64,
    /// Number of failed refresh attempts.
    pub failures: u64,
    /// Reason for shutdown.
    pub shutdown_reason: String,
}

// ──────────────────────────── Tests ──────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{OwnedNft, OwnedObject, Portfolio};

    fn make_portfolio() -> Portfolio {
        Portfolio {
            address: "0xtest".to_string(),
            balance: 10000,
            nonce: 0,
            objects: vec![
                OwnedObject {
                    id: "0xobj1".to_string(),
                    name: "healthy".to_string(),
                    energy: 1000,
                    max_energy: 1000,
                    current_energy: 900,
                    half_life: 100,
                    state: "Active".to_string(),
                    created_epoch: 0,
                    last_refreshed: 0,
                    decay_percentage: 10.0,
                    epochs_until_zero: Some(500),
                },
                OwnedObject {
                    id: "0xobj2".to_string(),
                    name: "critical".to_string(),
                    energy: 1000,
                    max_energy: 1000,
                    current_energy: 100,
                    half_life: 100,
                    state: "Active".to_string(),
                    created_epoch: 0,
                    last_refreshed: 0,
                    decay_percentage: 90.0,
                    epochs_until_zero: Some(50),
                },
                OwnedObject {
                    id: "0xobj3".to_string(),
                    name: "dead".to_string(),
                    energy: 0,
                    max_energy: 1000,
                    current_energy: 0,
                    half_life: 100,
                    state: "Ghost".to_string(),
                    created_epoch: 0,
                    last_refreshed: 0,
                    decay_percentage: 100.0,
                    epochs_until_zero: None,
                },
            ],
            nfts: vec![OwnedNft {
                id: 1,
                name: "low_nft".to_string(),
                collection: "test".to_string(),
                current_energy: 50,
                max_energy: 500,
                half_life: 50,
                state: "Active".to_string(),
                decay_percentage: 90.0,
                epochs_remaining: 10,
                grace_epoch: None,
                evaporated_epoch: None,
            }],
            tokens: vec![],
            total_energy_at_risk: 1050,
        }
    }

    #[test]
    fn test_scan_finds_critical_objects() {
        let refresher = AutoRefresher::with_default_config(); // 25% threshold
        let portfolio = make_portfolio();
        let actions = refresher.scan(&portfolio);

        // Should find obj2 (10%) and nft:1 (10%), not obj1 (90%) or obj3 (ghost)
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].asset_name, "critical");
        assert_eq!(actions[1].asset_name, "low_nft");
    }

    #[test]
    fn test_scan_excludes_ghosts() {
        let refresher = AutoRefresher::new(AutoRefreshConfig {
            threshold_pct: 100.0, // catch everything
            ..Default::default()
        });
        let portfolio = make_portfolio();
        let actions = refresher.scan(&portfolio);

        let ghost_actions: Vec<_> = actions.iter().filter(|a| a.asset_name == "dead").collect();
        assert!(ghost_actions.is_empty());
    }

    #[test]
    fn test_scan_respects_exclude_list() {
        let refresher = AutoRefresher::new(AutoRefreshConfig {
            threshold_pct: 100.0,
            exclude_ids: ["0xobj2".to_string()].into(),
            ..Default::default()
        });
        let portfolio = make_portfolio();
        let actions = refresher.scan(&portfolio);

        let excluded: Vec<_> = actions.iter().filter(|a| a.asset_id == "0xobj2").collect();
        assert!(excluded.is_empty());
    }

    #[test]
    fn test_scan_sorted_by_criticality() {
        let refresher = AutoRefresher::with_default_config();
        let portfolio = make_portfolio();
        let actions = refresher.scan(&portfolio);

        if actions.len() >= 2 {
            assert!(actions[0].current_pct <= actions[1].current_pct);
        }
    }

    #[test]
    fn test_energy_capped_at_max() {
        let refresher = AutoRefresher::new(AutoRefreshConfig {
            threshold_pct: 50.0,
            max_energy_per_refresh: 200,
            ..Default::default()
        });
        let portfolio = make_portfolio();
        let actions = refresher.scan(&portfolio);

        for action in &actions {
            assert!(action.energy_to_deposit <= 200);
        }
    }

    #[test]
    fn test_energy_minimum_enforced() {
        let refresher = AutoRefresher::new(AutoRefreshConfig {
            threshold_pct: 50.0,
            min_energy_per_refresh: 500,
            ..Default::default()
        });
        let portfolio = make_portfolio();
        let actions = refresher.scan(&portfolio);

        for action in &actions {
            assert!(action.energy_to_deposit >= 500);
        }
    }

    #[test]
    fn test_nft_exclusion() {
        let refresher = AutoRefresher::new(AutoRefreshConfig {
            threshold_pct: 50.0,
            include_nfts: false,
            ..Default::default()
        });
        let portfolio = make_portfolio();
        let actions = refresher.scan(&portfolio);

        let nft_actions: Vec<_> = actions
            .iter()
            .filter(|a| a.asset_id.starts_with("nft:"))
            .collect();
        assert!(nft_actions.is_empty());
    }

    #[test]
    fn test_default_config() {
        let config = AutoRefreshConfig::default();
        assert_eq!(config.threshold_pct, 25.0);
        assert_eq!(config.poll_interval_secs, 60);
        assert!(config.include_nfts);
    }

    // ─── Additional coverage tests (session 63) ───────────────────────────────

    #[test]
    fn test_config_getter() {
        let refresher = AutoRefresher::with_default_config();
        let cfg = refresher.config();
        assert_eq!(cfg.threshold_pct, 25.0);
        assert_eq!(cfg.poll_interval_secs, 60);
    }

    #[test]
    fn test_scan_zero_max_energy_object_treated_as_full() {
        let refresher = AutoRefresher::with_default_config();
        let portfolio = Portfolio {
            address: "0xtest".into(),
            balance: 0,
            nonce: 0,
            objects: vec![OwnedObject {
                id: "0xzero".into(),
                name: "zero_max".into(),
                energy: 0,
                max_energy: 0,
                current_energy: 0,
                half_life: 0,
                state: "Active".into(),
                created_epoch: 0,
                last_refreshed: 0,
                decay_percentage: 0.0,
                epochs_until_zero: None,
            }],
            nfts: vec![],
            tokens: vec![],
            total_energy_at_risk: 0,
        };
        let actions = refresher.scan(&portfolio);
        assert!(actions.is_empty());
    }

    #[test]
    fn test_scan_zero_max_energy_nft_treated_as_full() {
        let refresher = AutoRefresher::with_default_config();
        let portfolio = Portfolio {
            address: "0xtest".into(),
            balance: 0,
            nonce: 0,
            objects: vec![],
            nfts: vec![OwnedNft {
                id: 99,
                name: "zero_nft".into(),
                collection: "c".into(),
                current_energy: 0,
                max_energy: 0,
                half_life: 0,
                state: "Active".into(),
                decay_percentage: 0.0,
                epochs_remaining: 0,
                grace_epoch: None,
                evaporated_epoch: None,
            }],
            tokens: vec![],
            total_energy_at_risk: 0,
        };
        let actions = refresher.scan(&portfolio);
        assert!(actions.is_empty());
    }

    #[test]
    fn test_config_json_roundtrip() {
        let config = AutoRefreshConfig {
            threshold_pct: 30.0,
            poll_interval_secs: 120,
            max_energy_per_refresh: 5000,
            min_energy_per_refresh: 50,
            exclude_ids: ["0xabc".to_string()].into(),
            include_nfts: false,
        };
        let json = serde_json::to_string(&config).unwrap();
        let loaded: AutoRefreshConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.threshold_pct, 30.0);
        assert_eq!(loaded.poll_interval_secs, 120);
        assert!(!loaded.include_nfts);
    }
}
