//! Cross-chain bridge support — bridge registry and transfer tracking.
//!
//! Manages known bridges, tracks cross-chain transfers, and monitors
//! bridge health status. Supports multiple bridge types and chains.

use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("bridge not found: {0}")]
    BridgeNotFound(String),
    #[error("transfer not found: {0}")]
    TransferNotFound(String),
    #[error("unsupported chain: {0}")]
    UnsupportedChain(String),
    #[error("duplicate bridge: {0}")]
    DuplicateBridge(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ──────────────────────────── Types ──────────────────────────────────────

/// Known chain identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChainId {
    EvaporChain,
    Ethereum,
    Solana,
    Sui,
    Aptos,
    Polygon,
    Arbitrum,
    Custom(String),
}

impl ChainId {
    pub fn label(&self) -> &str {
        match self {
            Self::EvaporChain => "evaporchain",
            Self::Ethereum => "ethereum",
            Self::Solana => "solana",
            Self::Sui => "sui",
            Self::Aptos => "aptos",
            Self::Polygon => "polygon",
            Self::Arbitrum => "arbitrum",
            Self::Custom(s) => s.as_str(),
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "evaporchain" | "evap" => Self::EvaporChain,
            "ethereum" | "eth" => Self::Ethereum,
            "solana" | "sol" => Self::Solana,
            "sui" => Self::Sui,
            "aptos" | "apt" => Self::Aptos,
            "polygon" | "matic" => Self::Polygon,
            "arbitrum" | "arb" => Self::Arbitrum,
            other => Self::Custom(other.to_string()),
        }
    }
}

/// Bridge protocol type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BridgeType {
    /// Lock-and-mint bridge.
    LockMint,
    /// Burn-and-mint bridge.
    BurnMint,
    /// Liquidity pool bridge.
    LiquidityPool,
    /// Native bridge (built into chain).
    Native,
}

/// A registered bridge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bridge {
    /// Unique bridge ID.
    pub id: String,
    /// Bridge name.
    pub name: String,
    /// Bridge type.
    pub bridge_type: BridgeType,
    /// Source chain.
    pub source_chain: ChainId,
    /// Destination chain.
    pub dest_chain: ChainId,
    /// Bridge contract address on source chain.
    pub source_contract: String,
    /// Bridge contract address on dest chain.
    pub dest_contract: String,
    /// Supported token symbols.
    pub supported_tokens: Vec<String>,
    /// Estimated transfer time (minutes).
    pub estimated_time_min: u64,
    /// Bridge fee percentage.
    pub fee_pct: f64,
    /// Whether bridge is currently active.
    pub active: bool,
    /// When added.
    pub added_at: String,
}

/// Status of a cross-chain transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferStatus {
    Initiated,
    SourceConfirmed,
    Bridging,
    DestConfirmed,
    Completed,
    Failed,
}

/// A cross-chain transfer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeTransfer {
    /// Unique transfer ID.
    pub id: String,
    /// Bridge used.
    pub bridge_id: String,
    /// Source chain.
    pub source_chain: ChainId,
    /// Destination chain.
    pub dest_chain: ChainId,
    /// Token symbol.
    pub token: String,
    /// Amount transferred.
    pub amount: u64,
    /// Source tx hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_tx: Option<String>,
    /// Destination tx hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dest_tx: Option<String>,
    /// Sender address on source chain.
    pub sender: String,
    /// Recipient address on dest chain.
    pub recipient: String,
    /// Current status.
    pub status: TransferStatus,
    /// Fee paid.
    pub fee: u64,
    /// When initiated.
    pub created_at: String,
    /// When completed (if completed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

// ──────────────────────────── BridgeManager ──────────────────────────────

/// Manages bridge registry and transfer tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeManager {
    pub bridges: Vec<Bridge>,
    pub transfers: Vec<BridgeTransfer>,
}

impl BridgeManager {
    pub fn new() -> Self {
        Self {
            bridges: Vec::new(),
            transfers: Vec::new(),
        }
    }

    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, BridgeError> {
        let data = std::fs::read_to_string(path)?;
        let mgr: BridgeManager = serde_json::from_str(&data)?;
        Ok(mgr)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), BridgeError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    // ── Bridge Registry ─────────────────────────────────────────────────

