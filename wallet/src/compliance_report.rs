//! Regulatory Compliance Report Generator — transaction categorization,
//! jurisdiction-aware tax estimation, and report lifecycle management.
//!
//! Tracks categorized transactions, generates compliance reports per jurisdiction,
//! and estimates taxes based on configurable jurisdiction rules.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

// ──────────────────────────── Types ──────────────────────────────────────

#[derive(Debug, Clone, thiserror::Error)]
pub enum ComplianceError {
    #[error("transaction not found: {0}")]
    TxNotFound(String),
    #[error("report not found: {0}")]
    ReportNotFound(String),
    #[error("no jurisdiction rule for: {0}")]
    NoJurisdictionRule(String),
    #[error("invalid state transition: {0}")]
    InvalidState(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
}

impl From<std::io::Error> for ComplianceError {
    fn from(e: std::io::Error) -> Self { ComplianceError::Io(e.to_string()) }
}
impl From<serde_json::Error> for ComplianceError {
    fn from(e: serde_json::Error) -> Self { ComplianceError::Json(e.to_string()) }
}

/// Category of a transaction for compliance purposes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxCategory {
    Trade,
    Income,
    Gift,
    Airdrop,
    Staking,
    Mining,
    Fee,
    Transfer,
    Unknown,
}

/// Jurisdiction for regulatory reporting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Jurisdiction {
    US,
    UK,
    EU,
    Singapore,
    Japan,
    Australia,
    Custom(String),
}

impl Jurisdiction {
    /// Return a stable string key for HashMap lookups.
    pub fn key(&self) -> String {
        match self {
            Jurisdiction::US => "US".to_string(),
            Jurisdiction::UK => "UK".to_string(),
            Jurisdiction::EU => "EU".to_string(),
            Jurisdiction::Singapore => "Singapore".to_string(),
            Jurisdiction::Japan => "Japan".to_string(),
            Jurisdiction::Australia => "Australia".to_string(),
            Jurisdiction::Custom(s) => s.clone(),
        }
    }
}

/// Report time period type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportType {
    Annual,
    Quarterly,
    Monthly,
    Custom { start: String, end: String },
}

/// Status of a compliance report in its lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportStatus {
    Draft,
    Generated,
    Reviewed,
    Submitted,
}

// ──────────────────────────── Data Structs ───────────────────────────────

/// A transaction categorized for compliance reporting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategorizedTx {
    pub tx_hash: String,
    pub timestamp: String,
    pub category: TxCategory,
    pub token: String,
    pub amount: u64,
    pub value_usd: f64,
    pub counterparty: Option<String>,
    pub notes: String,
    pub flagged: bool,
}

/// A generated compliance report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceReport {
    pub id: String,
    pub report_type: ReportType,
    pub jurisdiction: Jurisdiction,
    pub status: ReportStatus,
    pub created_at: String,
    pub period_start: String,
    pub period_end: String,
    pub total_income: f64,
    pub total_gains: f64,
    pub total_losses: f64,
    pub total_fees: f64,
    pub tx_count: usize,
    pub flagged_count: usize,
}

/// Tax rules for a specific jurisdiction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JurisdictionRule {
    pub jurisdiction: Jurisdiction,
    pub short_term_days: u32,
    pub tax_rate_short: f64,
    pub tax_rate_long: f64,
    pub reporting_threshold: f64,
    pub requires_tx_detail: bool,
}

/// Aggregate statistics about the compliance manager state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceStats {
    pub total_transactions: usize,
    pub categorized: usize,
    pub uncategorized: usize,
    pub flagged: usize,
    pub reports_generated: usize,
    pub jurisdictions: usize,
}

// ──────────────────────────── Manager ────────────────────────────────────

/// Central store for compliance data — transactions, reports, and rules.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComplianceManager {
    pub transactions: Vec<CategorizedTx>,
    pub reports: HashMap<String, ComplianceReport>,
    pub rules: HashMap<String, JurisdictionRule>,
}

