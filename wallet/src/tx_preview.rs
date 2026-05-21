//! Transaction preview and simulation system.
//!
//! Provides a pre-signing preview of transaction effects so users can
//! review state changes, gas costs, and risk warnings before committing.
//!
//! # Features
//!
//! - Automatic risk detection (high-value, unknown recipient, large gas, etc.)
//! - Graduated severity levels (Safe → Caution → Warning → Danger)
//! - State change tracking with before/after snapshots
//! - Persistent preview history for audit trails
//! - Known-address registry to suppress false positives

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum TxPreviewError {
    #[error("invalid transaction: {0}")]
    InvalidTransaction(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ──────────────────────────── Enums ────────────────────────────────────

/// Risk warning categories detected during transaction preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskWarning {
    HighValue,
    UnknownRecipient,
    ContractInteraction,
    LargeGas,
    TokenApproval,
    FirstInteraction,
    SuspiciousPattern,
    LowBalance,
}

/// Overall preview safety status, ordered by severity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PreviewStatus {
    #[default]
    Safe,
    Caution,
    Warning,
    Danger,
}

// ──────────────────────────── StateChange ──────────────────────────────

/// A single state change produced by a transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateChange {
    pub key: String,
    pub before: String,
    pub after: String,
    pub category: String,
}

impl StateChange {
    pub fn new(key: &str, before: &str, after: &str, category: &str) -> Self {
        Self {
            key: key.to_string(),
            before: before.to_string(),
            after: after.to_string(),
            category: category.to_string(),
        }
    }

    /// Returns `true` if this change is a balance change.
    pub fn is_balance_change(&self) -> bool {
        self.category == "balance"
    }

    /// Human-readable display string.
    pub fn display(&self) -> String {
        format!("{}: {} → {}", self.key, self.before, self.after)
    }
}

// ──────────────────────────── GasEstimate ──────────────────────────────

/// Gas cost estimate for a transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasEstimate {
    pub base_gas: u64,
    pub priority_fee: u64,
    pub max_fee: u64,
    pub estimated_cost: u64,
    pub confidence: f64,
}

impl GasEstimate {
    pub fn new(base_gas: u64, priority_fee: u64, max_fee: u64) -> Self {
        Self {
            base_gas,
            priority_fee,
            max_fee,
            estimated_cost: base_gas + priority_fee,
            confidence: 0.8,
        }
    }

    /// Total gas cost (base + priority).
    pub fn total(&self) -> u64 {
        self.base_gas + self.priority_fee
    }

    /// Set confidence level (0.0–1.0).
    pub fn with_confidence(mut self, c: f64) -> Self {
        self.confidence = c;
        self
    }
}

// ──────────────────────────── TxPreview ────────────────────────────────

/// A full transaction preview with state changes, gas, and risk warnings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxPreview {
    pub tx_hash: Option<String>,
    pub from_address: String,
    pub to_address: Option<String>,
    pub value: u64,
    pub tx_type: String,
    pub state_changes: Vec<StateChange>,
    pub gas_estimate: GasEstimate,
    pub warnings: Vec<RiskWarning>,
    pub status: PreviewStatus,
    pub summary: String,
    pub created_at: String,
    pub metadata: HashMap<String, String>,
}

impl TxPreview {
    pub fn new(from: &str, to: Option<&str>, value: u64, tx_type: &str) -> Self {
        Self {
            tx_hash: None,
            from_address: from.to_string(),
            to_address: to.map(|s| s.to_string()),
            value,
            tx_type: tx_type.to_string(),
            state_changes: Vec::new(),
            gas_estimate: GasEstimate::new(0, 0, 0),
            warnings: Vec::new(),
            status: PreviewStatus::Safe,
            summary: String::new(),
            created_at: chrono::Utc::now().to_rfc3339(),
            metadata: HashMap::new(),
        }
    }

    /// Add a state change to the preview.
    pub fn add_state_change(&mut self, change: StateChange) {
        self.state_changes.push(change);
    }

    /// Add a risk warning and recalculate status.
    pub fn add_warning(&mut self, warning: RiskWarning) {
        self.warnings.push(warning);
        self.recalculate_status();
    }

