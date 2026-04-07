//! Self-diagnostic checks for the wallet.
//!
//! `wallet doctor` runs a series of health checks and reports
//! issues with actionable fix instructions.

use serde::Serialize;
use std::path::Path;

use crate::config::WalletConfig;
use crate::keystore::KeyStore;
use crate::rpc::RpcClient;

// ──────────────────────────── Check Result ────────────────────────────────

/// Outcome of a single diagnostic check.
#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub name: String,
    pub status: CheckStatus,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

impl CheckResult {
    fn pass(name: &str, msg: &str) -> Self {
        Self {
            name: name.to_string(),
            status: CheckStatus::Pass,
            message: msg.to_string(),
            fix: None,
        }
    }

    fn warn(name: &str, msg: &str, fix: &str) -> Self {
        Self {
            name: name.to_string(),
            status: CheckStatus::Warn,
            message: msg.to_string(),
            fix: Some(fix.to_string()),
        }
    }

    fn fail(name: &str, msg: &str, fix: &str) -> Self {
        Self {
            name: name.to_string(),
            status: CheckStatus::Fail,
            message: msg.to_string(),
            fix: Some(fix.to_string()),
        }
    }
}

// ──────────────────────────── Doctor ──────────────────────────────────────

/// Run all diagnostic checks and return results.
pub async fn run_checks(node_url: &str, keystore_path: &str) -> Vec<CheckResult> {
    let mut results = Vec::new();

    // 1. Config file
    results.push(check_config());

    // 2. Keystore file
    results.push(check_keystore(keystore_path));

    // 3. Node connection
    results.push(check_node(node_url).await);

    // 4. Data directories
    results.push(check_data_dir());

    // 5. Keystore has accounts
    results.push(check_accounts(keystore_path));

    // 6. Active account set
    results.push(check_active_account());

    results
}

fn check_config() -> CheckResult {
    let path = WalletConfig::default_path();
    if path.exists() {
        match WalletConfig::load_or_default(&path) {
            Ok(_) => CheckResult::pass("Config", &format!("Valid config at {:?}", path)),
            Err(e) => CheckResult::fail(
                "Config",
                &format!("Config file corrupt: {}", e),
                "wallet config reset",
            ),
        }
    } else {
        CheckResult::warn(
            "Config",
            "No config file — using defaults",
            "wallet config show (creates default config)",
        )
    }
}

fn check_keystore(path: &str) -> CheckResult {
    let p = Path::new(path);
    if p.exists() {
        match KeyStore::load(path) {
            Ok(ks) => CheckResult::pass(
                "Keystore",
                &format!("Valid keystore with {} keys", ks.len()),
            ),
            Err(e) => CheckResult::fail(
                "Keystore",
                &format!("Keystore corrupt: {}", e),
                "Restore from backup: wallet backup import <file>",
            ),
        }
    } else {
        CheckResult::warn(
            "Keystore",
            "No keystore file — will be created on first use",
            "wallet account create <name>",
        )
    }
}

async fn check_node(url: &str) -> CheckResult {
    match RpcClient::new(url) {
        Err(e) => CheckResult::fail(
            "Node",
            &format!("Invalid node URL: {}", e),
            "wallet config set node_url http://localhost:3000",
        ),
        Ok(rpc) => {
            let start = std::time::Instant::now();
            match rpc.get_status().await {
                Ok(status) => {
                    let latency = start.elapsed().as_millis();
                    CheckResult::pass(
                        "Node",
                        &format!(
                            "Connected to {} (block #{}, epoch {}, {}ms latency)",
                            url, status.block_height, status.epoch, latency
                        ),
                    )
                }
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("connection refused") {
                        CheckResult::fail(
                            "Node",
                            &format!("Cannot connect to {}", url),
                            "Start the node, or: wallet config set node_url <url>",
                        )
                    } else {
                        CheckResult::fail(
                            "Node",
                            &format!("Node error: {}", msg),
                            "Check node logs or try: wallet config set node_url <url>",
                        )
                    }
                }
            }
        }
    }
}

fn check_data_dir() -> CheckResult {
    let home = std::env::var("HOME").unwrap_or_default();
    let data_dir = format!("{}/.evaporchain", home);
    let p = Path::new(&data_dir);

    if p.exists() && p.is_dir() {
        // Check writable
        let test_path = p.join(".doctor_test");
        match std::fs::write(&test_path, "test") {
            Ok(_) => {
                let _ = std::fs::remove_file(&test_path);
                CheckResult::pass("Data Dir", &format!("{} exists and writable", data_dir))
            }
            Err(_) => CheckResult::fail(
                "Data Dir",
                &format!("{} exists but not writable", data_dir),
                &format!("chmod 755 {}", data_dir),
            ),
        }
    } else {
        CheckResult::warn(
            "Data Dir",
            &format!("{} doesn't exist yet", data_dir),
            "Will be created automatically on first use",
        )
    }
}

fn check_accounts(keystore_path: &str) -> CheckResult {
    match KeyStore::load(keystore_path) {
        Ok(ks) => {
            if ks.len() == 0 {
                CheckResult::warn(
                    "Accounts",
                    "No accounts in keystore",
                    "wallet account create <name>",
                )
            } else {
                CheckResult::pass("Accounts", &format!("{} account(s) available", ks.len()))
            }
        }
        Err(_) => CheckResult::warn(
            "Accounts",
            "Cannot read keystore",
            "wallet account create <name>",
        ),
    }
}

fn check_active_account() -> CheckResult {
    let path = WalletConfig::default_path();
    match WalletConfig::load_or_default(&path) {
        Ok(config) => {
            if config.active_account.is_some() {
                CheckResult::pass(
                    "Active Account",
                    &format!(
                        "Active: {}",
                        config.active_account.as_deref().unwrap_or("?")
                    ),
                )
            } else {
                CheckResult::warn(
                    "Active Account",
                    "No active account set",
                    "wallet account switch <name>",
                )
            }
        }
        Err(_) => CheckResult::warn(
            "Active Account",
            "Cannot read config",
            "wallet config reset",
        ),
    }
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_result_pass() {
        let r = CheckResult::pass("Test", "all good");
        assert_eq!(r.status, CheckStatus::Pass);
        assert!(r.fix.is_none());
    }

    #[test]
    fn test_check_result_warn() {
        let r = CheckResult::warn("Test", "not ideal", "do this");
        assert_eq!(r.status, CheckStatus::Warn);
        assert_eq!(r.fix.as_deref(), Some("do this"));
    }

    #[test]
    fn test_check_result_fail() {
        let r = CheckResult::fail("Test", "broken", "fix it");
        assert_eq!(r.status, CheckStatus::Fail);
    }

    #[test]
    fn test_check_result_serializable() {
        let r = CheckResult::pass("Node", "connected");
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"status\":\"pass\""));
        assert!(json.contains("\"name\":\"Node\""));
    }

    #[test]
    fn test_check_config_runs() {
        // Just verify it doesn't panic
        let _ = check_config();
    }

    #[test]
    fn test_check_data_dir_runs() {
        let _ = check_data_dir();
    }

    #[test]
    fn test_check_keystore_missing() {
        let r = check_keystore("/tmp/nonexistent_keystore_doctor_test.json");
        assert_eq!(r.status, CheckStatus::Warn);
    }

    #[test]
    fn test_check_accounts_missing_keystore() {
        let r = check_accounts("/tmp/nonexistent_keystore_doctor_test.json");
        assert_eq!(r.status, CheckStatus::Warn);
    }
}