    /// Register a bridge.
    pub fn register_bridge(&mut self, bridge: Bridge) -> Result<(), BridgeError> {
        if self.bridges.iter().any(|b| b.id == bridge.id) {
            return Err(BridgeError::DuplicateBridge(bridge.id));
        }
        self.bridges.push(bridge);
        Ok(())
    }

    /// Remove a bridge.
    pub fn remove_bridge(&mut self, id: &str) -> Result<(), BridgeError> {
        let before = self.bridges.len();
        self.bridges.retain(|b| b.id != id);
        if self.bridges.len() == before {
            return Err(BridgeError::BridgeNotFound(id.to_string()));
        }
        Ok(())
    }

    /// Get a bridge by ID.
    pub fn get_bridge(&self, id: &str) -> Option<&Bridge> {
        self.bridges.iter().find(|b| b.id == id)
    }

    /// List all bridges.
    pub fn list_bridges(&self) -> &[Bridge] {
        &self.bridges
    }

    /// Find bridges between two chains.
    pub fn find_bridges(&self, source: &ChainId, dest: &ChainId) -> Vec<&Bridge> {
        self.bridges
            .iter()
            .filter(|b| b.active && b.source_chain == *source && b.dest_chain == *dest)
            .collect()
    }

    /// Find bridges that support a specific token.
    pub fn find_bridges_for_token(&self, token: &str) -> Vec<&Bridge> {
        let token_lower = token.to_lowercase();
        self.bridges
            .iter()
            .filter(|b| {
                b.active
                    && b.supported_tokens
                        .iter()
                        .any(|t| t.to_lowercase() == token_lower)
            })
            .collect()
    }

    // ── Transfer Tracking ───────────────────────────────────────────────

    /// Initiate a cross-chain transfer.
    pub fn initiate_transfer(
        &mut self,
        bridge_id: &str,
        token: &str,
        amount: u64,
        sender: &str,
        recipient: &str,
    ) -> Result<&BridgeTransfer, BridgeError> {
        let bridge = self
            .get_bridge(bridge_id)
            .ok_or_else(|| BridgeError::BridgeNotFound(bridge_id.to_string()))?;

        let fee = (amount as f64 * bridge.fee_pct / 100.0) as u64;
        let now = chrono::Utc::now();
        let id = format!("xfer_{}", now.timestamp_millis());

        let transfer = BridgeTransfer {
            id,
            bridge_id: bridge_id.to_string(),
            source_chain: bridge.source_chain.clone(),
            dest_chain: bridge.dest_chain.clone(),
            token: token.to_string(),
            amount,
            source_tx: None,
            dest_tx: None,
            sender: sender.to_string(),
            recipient: recipient.to_string(),
            status: TransferStatus::Initiated,
            fee,
            created_at: now.to_rfc3339(),
            completed_at: None,
        };

        self.transfers.push(transfer);
        Ok(self.transfers.last().unwrap())
    }

    /// Update transfer status.
    pub fn update_transfer_status(
        &mut self,
        transfer_id: &str,
        status: TransferStatus,
        tx_hash: Option<&str>,
    ) -> Result<(), BridgeError> {
        let transfer = self
            .transfers
            .iter_mut()
            .find(|t| t.id == transfer_id)
            .ok_or_else(|| BridgeError::TransferNotFound(transfer_id.to_string()))?;

        transfer.status = status;
        if let Some(hash) = tx_hash {
            match status {
                TransferStatus::SourceConfirmed => transfer.source_tx = Some(hash.to_string()),
                TransferStatus::DestConfirmed | TransferStatus::Completed => {
                    transfer.dest_tx = Some(hash.to_string());
                }
                _ => {}
            }
        }
        if status == TransferStatus::Completed {
            transfer.completed_at = Some(chrono::Utc::now().to_rfc3339());
        }
        Ok(())
    }