    /// Set the gas estimate.
    pub fn set_gas(&mut self, estimate: GasEstimate) {
        self.gas_estimate = estimate;
    }

    /// Set the human-readable summary.
    pub fn set_summary(&mut self, summary: &str) {
        self.summary = summary.to_string();
    }

    /// Add a metadata key-value pair (builder pattern).
    pub fn with_metadata(mut self, key: &str, value: &str) -> Self {
        self.metadata.insert(key.to_string(), value.to_string());
        self
    }

    /// Returns `true` if the preview status is Safe.
    pub fn is_safe(&self) -> bool {
        self.status == PreviewStatus::Safe
    }

    /// Filter state changes to only balance changes.
    pub fn balance_changes(&self) -> Vec<&StateChange> {
        self.state_changes
            .iter()
            .filter(|c| c.is_balance_change())
            .collect()
    }

    /// Total cost including value and gas.
    pub fn total_cost(&self) -> u64 {
        self.value + self.gas_estimate.total()
    }

    /// Number of warnings.
    pub fn warning_count(&self) -> usize {
        self.warnings.len()
    }

    /// Multi-line formatted display.
    pub fn display(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!("Transaction Preview ({:?})", self.status));
        lines.push(format!("  From: {}", self.from_address));
        if let Some(ref to) = self.to_address {
            lines.push(format!("  To:   {}", to));
        }
        lines.push(format!("  Value: {} EVAP", self.value));
        lines.push(format!("  Gas:   {} EVAP", self.gas_estimate.total()));
        lines.push(format!("  Total: {} EVAP", self.total_cost()));
        if !self.warnings.is_empty() {
            lines.push(format!("  Warnings: {}", self.warnings.len()));
            for w in &self.warnings {
                lines.push(format!("    - {:?}", w));
            }
        }
        if !self.summary.is_empty() {
            lines.push(format!("  Summary: {}", self.summary));
        }
        lines.join("\n")
    }

    // ── internal ──

    fn recalculate_status(&mut self) {
        let count = self.warnings.len();
        let has_severe = self
            .warnings
            .iter()
            .any(|w| matches!(w, RiskWarning::HighValue | RiskWarning::SuspiciousPattern));

        self.status = match count {
            0 => PreviewStatus::Safe,
            1..=2 => PreviewStatus::Caution,
            3..=4 => PreviewStatus::Warning,
            _ => PreviewStatus::Danger,
        };

        // Severe warnings force at least Warning level.
        if has_severe && self.status < PreviewStatus::Warning {
            self.status = PreviewStatus::Warning;
        }
    }
}

// ──────────────────────────── PreviewHistory ───────────────────────────

/// Stores past previews for audit purposes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PreviewHistory {
    pub previews: Vec<TxPreview>,
}

impl PreviewHistory {
    pub fn new() -> Self {
        Self {
            previews: Vec::new(),
        }
    }

    pub fn push(&mut self, preview: TxPreview) {
        self.previews.push(preview);
    }

    pub fn len(&self) -> usize {
        self.previews.len()
    }

    pub fn is_empty(&self) -> bool {
        self.previews.is_empty()
    }
}

// ──────────────────────────── PreviewerStats ──────────────────────────

/// Summary statistics for the previewer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewerStats {
    pub total_previews: usize,
    pub safe: usize,
    pub caution: usize,
    pub warning: usize,
    pub danger: usize,
    pub known_addresses: usize,
}

// ──────────────────────────── TxPreviewer ──────────────────────────────

/// Main previewer engine — creates previews with automatic risk detection.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TxPreviewer {
    pub history: Vec<TxPreview>,
    pub known_addresses: HashMap<String, String>,
    pub high_value_threshold: u64,
    pub large_gas_threshold: u64,
}

const MAX_HISTORY: usize = 200;

impl TxPreviewer {
    pub fn new() -> Self {
        Self {
            history: Vec::new(),
            known_addresses: HashMap::new(),
            high_value_threshold: 100_000,
            large_gas_threshold: 50_000,
        }
    }

