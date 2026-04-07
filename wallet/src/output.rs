//! Structured output for CLI commands.
//!
//! Supports two modes:
//! - **Human**: colored, formatted text (default)
//! - **JSON**: machine-readable JSON to stdout
//!
//! Commands use `Output` to emit results. The mode is set once at startup
//! and checked by each output call.

use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};

static JSON_MODE: AtomicBool = AtomicBool::new(false);

/// Enable JSON output mode (call once at startup).
pub fn set_json_mode(enabled: bool) {
    JSON_MODE.store(enabled, Ordering::Relaxed);
}

/// Check if JSON mode is active.
pub fn is_json_mode() -> bool {
    JSON_MODE.load(Ordering::Relaxed)
}

/// Print a value as JSON if in JSON mode, otherwise call the human formatter.
/// Returns true if JSON was printed (caller should skip human output).
pub fn json_or<T: Serialize, F: FnOnce()>(value: &T, human: F) {
    if is_json_mode() {
        print_json(value);
    } else {
        human();
    }
}

/// Print a serializable value as pretty JSON.
pub fn print_json<T: Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(json) => println!("{}", json),
        Err(e) => eprintln!("{{\"error\": \"JSON serialization failed: {}\"}}", e),
    }
}

/// Print a JSON error object.
pub fn print_json_error(error: &str) {
    println!("{{\"error\": {}}}", serde_json::to_string(error).unwrap_or_else(|_| format!("\"{}\"", error)));
}

// ──────────────────────────── Common Output Types ────────────────────────

/// Generic success response for JSON mode.
#[derive(Debug, Serialize)]
pub struct SuccessResponse {
    pub status: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
}

impl SuccessResponse {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            status: "ok",
            message: message.into(),
            tx_hash: None,
            address: None,
        }
    }

    pub fn with_tx(message: impl Into<String>, tx_hash: &str) -> Self {
        Self {
            status: "ok",
            message: message.into(),
            tx_hash: Some(tx_hash.to_string()),
            address: None,
        }
    }

    pub fn with_address(message: impl Into<String>, address: &str) -> Self {
        Self {
            status: "ok",
            message: message.into(),
            tx_hash: None,
            address: Some(address.to_string()),
        }
    }
}

/// Account info for JSON output.
#[derive(Debug, Serialize)]
pub struct AccountInfo {
    pub name: String,
    pub address: String,
    pub balance: Option<u64>,
    pub is_active: bool,
}

/// Energy alert for JSON output.
#[derive(Debug, Serialize)]
pub struct EnergyAlertJson {
    pub asset_id: String,
    pub asset_name: String,
    pub severity: String,
    pub energy_pct: f64,
    pub epochs_until_zero: Option<u64>,
}

/// Gas estimate for JSON output.
#[derive(Debug, Serialize)]
pub struct GasEstimateJson {
    pub gas_used: u64,
    pub base_fee: u64,
    pub gas_fee: u64,
    pub extra_fee: u64,
    pub total_fee: u64,
}

/// History entry for JSON output.
#[derive(Debug, Serialize)]
pub struct HistoryEntryJson {
    pub tx_hash: Option<String>,
    pub tx_type: String,
    pub from: String,
    pub to: Option<String>,
    pub amount: Option<u64>,
    pub outcome: String,
    pub submitted_at: String,
}

/// Portfolio summary for JSON output.
#[derive(Debug, Serialize)]
pub struct PortfolioJson {
    pub address: String,
    pub balance: u64,
    pub objects: usize,
    pub nfts: usize,
    pub tokens: usize,
    pub total_energy_at_risk: u64,
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_mode_toggle() {
        set_json_mode(false);
        assert!(!is_json_mode());
        set_json_mode(true);
        assert!(is_json_mode());
        set_json_mode(false); // reset for other tests
    }

    #[test]
    fn test_success_response_ok() {
        let resp = SuccessResponse::ok("done");
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"status\":\"ok\""));
        assert!(json.contains("\"message\":\"done\""));
        assert!(!json.contains("tx_hash"));
    }

    #[test]
    fn test_success_response_with_tx() {
        let resp = SuccessResponse::with_tx("sent", "0xhash123");
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"tx_hash\":\"0xhash123\""));
    }

    #[test]
    fn test_success_response_with_address() {
        let resp = SuccessResponse::with_address("created", "0xaddr");
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"address\":\"0xaddr\""));
    }

    #[test]
    fn test_account_info_serialization() {
        let info = AccountInfo {
            name: "alice".to_string(),
            address: "0xabc".to_string(),
            balance: Some(5000),
            is_active: true,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"name\":\"alice\""));
        assert!(json.contains("\"is_active\":true"));
    }

    #[test]
    fn test_gas_estimate_serialization() {
        let est = GasEstimateJson {
            gas_used: 21000,
            base_fee: 10,
            gas_fee: 210000,
            extra_fee: 0,
            total_fee: 210000,
        };
        let json = serde_json::to_string(&est).unwrap();
        assert!(json.contains("\"gas_used\":21000"));
    }

    #[test]
    fn test_energy_alert_serialization() {
        let alert = EnergyAlertJson {
            asset_id: "0xobj1".to_string(),
            asset_name: "my_object".to_string(),
            severity: "critical".to_string(),
            energy_pct: 5.2,
            epochs_until_zero: Some(10),
        };
        let json = serde_json::to_string(&alert).unwrap();
        assert!(json.contains("\"severity\":\"critical\""));
    }

    #[test]
    fn test_portfolio_serialization() {
        let p = PortfolioJson {
            address: "0xabc".to_string(),
            balance: 10000,
            objects: 3,
            nfts: 1,
            tokens: 2,
            total_energy_at_risk: 5000,
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"total_energy_at_risk\":5000"));
    }

    #[test]
    fn test_history_entry_serialization() {
        let entry = HistoryEntryJson {
            tx_hash: Some("0xhash".to_string()),
            tx_type: "Transfer".to_string(),
            from: "0xaaa".to_string(),
            to: Some("0xbbb".to_string()),
            amount: Some(1000),
            outcome: "accepted".to_string(),
            submitted_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"tx_type\":\"Transfer\""));
    }
}