    /// Get a transfer by ID.
    pub fn get_transfer(&self, id: &str) -> Option<&BridgeTransfer> {
        self.transfers.iter().find(|t| t.id == id)
    }

    /// List all transfers.
    pub fn list_transfers(&self) -> &[BridgeTransfer] {
        &self.transfers
    }

    /// List pending transfers.
    pub fn pending_transfers(&self) -> Vec<&BridgeTransfer> {
        self.transfers
            .iter()
            .filter(|t| t.status != TransferStatus::Completed && t.status != TransferStatus::Failed)
            .collect()
    }

    /// Bridge count.
    pub fn bridge_count(&self) -> usize {
        self.bridges.len()
    }

    /// Transfer count.
    pub fn transfer_count(&self) -> usize {
        self.transfers.len()
    }
}

impl Default for BridgeManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Default path.
pub fn default_bridge_path() -> std::path::PathBuf {
    crate::config::default_data_dir().join("bridges.json")
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bridge() -> Bridge {
        Bridge {
            id: "evap-eth-v1".to_string(),
            name: "EvapBridge ETH".to_string(),
            bridge_type: BridgeType::LockMint,
            source_chain: ChainId::EvaporChain,
            dest_chain: ChainId::Ethereum,
            source_contract: "0xbridge_evap".to_string(),
            dest_contract: "0xbridge_eth".to_string(),
            supported_tokens: vec!["EVAP".into(), "WETH".into()],
            estimated_time_min: 15,
            fee_pct: 0.3,
            active: true,
            added_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    fn make_manager() -> BridgeManager {
        let mut mgr = BridgeManager::new();
        mgr.register_bridge(make_bridge()).unwrap();
        mgr
    }

    #[test]
    fn test_register_bridge() {
        let mgr = make_manager();
        assert_eq!(mgr.bridge_count(), 1);
        assert!(mgr.get_bridge("evap-eth-v1").is_some());
    }

    #[test]
    fn test_register_duplicate() {
        let mut mgr = make_manager();
        let err = mgr.register_bridge(make_bridge()).unwrap_err();
        assert!(matches!(err, BridgeError::DuplicateBridge(_)));
    }

    #[test]
    fn test_remove_bridge() {
        let mut mgr = make_manager();
        mgr.remove_bridge("evap-eth-v1").unwrap();
        assert_eq!(mgr.bridge_count(), 0);
    }

    #[test]
    fn test_remove_not_found() {
        let mut mgr = BridgeManager::new();
        let err = mgr.remove_bridge("nope").unwrap_err();
        assert!(matches!(err, BridgeError::BridgeNotFound(_)));
    }

    #[test]
    fn test_find_bridges() {
        let mgr = make_manager();
        let found = mgr.find_bridges(&ChainId::EvaporChain, &ChainId::Ethereum);
        assert_eq!(found.len(), 1);
        let found = mgr.find_bridges(&ChainId::Ethereum, &ChainId::Solana);
        assert_eq!(found.len(), 0);
    }

    #[test]
    fn test_find_bridges_for_token() {
        let mgr = make_manager();
        assert_eq!(mgr.find_bridges_for_token("EVAP").len(), 1);
        assert_eq!(mgr.find_bridges_for_token("WETH").len(), 1);
        assert_eq!(mgr.find_bridges_for_token("BTC").len(), 0);
    }

    #[test]
    fn test_initiate_transfer() {
        let mut mgr = make_manager();
        let xfer = mgr
            .initiate_transfer("evap-eth-v1", "EVAP", 10000, "0xsender", "0xrecipient")
            .unwrap();
        assert_eq!(xfer.status, TransferStatus::Initiated);
        assert_eq!(xfer.amount, 10000);
        assert_eq!(xfer.fee, 30); // 0.3% of 10000
    }

    #[test]
    fn test_initiate_transfer_bridge_not_found() {
        let mut mgr = BridgeManager::new();
        let err = mgr
            .initiate_transfer("nope", "EVAP", 100, "0x1", "0x2")
            .unwrap_err();
        assert!(matches!(err, BridgeError::BridgeNotFound(_)));
    }

    #[test]
    fn test_update_transfer_status() {
        let mut mgr = make_manager();
        let xfer = mgr
            .initiate_transfer("evap-eth-v1", "EVAP", 1000, "0x1", "0x2")
            .unwrap();
        let xfer_id = xfer.id.clone();

        mgr.update_transfer_status(
            &xfer_id,
            TransferStatus::SourceConfirmed,
            Some("0xsrc_hash"),
        )
        .unwrap();
        let xfer = mgr.get_transfer(&xfer_id).unwrap();
        assert_eq!(xfer.status, TransferStatus::SourceConfirmed);
        assert_eq!(xfer.source_tx.as_deref(), Some("0xsrc_hash"));

        mgr.update_transfer_status(&xfer_id, TransferStatus::Completed, Some("0xdst_hash"))
            .unwrap();
        let xfer = mgr.get_transfer(&xfer_id).unwrap();
        assert_eq!(xfer.status, TransferStatus::Completed);
        assert!(xfer.completed_at.is_some());
    }

    #[test]
    fn test_pending_transfers() {
        let mut mgr = make_manager();
        mgr.initiate_transfer("evap-eth-v1", "EVAP", 100, "0x1", "0x2")
            .unwrap();
        mgr.initiate_transfer("evap-eth-v1", "EVAP", 200, "0x1", "0x2")
            .unwrap();

        assert_eq!(mgr.pending_transfers().len(), 2);

        let id = mgr.transfers[0].id.clone();
        mgr.update_transfer_status(&id, TransferStatus::Completed, None)
            .unwrap();
        assert_eq!(mgr.pending_transfers().len(), 1);
    }

    #[test]
    fn test_chain_id_from_str() {
        assert_eq!(ChainId::from_str("ethereum"), ChainId::Ethereum);
        assert_eq!(ChainId::from_str("eth"), ChainId::Ethereum);
        assert_eq!(ChainId::from_str("solana"), ChainId::Solana);
        assert_eq!(ChainId::from_str("evap"), ChainId::EvaporChain);
        assert!(matches!(ChainId::from_str("unknown"), ChainId::Custom(_)));
    }

    #[test]
    fn test_chain_id_label() {
        assert_eq!(ChainId::EvaporChain.label(), "evaporchain");
        assert_eq!(ChainId::Ethereum.label(), "ethereum");
    }

    #[test]
    fn test_bridge_serializable() {
        let bridge = make_bridge();
        let json = serde_json::to_string(&bridge).unwrap();
        assert!(json.contains("\"bridge_type\":\"lockmint\""));
    }

    #[test]
    fn test_transfer_serializable() {
        let xfer = BridgeTransfer {
            id: "xfer_1".into(),
            bridge_id: "evap-eth-v1".into(),
            source_chain: ChainId::EvaporChain,
            dest_chain: ChainId::Ethereum,
            token: "EVAP".into(),
            amount: 1000,
            source_tx: None,
            dest_tx: None,
            sender: "0x1".into(),
            recipient: "0x2".into(),
            status: TransferStatus::Initiated,
            fee: 3,
            created_at: "2026-01-01".into(),
            completed_at: None,
        };
        let json = serde_json::to_string(&xfer).unwrap();
        assert!(json.contains("\"status\":\"initiated\""));
    }

    #[test]
    fn test_json_roundtrip() {
        let mut mgr = make_manager();
        mgr.initiate_transfer("evap-eth-v1", "EVAP", 100, "0x1", "0x2")
            .unwrap();

        let json = serde_json::to_string_pretty(&mgr).unwrap();
        let loaded: BridgeManager = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.bridge_count(), 1);
        assert_eq!(loaded.transfer_count(), 1);
    }

    #[test]
    fn test_file_save_and_load() {
        let dir = std::env::temp_dir().join("evaporchain_bridge_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bridges.json");

        let mgr = make_manager();
        mgr.save(&path).unwrap();

        let loaded = BridgeManager::load(&path).unwrap();
        assert_eq!(loaded.bridge_count(), 1);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }
}