    /// Set custom thresholds (builder pattern).
    pub fn with_thresholds(mut self, high_value: u64, large_gas: u64) -> Self {
        self.high_value_threshold = high_value;
        self.large_gas_threshold = large_gas;
        self
    }

    /// Register a known address with a label.
    pub fn add_known_address(&mut self, address: &str, label: &str) {
        self.known_addresses
            .insert(address.to_string(), label.to_string());
    }

    /// Check whether an address is registered.
    pub fn is_known(&self, address: &str) -> bool {
        self.known_addresses.contains_key(address)
    }

    /// Create a full transaction preview with automatic risk detection.
    pub fn preview(
        &mut self,
        from: &str,
        to: Option<&str>,
        value: u64,
        tx_type: &str,
        gas: GasEstimate,
        balance: u64,
    ) -> TxPreview {
        let mut preview = TxPreview::new(from, to, value, tx_type);
        preview.set_gas(gas);

        // Auto-detect risks.
        if value > self.high_value_threshold {
            preview.add_warning(RiskWarning::HighValue);
        }
        if let Some(addr) = to {
            if !self.known_addresses.contains_key(addr) {
                preview.add_warning(RiskWarning::UnknownRecipient);
            }
        }
        if preview.gas_estimate.total() > self.large_gas_threshold {
            preview.add_warning(RiskWarning::LargeGas);
        }
        if balance < value + preview.gas_estimate.total() {
            preview.add_warning(RiskWarning::LowBalance);
        }
        if tx_type.contains("contract") {
            preview.add_warning(RiskWarning::ContractInteraction);
        }

        // Build summary.
        let status_label = format!("{:?}", preview.status);
        let summary = if preview.warnings.is_empty() {
            format!("Transfer of {} EVAP — {}", value, status_label)
        } else {
            format!(
                "Transfer of {} EVAP — {} ({} warning{})",
                value,
                status_label,
                preview.warnings.len(),
                if preview.warnings.len() == 1 { "" } else { "s" },
            )
        };
        preview.set_summary(&summary);

        // Push to history, prune if needed.
        self.history.push(preview.clone());
        if self.history.len() > MAX_HISTORY {
            let excess = self.history.len() - MAX_HISTORY;
            self.history.drain(0..excess);
        }

        preview
    }

    /// Convenience wrapper for a simple transfer.
    pub fn preview_transfer(
        &mut self,
        from: &str,
        to: &str,
        value: u64,
        gas: GasEstimate,
        balance: u64,
    ) -> TxPreview {
        self.preview(from, Some(to), value, "transfer", gas, balance)
    }

    /// Return the `n` most recent previews.
    pub fn recent_previews(&self, n: usize) -> Vec<&TxPreview> {
        self.history.iter().rev().take(n).collect()
    }

    /// Return only previews that have warnings.
    pub fn previews_with_warnings(&self) -> Vec<&TxPreview> {
        self.history
            .iter()
            .filter(|p| !p.warnings.is_empty())
            .collect()
    }

    /// Compute summary statistics.
    pub fn stats(&self) -> PreviewerStats {
        let mut safe = 0;
        let mut caution = 0;
        let mut warning = 0;
        let mut danger = 0;
        for p in &self.history {
            match p.status {
                PreviewStatus::Safe => safe += 1,
                PreviewStatus::Caution => caution += 1,
                PreviewStatus::Warning => warning += 1,
                PreviewStatus::Danger => danger += 1,
            }
        }
        PreviewerStats {
            total_previews: self.history.len(),
            safe,
            caution,
            warning,
            danger,
            known_addresses: self.known_addresses.len(),
        }
    }

    // ── Persistence ──────────────────────────────────────────────────