impl ComplianceManager {
    /// Create an empty compliance manager.
    pub fn new() -> Self {
        Self::default()
    }

    // ── Persistence ─────────────────────────────────────────────────────

    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ComplianceError> {
        let data = std::fs::read_to_string(path)?;
        let mgr: ComplianceManager = serde_json::from_str(&data)?;
        Ok(mgr)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), ComplianceError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }

    // ── Transactions ────────────────────────────────────────────────────

    /// Append a categorized transaction.
    pub fn add_transaction(&mut self, tx: CategorizedTx) {
        self.transactions.push(tx);
    }

    /// Update the category of a transaction by hash.
    pub fn categorize(&mut self, tx_hash: &str, category: TxCategory) -> Result<(), ComplianceError> {
        let tx = self.transactions.iter_mut()
            .find(|t| t.tx_hash == tx_hash)
            .ok_or_else(|| ComplianceError::TxNotFound(tx_hash.to_string()))?;
        tx.category = category;
        Ok(())
    }

    /// Flag a transaction for review.
    pub fn flag_transaction(&mut self, tx_hash: &str) -> Result<(), ComplianceError> {
        let tx = self.transactions.iter_mut()
            .find(|t| t.tx_hash == tx_hash)
            .ok_or_else(|| ComplianceError::TxNotFound(tx_hash.to_string()))?;
        tx.flagged = true;
        Ok(())
    }

    /// Remove flag from a transaction.
    pub fn unflag_transaction(&mut self, tx_hash: &str) -> Result<(), ComplianceError> {
        let tx = self.transactions.iter_mut()
            .find(|t| t.tx_hash == tx_hash)
            .ok_or_else(|| ComplianceError::TxNotFound(tx_hash.to_string()))?;
        tx.flagged = false;
        Ok(())
    }

    // ── Rules ───────────────────────────────────────────────────────────

    /// Add or update a jurisdiction rule.
    pub fn add_rule(&mut self, rule: JurisdictionRule) {
        let key = rule.jurisdiction.key();
        self.rules.insert(key, rule);
    }

    // ── Reports ─────────────────────────────────────────────────────────

    /// Derive the period start/end timestamps from the report type.
    fn period_bounds(report_type: &ReportType) -> (String, String) {
        let now = chrono::Utc::now();
        match report_type {
            ReportType::Annual => {
                let year = now.format("%Y").to_string();
                (format!("{}-01-01T00:00:00Z", year), format!("{}-12-31T23:59:59Z", year))
            }
            ReportType::Quarterly => {
                let month = now.format("%m").to_string().parse::<u32>().unwrap_or(1);
                let q_start = match month {
                    1..=3 => "01",
                    4..=6 => "04",
                    7..=9 => "07",
                    _ => "10",
                };
                let q_end = match month {
                    1..=3 => "03",
                    4..=6 => "06",
                    7..=9 => "09",
                    _ => "12",
                };
                let year = now.format("%Y").to_string();
                (
                    format!("{}-{}-01T00:00:00Z", year, q_start),
                    format!("{}-{}-31T23:59:59Z", year, q_end),
                )
            }
            ReportType::Monthly => {
                let ym = now.format("%Y-%m").to_string();
                (format!("{}-01T00:00:00Z", ym), format!("{}-31T23:59:59Z", ym))
            }
            ReportType::Custom { start, end } => (start.clone(), end.clone()),
        }
    }

    /// Generate a compliance report, aggregating transactions within the period.
    pub fn generate_report(
        &mut self,
        report_type: ReportType,
        jurisdiction: Jurisdiction,
    ) -> Result<String, ComplianceError> {
        let (period_start, period_end) = Self::period_bounds(&report_type);
        let txs = self.transactions_in_period(&period_start, &period_end);

        let mut total_income = 0.0_f64;
        let mut total_gains = 0.0_f64;
        let mut total_losses = 0.0_f64;
        let mut total_fees = 0.0_f64;
        let mut flagged_count = 0_usize;

        for tx in &txs {
            match tx.category {
                TxCategory::Income | TxCategory::Staking | TxCategory::Mining | TxCategory::Airdrop => {
                    total_income += tx.value_usd;
                }
                TxCategory::Trade => {
                    if tx.value_usd >= 0.0 {
                        total_gains += tx.value_usd;
                    } else {
                        total_losses += tx.value_usd.abs();
                    }
                }
                TxCategory::Fee => {
                    total_fees += tx.value_usd;
                }
                _ => {}
            }
            if tx.flagged {
                flagged_count += 1;
            }
        }

        let tx_count = txs.len();
        let id = format!("rpt-{}", chrono::Utc::now().timestamp_millis());

        let report = ComplianceReport {
            id: id.clone(),
            report_type,
            jurisdiction,
            status: ReportStatus::Generated,
            created_at: chrono::Utc::now().to_rfc3339(),
            period_start,
            period_end,
            total_income,
            total_gains,
            total_losses,
            total_fees,
            tx_count,
            flagged_count,
        };

        self.reports.insert(id.clone(), report);
        Ok(id)
    }

    /// Get a report by ID.
    pub fn get_report(&self, id: &str) -> Option<&ComplianceReport> {
        self.reports.get(id)
    }

    /// Transition a report to Reviewed status.
    pub fn mark_reviewed(&mut self, report_id: &str) -> Result<(), ComplianceError> {
        let report = self.reports.get_mut(report_id)
            .ok_or_else(|| ComplianceError::ReportNotFound(report_id.to_string()))?;
        if report.status != ReportStatus::Generated {
            return Err(ComplianceError::InvalidState(
                format!("report must be Generated to review, currently {:?}", report.status),
            ));
        }
        report.status = ReportStatus::Reviewed;
        Ok(())
    }

    /// Transition a report to Submitted status.
    pub fn mark_submitted(&mut self, report_id: &str) -> Result<(), ComplianceError> {
        let report = self.reports.get_mut(report_id)
            .ok_or_else(|| ComplianceError::ReportNotFound(report_id.to_string()))?;
        if report.status != ReportStatus::Reviewed {
            return Err(ComplianceError::InvalidState(
                format!("report must be Reviewed to submit, currently {:?}", report.status),
            ));
        }
        report.status = ReportStatus::Submitted;
        Ok(())
    }

    // ── Queries ─────────────────────────────────────────────────────────

    /// Return transactions whose timestamp falls within [start, end] (string comparison).
    pub fn transactions_in_period<'a>(&'a self, start: &str, end: &str) -> Vec<&'a CategorizedTx> {
        self.transactions.iter()
            .filter(|tx| tx.timestamp.as_str() >= start && tx.timestamp.as_str() <= end)
            .collect()
    }

    /// Return all flagged transactions.
    pub fn flagged_transactions(&self) -> Vec<&CategorizedTx> {
        self.transactions.iter().filter(|tx| tx.flagged).collect()
    }

    /// Return transactions matching a given category.
    pub fn transactions_by_category(&self, cat: &TxCategory) -> Vec<&CategorizedTx> {
        self.transactions.iter().filter(|tx| tx.category == *cat).collect()
    }

    /// Estimate tax for a report using jurisdiction rules.
    /// Computes: (total_gains - total_losses) * applicable short-term rate.
    pub fn estimate_tax(&self, report_id: &str) -> Result<f64, ComplianceError> {
        let report = self.reports.get(report_id)
            .ok_or_else(|| ComplianceError::ReportNotFound(report_id.to_string()))?;
        let jkey = report.jurisdiction.key();
        let rule = self.rules.get(&jkey)
            .ok_or(ComplianceError::NoJurisdictionRule(jkey))?;

        let net_gains = report.total_gains - report.total_losses;
        if net_gains <= 0.0 {
            return Ok(0.0);
        }
        // Use short-term rate as a conservative default estimate.
        Ok(net_gains * rule.tax_rate_short)
    }

    /// Compute aggregate statistics.
    pub fn stats(&self) -> ComplianceStats {
        let total_transactions = self.transactions.len();
        let uncategorized = self.transactions.iter()
            .filter(|tx| tx.category == TxCategory::Unknown)
            .count();
        let categorized = total_transactions - uncategorized;
        let flagged = self.transactions.iter().filter(|tx| tx.flagged).count();
        let reports_generated = self.reports.len();
        let jurisdictions = self.rules.len();

        ComplianceStats {
            total_transactions,
            categorized,
            uncategorized,
            flagged,
            reports_generated,
            jurisdictions,
        }
    }
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tx(hash: &str, category: TxCategory, value_usd: f64) -> CategorizedTx {
        CategorizedTx {
            tx_hash: hash.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            category,
            token: "EVAP".to_string(),
            amount: 1000,
            value_usd,
            counterparty: None,
            notes: String::new(),
            flagged: false,
        }
    }

    fn make_tx_at(hash: &str, timestamp: &str, category: TxCategory, value_usd: f64) -> CategorizedTx {
        CategorizedTx {
            tx_hash: hash.to_string(),
            timestamp: timestamp.to_string(),
            category,
            token: "EVAP".to_string(),
            amount: 500,
            value_usd,
            counterparty: None,
            notes: String::new(),
            flagged: false,
        }
    }

    fn us_rule() -> JurisdictionRule {
        JurisdictionRule {
            jurisdiction: Jurisdiction::US,
            short_term_days: 365,
            tax_rate_short: 0.37,
            tax_rate_long: 0.20,
            reporting_threshold: 600.0,
            requires_tx_detail: true,
        }
    }

    fn uk_rule() -> JurisdictionRule {
        JurisdictionRule {
            jurisdiction: Jurisdiction::UK,
            short_term_days: 365,
            tax_rate_short: 0.20,
            tax_rate_long: 0.10,
            reporting_threshold: 12300.0,
            requires_tx_detail: false,
        }
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!("evap_compliance_test_{}_{}", std::process::id(), name))
    }

    // ── Basic CRUD ──────────────────────────────────────────────────────

    #[test]
    fn test_new_manager_is_empty() {
        let mgr = ComplianceManager::new();
        assert!(mgr.transactions.is_empty());
        assert!(mgr.reports.is_empty());
        assert!(mgr.rules.is_empty());
    }

    #[test]
    fn test_add_transaction() {
        let mut mgr = ComplianceManager::new();
        mgr.add_transaction(make_tx("tx1", TxCategory::Trade, 100.0));
        mgr.add_transaction(make_tx("tx2", TxCategory::Income, 50.0));
        assert_eq!(mgr.transactions.len(), 2);
    }

    #[test]
    fn test_categorize_success() {
        let mut mgr = ComplianceManager::new();
        mgr.add_transaction(make_tx("tx1", TxCategory::Unknown, 100.0));
        mgr.categorize("tx1", TxCategory::Trade).unwrap();
        assert_eq!(mgr.transactions[0].category, TxCategory::Trade);
    }

    #[test]
    fn test_categorize_not_found() {
        let mut mgr = ComplianceManager::new();
        let err = mgr.categorize("missing", TxCategory::Trade).unwrap_err();
        assert!(matches!(err, ComplianceError::TxNotFound(_)));
    }

    #[test]
    fn test_flag_transaction() {
        let mut mgr = ComplianceManager::new();
        mgr.add_transaction(make_tx("tx1", TxCategory::Trade, 100.0));
        assert!(!mgr.transactions[0].flagged);
        mgr.flag_transaction("tx1").unwrap();
        assert!(mgr.transactions[0].flagged);
    }

    #[test]
    fn test_unflag_transaction() {
        let mut mgr = ComplianceManager::new();
        let mut tx = make_tx("tx1", TxCategory::Trade, 100.0);
        tx.flagged = true;
        mgr.add_transaction(tx);
        mgr.unflag_transaction("tx1").unwrap();
        assert!(!mgr.transactions[0].flagged);
    }

    #[test]
    fn test_flag_not_found() {
        let mut mgr = ComplianceManager::new();
        assert!(mgr.flag_transaction("nope").is_err());
        assert!(mgr.unflag_transaction("nope").is_err());
    }

    // ── Rules ───────────────────────────────────────────────────────────

    #[test]
    fn test_add_rule() {
        let mut mgr = ComplianceManager::new();
        mgr.add_rule(us_rule());
        mgr.add_rule(uk_rule());
        assert_eq!(mgr.rules.len(), 2);
        assert!(mgr.rules.contains_key("US"));
        assert!(mgr.rules.contains_key("UK"));
    }

    #[test]
    fn test_add_rule_overwrites() {
        let mut mgr = ComplianceManager::new();
        mgr.add_rule(us_rule());
        let mut updated = us_rule();
        updated.tax_rate_short = 0.40;
        mgr.add_rule(updated);
        assert_eq!(mgr.rules.len(), 1);
        assert_eq!(mgr.rules["US"].tax_rate_short, 0.40);
    }

    // ── Queries ─────────────────────────────────────────────────────────

    #[test]
    fn test_transactions_in_period() {
        let mut mgr = ComplianceManager::new();
        mgr.add_transaction(make_tx_at("tx1", "2026-01-15T00:00:00Z", TxCategory::Trade, 100.0));
        mgr.add_transaction(make_tx_at("tx2", "2026-06-15T00:00:00Z", TxCategory::Income, 200.0));
        mgr.add_transaction(make_tx_at("tx3", "2025-12-31T23:59:59Z", TxCategory::Fee, 10.0));

        let in_range = mgr.transactions_in_period("2026-01-01T00:00:00Z", "2026-12-31T23:59:59Z");
        assert_eq!(in_range.len(), 2);
    }

    #[test]
    fn test_flagged_transactions() {
        let mut mgr = ComplianceManager::new();
        mgr.add_transaction(make_tx("tx1", TxCategory::Trade, 100.0));
        let mut flagged = make_tx("tx2", TxCategory::Income, 50.0);
        flagged.flagged = true;
        mgr.add_transaction(flagged);

        let result = mgr.flagged_transactions();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].tx_hash, "tx2");
    }

    #[test]
    fn test_transactions_by_category() {
        let mut mgr = ComplianceManager::new();
        mgr.add_transaction(make_tx("tx1", TxCategory::Trade, 100.0));
        mgr.add_transaction(make_tx("tx2", TxCategory::Income, 50.0));
        mgr.add_transaction(make_tx("tx3", TxCategory::Trade, 200.0));

        let trades = mgr.transactions_by_category(&TxCategory::Trade);
        assert_eq!(trades.len(), 2);
        let income = mgr.transactions_by_category(&TxCategory::Income);
        assert_eq!(income.len(), 1);
    }

    // ── Report Generation ───────────────────────────────────────────────

    #[test]
    fn test_generate_custom_report() {
        let mut mgr = ComplianceManager::new();
        mgr.add_rule(us_rule());
        mgr.add_transaction(make_tx_at("tx1", "2026-03-01T00:00:00Z", TxCategory::Income, 500.0));
        mgr.add_transaction(make_tx_at("tx2", "2026-03-15T00:00:00Z", TxCategory::Trade, 300.0));
        mgr.add_transaction(make_tx_at("tx3", "2026-03-20T00:00:00Z", TxCategory::Fee, 25.0));

        let report_type = ReportType::Custom {
            start: "2026-03-01T00:00:00Z".to_string(),
            end: "2026-03-31T23:59:59Z".to_string(),
        };
        let id = mgr.generate_report(report_type, Jurisdiction::US).unwrap();
        let report = mgr.get_report(&id).unwrap();

        assert_eq!(report.tx_count, 3);
        assert_eq!(report.total_income, 500.0);
        assert_eq!(report.total_gains, 300.0);
        assert_eq!(report.total_fees, 25.0);
        assert_eq!(report.status, ReportStatus::Generated);
    }

    #[test]
    fn test_report_lifecycle() {
        let mut mgr = ComplianceManager::new();
        mgr.add_rule(us_rule());
        mgr.add_transaction(make_tx_at("tx1", "2026-06-01T00:00:00Z", TxCategory::Trade, 100.0));

        let rt = ReportType::Custom {
            start: "2026-01-01T00:00:00Z".to_string(),
            end: "2026-12-31T23:59:59Z".to_string(),
        };
        let id = mgr.generate_report(rt, Jurisdiction::US).unwrap();

        assert_eq!(mgr.get_report(&id).unwrap().status, ReportStatus::Generated);
        mgr.mark_reviewed(&id).unwrap();
        assert_eq!(mgr.get_report(&id).unwrap().status, ReportStatus::Reviewed);
        mgr.mark_submitted(&id).unwrap();
        assert_eq!(mgr.get_report(&id).unwrap().status, ReportStatus::Submitted);
    }

    #[test]
    fn test_mark_reviewed_wrong_state() {
        let mut mgr = ComplianceManager::new();
        mgr.add_rule(us_rule());
        let rt = ReportType::Custom {
            start: "2026-01-01T00:00:00Z".to_string(),
            end: "2026-12-31T23:59:59Z".to_string(),
        };
        let id = mgr.generate_report(rt, Jurisdiction::US).unwrap();
        mgr.mark_reviewed(&id).unwrap();
        // Trying to review again should fail
        let err = mgr.mark_reviewed(&id).unwrap_err();
        assert!(matches!(err, ComplianceError::InvalidState(_)));
    }

    #[test]
    fn test_mark_submitted_wrong_state() {
        let mut mgr = ComplianceManager::new();
        mgr.add_rule(us_rule());
        let rt = ReportType::Custom {
            start: "2026-01-01T00:00:00Z".to_string(),
            end: "2026-12-31T23:59:59Z".to_string(),
        };
        let id = mgr.generate_report(rt, Jurisdiction::US).unwrap();
        // Skip reviewed, go straight to submitted
        let err = mgr.mark_submitted(&id).unwrap_err();
        assert!(matches!(err, ComplianceError::InvalidState(_)));
    }

    #[test]
    fn test_get_report_not_found() {
        let mgr = ComplianceManager::new();
        assert!(mgr.get_report("nonexistent").is_none());
    }

    #[test]
    fn test_mark_reviewed_not_found() {
        let mut mgr = ComplianceManager::new();
        let err = mgr.mark_reviewed("nope").unwrap_err();
        assert!(matches!(err, ComplianceError::ReportNotFound(_)));
    }

    // ── Tax Estimation ──────────────────────────────────────────────────

    #[test]
    fn test_estimate_tax() {
        let mut mgr = ComplianceManager::new();
        mgr.add_rule(us_rule());
        mgr.add_transaction(make_tx_at("tx1", "2026-05-01T00:00:00Z", TxCategory::Trade, 1000.0));
        mgr.add_transaction(make_tx_at("tx2", "2026-05-10T00:00:00Z", TxCategory::Fee, 50.0));

        let rt = ReportType::Custom {
            start: "2026-01-01T00:00:00Z".to_string(),
            end: "2026-12-31T23:59:59Z".to_string(),
        };
        let id = mgr.generate_report(rt, Jurisdiction::US).unwrap();
        let tax = mgr.estimate_tax(&id).unwrap();
        // gains=1000, losses=0 => net=1000 * 0.37 = 370.0
        assert!((tax - 370.0).abs() < 0.01);
    }

    #[test]
    fn test_estimate_tax_no_rule() {
        let mut mgr = ComplianceManager::new();
        // No rules added
        mgr.add_transaction(make_tx_at("tx1", "2026-05-01T00:00:00Z", TxCategory::Trade, 100.0));
        let rt = ReportType::Custom {
            start: "2026-01-01T00:00:00Z".to_string(),
            end: "2026-12-31T23:59:59Z".to_string(),
        };
        let id = mgr.generate_report(rt, Jurisdiction::US).unwrap();
        let err = mgr.estimate_tax(&id).unwrap_err();
        assert!(matches!(err, ComplianceError::NoJurisdictionRule(_)));
    }

    #[test]
    fn test_estimate_tax_net_negative() {
        let mut mgr = ComplianceManager::new();
        mgr.add_rule(us_rule());
        // Trade with negative value => loss
        let mut tx = make_tx_at("tx1", "2026-05-01T00:00:00Z", TxCategory::Trade, -500.0);
        tx.value_usd = -500.0;
        mgr.add_transaction(tx);

        let rt = ReportType::Custom {
            start: "2026-01-01T00:00:00Z".to_string(),
            end: "2026-12-31T23:59:59Z".to_string(),
        };
        let id = mgr.generate_report(rt, Jurisdiction::US).unwrap();
        let tax = mgr.estimate_tax(&id).unwrap();
        assert_eq!(tax, 0.0);
    }

    // ── Stats ───────────────────────────────────────────────────────────

    #[test]
    fn test_stats() {
        let mut mgr = ComplianceManager::new();
        mgr.add_rule(us_rule());
        mgr.add_rule(uk_rule());
        mgr.add_transaction(make_tx("tx1", TxCategory::Trade, 100.0));
        mgr.add_transaction(make_tx("tx2", TxCategory::Unknown, 50.0));
        let mut flagged = make_tx("tx3", TxCategory::Income, 200.0);
        flagged.flagged = true;
        mgr.add_transaction(flagged);

        let s = mgr.stats();
        assert_eq!(s.total_transactions, 3);
        assert_eq!(s.categorized, 2);
        assert_eq!(s.uncategorized, 1);
        assert_eq!(s.flagged, 1);
        assert_eq!(s.reports_generated, 0);
        assert_eq!(s.jurisdictions, 2);
    }

    // ── Persistence ─────────────────────────────────────────────────────

    #[test]
    fn test_save_and_load() {
        let dir = temp_path("save_load");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("compliance.json");

        let mut mgr = ComplianceManager::new();
        mgr.add_rule(us_rule());
        mgr.add_transaction(make_tx("tx1", TxCategory::Trade, 100.0));
        mgr.save(&path).unwrap();

        let loaded = ComplianceManager::load(&path).unwrap();
        assert_eq!(loaded.transactions.len(), 1);
        assert_eq!(loaded.rules.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = temp_path("missing_file").join("nope.json");
        let mgr = ComplianceManager::load_or_default(&path);
        assert!(mgr.transactions.is_empty());
        assert!(mgr.reports.is_empty());
    }

    // ── Jurisdiction Key ────────────────────────────────────────────────

    #[test]
    fn test_jurisdiction_key() {
        assert_eq!(Jurisdiction::US.key(), "US");
        assert_eq!(Jurisdiction::UK.key(), "UK");
        assert_eq!(Jurisdiction::Custom("Bermuda".into()).key(), "Bermuda");
    }

    // ── Default trait ───────────────────────────────────────────────────

    #[test]
    fn test_default_trait() {
        let mgr = ComplianceManager::default();
        assert!(mgr.transactions.is_empty());
        assert!(mgr.reports.is_empty());
        assert!(mgr.rules.is_empty());
    }
}