    pub fn save(&self, path: &Path) -> Result<(), TxPreviewError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, TxPreviewError> {
        let data = std::fs::read_to_string(path)?;
        let previewer: Self = serde_json::from_str(&data)?;
        Ok(previewer)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ──────────────────────────── Tests ────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("tx_preview_test_{}_{}", name, std::process::id()))
    }

    fn default_gas() -> GasEstimate {
        GasEstimate::new(1000, 200, 2000)
    }

    // ── StateChange ──

    #[test]
    fn test_state_change_display() {
        let sc = StateChange::new("balance", "1000", "800", "balance");
        assert_eq!(sc.display(), "balance: 1000 → 800");
        assert!(sc.is_balance_change());

        let sc2 = StateChange::new("nonce", "5", "6", "nonce");
        assert!(!sc2.is_balance_change());
    }

    // ── GasEstimate ──

    #[test]
    fn test_gas_estimate() {
        let gas = GasEstimate::new(1000, 200, 2000);
        assert_eq!(gas.base_gas, 1000);
        assert_eq!(gas.priority_fee, 200);
        assert_eq!(gas.max_fee, 2000);
        assert_eq!(gas.estimated_cost, 1200);
        assert!((gas.confidence - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn test_gas_estimate_total() {
        let gas = GasEstimate::new(500, 100, 1000).with_confidence(0.95);
        assert_eq!(gas.total(), 600);
        assert!((gas.confidence - 0.95).abs() < f64::EPSILON);
    }

    // ── TxPreview ──

    #[test]
    fn test_preview_safe_transfer() {
        let preview = TxPreview::new("alice", Some("bob"), 100, "transfer");
        assert_eq!(preview.status, PreviewStatus::Safe);
        assert!(preview.is_safe());
        assert_eq!(preview.warning_count(), 0);
        assert!(preview.warnings.is_empty());
    }

    #[test]
    fn test_preview_high_value_warning() {
        let mut previewer = TxPreviewer::new();
        let preview = previewer.preview(
            "alice",
            Some("bob"),
            200_000,
            "transfer",
            default_gas(),
            500_000,
        );
        assert!(preview.warnings.contains(&RiskWarning::HighValue));
        assert!(preview.status >= PreviewStatus::Warning);
    }

    #[test]
    fn test_preview_unknown_recipient() {
        let mut previewer = TxPreviewer::new();
        let preview = previewer.preview(
            "alice",
            Some("stranger"),
            100,
            "transfer",
            default_gas(),
            500_000,
        );
        assert!(preview.warnings.contains(&RiskWarning::UnknownRecipient));
    }

    #[test]
    fn test_preview_large_gas() {
        let mut previewer = TxPreviewer::new();
        let gas = GasEstimate::new(40_000, 20_000, 80_000);
        let preview = previewer.preview("alice", Some("bob"), 100, "transfer", gas, 500_000);
        assert!(preview.warnings.contains(&RiskWarning::LargeGas));
    }

    #[test]
    fn test_preview_low_balance() {
        let mut previewer = TxPreviewer::new();
        let preview = previewer.preview("alice", Some("bob"), 5000, "transfer", default_gas(), 100);
        assert!(preview.warnings.contains(&RiskWarning::LowBalance));
    }

    #[test]
    fn test_preview_contract_interaction() {
        let mut previewer = TxPreviewer::new();
        let preview = previewer.preview(
            "alice",
            Some("0xcontract"),
            100,
            "contract_call",
            default_gas(),
            500_000,
        );
        assert!(preview.warnings.contains(&RiskWarning::ContractInteraction));
    }

    #[test]
    fn test_preview_multiple_warnings_escalates() {
        let mut previewer = TxPreviewer::new();
        // high value + unknown + contract + large gas + low balance = 5 warnings → Danger
        let gas = GasEstimate::new(40_000, 20_000, 80_000);
        let preview = previewer.preview(
            "alice",
            Some("stranger"),
            200_000,
            "contract_call",
            gas,
            100,
        );
        assert!(preview.warnings.len() >= 5);
        assert_eq!(preview.status, PreviewStatus::Danger);
    }

    #[test]
    fn test_preview_suspicious_pattern_forces_warning() {
        let mut preview = TxPreview::new("alice", Some("bob"), 10, "transfer");
        preview.add_warning(RiskWarning::SuspiciousPattern);
        assert!(preview.status >= PreviewStatus::Warning);
    }

    #[test]
    fn test_add_known_address() {
        let mut previewer = TxPreviewer::new();
        assert!(!previewer.is_known("bob"));
        previewer.add_known_address("bob", "Bob's Wallet");
        assert!(previewer.is_known("bob"));
    }

    #[test]
    fn test_preview_known_recipient_no_warning() {
        let mut previewer = TxPreviewer::new();
        previewer.add_known_address("bob", "Bob's Wallet");
        let preview = previewer.preview(
            "alice",
            Some("bob"),
            100,
            "transfer",
            default_gas(),
            500_000,
        );
        assert!(!preview.warnings.contains(&RiskWarning::UnknownRecipient));
    }

    #[test]
    fn test_preview_transfer_convenience() {
        let mut previewer = TxPreviewer::new();
        previewer.add_known_address("bob", "Bob");
        let preview = previewer.preview_transfer("alice", "bob", 500, default_gas(), 500_000);
        assert_eq!(preview.tx_type, "transfer");
        assert_eq!(preview.value, 500);
        assert_eq!(preview.to_address, Some("bob".to_string()));
    }

    #[test]
    fn test_balance_changes() {
        let mut preview = TxPreview::new("alice", Some("bob"), 100, "transfer");
        preview.add_state_change(StateChange::new("alice_balance", "1000", "900", "balance"));
        preview.add_state_change(StateChange::new("nonce", "5", "6", "nonce"));
        preview.add_state_change(StateChange::new("bob_balance", "500", "600", "balance"));

        let balance = preview.balance_changes();
        assert_eq!(balance.len(), 2);
        assert!(balance.iter().all(|c| c.is_balance_change()));
    }

    #[test]
    fn test_total_cost() {
        let mut preview = TxPreview::new("alice", Some("bob"), 1000, "transfer");
        preview.set_gas(GasEstimate::new(500, 100, 1000));
        assert_eq!(preview.total_cost(), 1600); // 1000 + 500 + 100
    }

    #[test]
    fn test_preview_display() {
        let mut previewer = TxPreviewer::new();
        previewer.add_known_address("bob", "Bob");
        let preview = previewer.preview_transfer("alice", "bob", 500, default_gas(), 500_000);
        let text = preview.display();
        assert!(text.contains("Transaction Preview"));
        assert!(text.contains("alice"));
        assert!(text.contains("bob"));
        assert!(text.contains("500 EVAP"));
    }

    #[test]
    fn test_recent_previews() {
        let mut previewer = TxPreviewer::new();
        previewer.add_known_address("bob", "Bob");
        for i in 0..5 {
            previewer.preview_transfer("alice", "bob", i * 100, default_gas(), 500_000);
        }
        let recent = previewer.recent_previews(3);
        assert_eq!(recent.len(), 3);
        // Most recent first.
        assert_eq!(recent[0].value, 400);
        assert_eq!(recent[1].value, 300);
        assert_eq!(recent[2].value, 200);
    }

    #[test]
    fn test_previews_with_warnings() {
        let mut previewer = TxPreviewer::new();
        previewer.add_known_address("bob", "Bob");
        // Safe transfer.
        previewer.preview_transfer("alice", "bob", 100, default_gas(), 500_000);
        // High value transfer (triggers HighValue warning).
        previewer.preview_transfer("alice", "bob", 200_000, default_gas(), 500_000);

        let warned = previewer.previews_with_warnings();
        assert_eq!(warned.len(), 1);
        assert!(warned[0].warnings.contains(&RiskWarning::HighValue));
    }

    #[test]
    fn test_history_capped() {
        let mut previewer = TxPreviewer::new();
        previewer.add_known_address("bob", "Bob");
        for _ in 0..210 {
            previewer.preview_transfer("alice", "bob", 10, default_gas(), 500_000);
        }
        assert_eq!(previewer.history.len(), MAX_HISTORY);
    }

    #[test]
    fn test_stats() {
        let mut previewer = TxPreviewer::new();
        previewer.add_known_address("bob", "Bob");
        // 2 safe.
        previewer.preview_transfer("alice", "bob", 100, default_gas(), 500_000);
        previewer.preview_transfer("alice", "bob", 200, default_gas(), 500_000);
        // 1 with warning (high value).
        previewer.preview_transfer("alice", "bob", 200_000, default_gas(), 500_000);

        let stats = previewer.stats();
        assert_eq!(stats.total_previews, 3);
        assert_eq!(stats.safe, 2);
        assert_eq!(stats.known_addresses, 1);
        assert!(stats.warning >= 1 || stats.caution >= 1 || stats.danger >= 1);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path("roundtrip");

        let mut previewer = TxPreviewer::new().with_thresholds(50_000, 25_000);
        previewer.add_known_address("bob", "Bob");
        previewer.preview_transfer("alice", "bob", 1000, default_gas(), 500_000);

        previewer.save(&path).expect("save failed");

        let loaded = TxPreviewer::load(&path).expect("load failed");
        assert_eq!(loaded.history.len(), 1);
        assert_eq!(loaded.known_addresses.len(), 1);
        assert_eq!(loaded.high_value_threshold, 50_000);
        assert_eq!(loaded.large_gas_threshold, 25_000);

        // load_or_default with valid file.
        let from_default = TxPreviewer::load_or_default(&path);
        assert_eq!(from_default.history.len(), 1);

        // load_or_default with missing file.
        let missing = TxPreviewer::load_or_default(Path::new("/tmp/nonexistent_tx_preview.json"));
        assert_eq!(missing.history.len(), 0);

        // Clean up.
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_with_metadata_covers_lines_184_187() {
        let preview = TxPreview::new("alice", Some("bob"), 100, "transfer")
            .with_metadata("note", "test payment")
            .with_metadata("tag", "urgent");
        assert_eq!(preview.metadata.get("note").unwrap(), "test payment");
        assert_eq!(preview.metadata.get("tag").unwrap(), "urgent");
    }

    #[test]
    fn test_display_with_warnings_covers_lines_224_227() {
        let mut previewer = TxPreviewer::new();
        // High value + unknown recipient = warnings → triggers display branches
        let preview = previewer.preview(
            "alice",
            Some("unknown_addr"),
            200_000,
            "transfer",
            default_gas(),
            500_000,
        );
        let output = preview.display();
        assert!(output.contains("Warnings:"));
        assert!(output.contains("Summary:"));
    }

    #[test]
    fn test_recalculate_status_severe_covers_line_245() {
        let mut preview = TxPreview::new("alice", Some("bob"), 100, "transfer");
        // 1 warning total but it's severe (HighValue) → forces status to Warning
        preview.add_warning(RiskWarning::HighValue);
        assert_eq!(preview.status, PreviewStatus::Warning);
    }

    #[test]
    fn test_preview_history_covers_lines_267_283() {
        let mut h = PreviewHistory::new();
        assert!(h.is_empty());
        assert_eq!(h.len(), 0);
        h.push(TxPreview::new("alice", None, 100, "transfer"));
        assert!(!h.is_empty());
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn test_preview_with_none_to_covers_line_361() {
        let mut previewer = TxPreviewer::new();
        // to = None → no UnknownRecipient warning; closing brace of if let Some(addr) is hit
        let preview = previewer.preview("alice", None, 100, "transfer", default_gas(), 500_000);
        assert!(!preview.warnings.contains(&RiskWarning::UnknownRecipient));
    }

    #[test]
    fn test_stats_warning_and_danger_covers_lines_431_433() {
        let mut previewer = TxPreviewer::new();
        // 3 warnings → Warning status
        let mut p1 = TxPreview::new("alice", None, 100, "transfer");
        p1.add_warning(RiskWarning::HighValue);
        p1.add_warning(RiskWarning::LargeGas);
        p1.add_warning(RiskWarning::LowBalance);
        previewer.history.push(p1);
        // 5 warnings → Danger status
        let mut p2 = TxPreview::new("alice", None, 100, "transfer");
        p2.add_warning(RiskWarning::HighValue);
        p2.add_warning(RiskWarning::LargeGas);
        p2.add_warning(RiskWarning::LowBalance);
        p2.add_warning(RiskWarning::ContractInteraction);
        p2.add_warning(RiskWarning::UnknownRecipient);
        previewer.history.push(p2);
        let stats = previewer.stats();
        assert_eq!(stats.warning, 1);
        assert_eq!(stats.danger, 1);
    }
}
