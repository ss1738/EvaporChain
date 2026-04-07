//! Tax Report Generation — cost basis calculation, gains/losses, CSV export.
//!
//! Supports FIFO, LIFO, and HIFO (Highest In, First Out) cost basis methods.
//! Tracks acquisition lots, computes realized gains/losses per disposal,
//! and generates CSV reports for tax filing.

use std::path::Path;

use serde::{Deserialize, Serialize};

// ──────────────────────────── Types ──────────────────────────────────────

#[derive(Debug, Clone, thiserror::Error)]
pub enum TaxError {
    #[error("no lots available to dispose")]
    NoLots,
    #[error("insufficient lots: trying to dispose {0} but only {1} available")]
    InsufficientLots(u64, u64),
    #[error("invalid tax year: {0}")]
    InvalidYear(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
}

impl From<std::io::Error> for TaxError {
    fn from(e: std::io::Error) -> Self { TaxError::Io(e.to_string()) }
}
impl From<serde_json::Error> for TaxError {
    fn from(e: serde_json::Error) -> Self { TaxError::Json(e.to_string()) }
}

/// Cost basis method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CostBasisMethod {
    /// First In, First Out.
    Fifo,
    /// Last In, First Out.
    Lifo,
    /// Highest In, First Out (minimize gains).
    Hifo,
}

impl CostBasisMethod {
    pub fn label(&self) -> &'static str {
        match self {
            CostBasisMethod::Fifo => "FIFO",
            CostBasisMethod::Lifo => "LIFO",
            CostBasisMethod::Hifo => "HIFO",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "fifo" => Some(CostBasisMethod::Fifo),
            "lifo" => Some(CostBasisMethod::Lifo),
            "hifo" => Some(CostBasisMethod::Hifo),
            _ => None,
        }
    }
}

/// An acquisition lot — a batch of tokens acquired at a specific cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lot {
    /// Acquisition timestamp (RFC3339).
    pub acquired_at: String,
    /// Number of tokens in this lot (remaining).
    pub amount: u64,
    /// Original amount acquired.
    pub original_amount: u64,
    /// Cost per token (in smallest unit, e.g. USD cents or a reference value).
    pub cost_per_unit: f64,
    /// Source of acquisition.
    pub source: String,
    /// Reference (tx hash, etc.).
    pub reference: String,
}

impl Lot {
    pub fn total_cost(&self) -> f64 {
        self.amount as f64 * self.cost_per_unit
    }
}

/// A disposal event (sell, transfer out, spend).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Disposal {
    /// Timestamp (RFC3339).
    pub timestamp: String,
    /// Amount disposed.
    pub amount: u64,
    /// Proceeds per unit at time of disposal.
    pub proceeds_per_unit: f64,
    /// Total proceeds.
    pub total_proceeds: f64,
    /// Total cost basis (computed from lots).
    pub cost_basis: f64,
    /// Realized gain/loss.
    pub gain_loss: f64,
    /// Whether this is a long-term gain (held > 365 days).
    pub long_term: bool,
    /// Disposal type.
    pub disposal_type: String,
    /// Reference.
    pub reference: String,
    /// Cost basis method used.
    pub method: CostBasisMethod,
}

/// Annual tax summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnualSummary {
    pub year: u32,
    pub method: CostBasisMethod,
    pub total_acquisitions: u64,
    pub total_disposals: u64,
    pub total_proceeds: f64,
    pub total_cost_basis: f64,
    pub total_gain_loss: f64,
    pub short_term_gain: f64,
    pub long_term_gain: f64,
    pub energy_costs: f64,
    pub gas_costs: f64,
    pub disposal_count: usize,
}

// ──────────────────────────── Tracker ────────────────────────────────────

/// Tax tracking engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxTracker {
    /// Open lots (available for disposal).
    pub lots: Vec<Lot>,
    /// Historical disposals.
    pub disposals: Vec<Disposal>,
    /// Preferred cost basis method.
    pub method: CostBasisMethod,
    /// Energy costs tracked separately (deductible as operational expense).
    pub energy_costs: Vec<CostEntry>,
    /// Gas fees tracked separately.
    pub gas_costs: Vec<CostEntry>,
}

/// A simple cost entry for expenses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostEntry {
    pub timestamp: String,
    pub amount: f64,
    pub description: String,
    pub reference: String,
}

impl TaxTracker {
    pub fn new(method: CostBasisMethod) -> Self {
        Self {
            lots: Vec::new(),
            disposals: Vec::new(),
            method,
            energy_costs: Vec::new(),
            gas_costs: Vec::new(),
        }
    }

    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, TaxError> {
        let data = std::fs::read_to_string(path)?;
        let tracker: TaxTracker = serde_json::from_str(&data)?;
        Ok(tracker)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), TaxError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Record an acquisition (buy, receive, mine, faucet, etc.).
    pub fn acquire(
        &mut self,
        amount: u64,
        cost_per_unit: f64,
        source: &str,
        reference: &str,
    ) {
        let lot = Lot {
            acquired_at: chrono::Utc::now().to_rfc3339(),
            amount,
            original_amount: amount,
            cost_per_unit,
            source: source.to_string(),
            reference: reference.to_string(),
        };
        self.lots.push(lot);
    }

    /// Record an acquisition with custom timestamp.
    pub fn acquire_at(
        &mut self,
        timestamp: &str,
        amount: u64,
        cost_per_unit: f64,
        source: &str,
        reference: &str,
    ) {
        let lot = Lot {
            acquired_at: timestamp.to_string(),
            amount,
            original_amount: amount,
            cost_per_unit,
            source: source.to_string(),
            reference: reference.to_string(),
        };
        self.lots.push(lot);
    }

    /// Record a disposal (sell, send, spend). Computes gain/loss using the configured method.
    pub fn dispose(
        &mut self,
        amount: u64,
        proceeds_per_unit: f64,
        disposal_type: &str,
        reference: &str,
    ) -> Result<Disposal, TaxError> {
        let total_available: u64 = self.lots.iter().map(|l| l.amount).sum();
        if total_available == 0 {
            return Err(TaxError::NoLots);
        }
        if total_available < amount {
            return Err(TaxError::InsufficientLots(amount, total_available));
        }

        // Sort lots based on method
        let lot_order = self.get_lot_order();

        let mut remaining = amount;
        let mut total_cost_basis = 0.0;
        let mut earliest_acquired = String::new();

        for &idx in &lot_order {
            if remaining == 0 { break; }
            let lot = &mut self.lots[idx];
            if lot.amount == 0 { continue; }

            let take = remaining.min(lot.amount);
            total_cost_basis += take as f64 * lot.cost_per_unit;
            lot.amount -= take;
            remaining -= take;

            if earliest_acquired.is_empty() || lot.acquired_at < earliest_acquired {
                earliest_acquired = lot.acquired_at.clone();
            }
        }

        // Clean up empty lots
        self.lots.retain(|l| l.amount > 0);

        let total_proceeds = amount as f64 * proceeds_per_unit;
        let gain_loss = total_proceeds - total_cost_basis;

        // Determine if long-term (> 365 days since earliest lot used)
        let long_term = if let Ok(acq) = chrono::DateTime::parse_from_rfc3339(&earliest_acquired) {
            let duration = chrono::Utc::now().signed_duration_since(acq.with_timezone(&chrono::Utc));
            duration.num_days() > 365
        } else {
            false
        };

        let disposal = Disposal {
            timestamp: chrono::Utc::now().to_rfc3339(),
            amount,
            proceeds_per_unit,
            total_proceeds,
            cost_basis: total_cost_basis,
            gain_loss,
            long_term,
            disposal_type: disposal_type.to_string(),
            reference: reference.to_string(),
            method: self.method,
        };
        self.disposals.push(disposal.clone());
        Ok(disposal)
    }

    /// Record an energy cost (operational expense).
    pub fn record_energy_cost(&mut self, amount: f64, description: &str, reference: &str) {
        self.energy_costs.push(CostEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            amount,
            description: description.to_string(),
            reference: reference.to_string(),
        });
    }

    /// Record a gas cost.
    pub fn record_gas_cost(&mut self, amount: f64, reference: &str) {
        self.gas_costs.push(CostEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            amount,
            description: "Gas fee".to_string(),
            reference: reference.to_string(),
        });
    }

    /// Generate annual summary for a given year.
    pub fn annual_summary(&self, year: u32) -> AnnualSummary {
        let year_start = format!("{}-01-01", year);
        let year_end = format!("{}-01-01", year + 1);

        let year_disposals: Vec<&Disposal> = self.disposals.iter()
            .filter(|d| d.timestamp >= year_start && d.timestamp < year_end)
            .collect();

        let year_lots: Vec<&Lot> = self.lots.iter()
            .filter(|l| l.acquired_at >= year_start && l.acquired_at < year_end)
            .collect();

        let total_acquisitions: u64 = year_lots.iter().map(|l| l.original_amount).sum();
        let total_disposals: u64 = year_disposals.iter().map(|d| d.amount).sum();
        let total_proceeds: f64 = year_disposals.iter().map(|d| d.total_proceeds).sum();
        let total_cost_basis: f64 = year_disposals.iter().map(|d| d.cost_basis).sum();
        let total_gain_loss: f64 = year_disposals.iter().map(|d| d.gain_loss).sum();
        let short_term: f64 = year_disposals.iter().filter(|d| !d.long_term).map(|d| d.gain_loss).sum();
        let long_term: f64 = year_disposals.iter().filter(|d| d.long_term).map(|d| d.gain_loss).sum();

        let energy_costs: f64 = self.energy_costs.iter()
            .filter(|c| c.timestamp >= year_start && c.timestamp < year_end)
            .map(|c| c.amount)
            .sum();
        let gas_costs: f64 = self.gas_costs.iter()
            .filter(|c| c.timestamp >= year_start && c.timestamp < year_end)
            .map(|c| c.amount)
            .sum();

        AnnualSummary {
            year,
            method: self.method,
            total_acquisitions,
            total_disposals,
            total_proceeds,
            total_cost_basis,
            total_gain_loss,
            short_term_gain: short_term,
            long_term_gain: long_term,
            energy_costs,
            gas_costs,
            disposal_count: year_disposals.len(),
        }
    }

    /// Export disposals as CSV.
    pub fn disposals_csv(&self) -> String {
        let mut csv = String::from("date,amount,proceeds_per_unit,total_proceeds,cost_basis,gain_loss,long_term,type,method,reference\n");
        for d in &self.disposals {
            csv.push_str(&format!(
                "{},{},{:.4},{:.2},{:.2},{:.2},{},{},{},{}\n",
                &d.timestamp[..10], d.amount, d.proceeds_per_unit,
                d.total_proceeds, d.cost_basis, d.gain_loss,
                d.long_term, d.disposal_type, d.method.label(), d.reference
            ));
        }
        csv
    }

    /// Export open lots as CSV.
    pub fn lots_csv(&self) -> String {
        let mut csv = String::from("acquired,remaining,original,cost_per_unit,total_cost,source,reference\n");
        for l in &self.lots {
            csv.push_str(&format!(
                "{},{},{},{:.4},{:.2},{},{}\n",
                &l.acquired_at[..10], l.amount, l.original_amount,
                l.cost_per_unit, l.total_cost(), l.source, l.reference
            ));
        }
        csv
    }

    /// Total unrealized value of open lots.
    pub fn total_cost_basis(&self) -> f64 {
        self.lots.iter().map(|l| l.total_cost()).sum()
    }

    /// Total tokens in open lots.
    pub fn total_holdings(&self) -> u64 {
        self.lots.iter().map(|l| l.amount).sum()
    }

    /// Set cost basis method.
    pub fn set_method(&mut self, method: CostBasisMethod) {
        self.method = method;
    }

    // Internal: get lot indices in the order dictated by the method.
    fn get_lot_order(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..self.lots.len()).collect();
        match self.method {
            CostBasisMethod::Fifo => {
                // Already in order (oldest first)
            }
            CostBasisMethod::Lifo => {
                indices.reverse();
            }
            CostBasisMethod::Hifo => {
                indices.sort_by(|&a, &b| {
                    self.lots[b].cost_per_unit
                        .partial_cmp(&self.lots[a].cost_per_unit)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
        }
        indices
    }
}

impl Default for TaxTracker {
    fn default() -> Self { Self::new(CostBasisMethod::Fifo) }
}

/// Default path.
pub fn default_tax_path() -> std::path::PathBuf {
    crate::config::default_data_dir().join("tax.json")
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tracker() -> TaxTracker {
        let mut t = TaxTracker::new(CostBasisMethod::Fifo);
        t.acquire(1000, 1.00, "faucet", "faucet_1");
        t.acquire(500, 2.00, "purchase", "buy_1");
        t.acquire(300, 3.00, "purchase", "buy_2");
        t
    }

    #[test]
    fn test_acquire() {
        let t = make_tracker();
        assert_eq!(t.total_holdings(), 1800);
        assert_eq!(t.lots.len(), 3);
    }

    #[test]
    fn test_total_cost_basis() {
        let t = make_tracker();
        // 1000*1 + 500*2 + 300*3 = 1000 + 1000 + 900 = 2900
        assert_eq!(t.total_cost_basis(), 2900.0);
    }

    #[test]
    fn test_dispose_fifo() {
        let mut t = make_tracker();
        let d = t.dispose(1200, 5.00, "sell", "sell_1").unwrap();
        // FIFO: takes 1000 @ 1.00 + 200 @ 2.00 = 1000 + 400 = 1400 cost basis
        assert_eq!(d.amount, 1200);
        assert_eq!(d.cost_basis, 1400.0);
        assert_eq!(d.total_proceeds, 6000.0); // 1200 * 5.00
        assert_eq!(d.gain_loss, 4600.0); // 6000 - 1400
        // Remaining: 300 @ 2.00 + 300 @ 3.00 = 600 tokens
        assert_eq!(t.total_holdings(), 600);
    }

    #[test]
    fn test_dispose_lifo() {
        let mut t = TaxTracker::new(CostBasisMethod::Lifo);
        t.acquire(1000, 1.00, "faucet", "f1");
        t.acquire(500, 2.00, "buy", "b1");
        t.acquire(300, 3.00, "buy", "b2");
        let d = t.dispose(500, 5.00, "sell", "s1").unwrap();
        // LIFO: takes 300 @ 3.00 + 200 @ 2.00 = 900 + 400 = 1300 cost basis
        assert_eq!(d.cost_basis, 1300.0);
        assert_eq!(d.gain_loss, 2500.0 - 1300.0); // 1200
    }

    #[test]
    fn test_dispose_hifo() {
        let mut t = TaxTracker::new(CostBasisMethod::Hifo);
        t.acquire(1000, 1.00, "faucet", "f1");
        t.acquire(500, 2.00, "buy", "b1");
        t.acquire(300, 3.00, "buy", "b2");
        let d = t.dispose(500, 5.00, "sell", "s1").unwrap();
        // HIFO: takes 300 @ 3.00 + 200 @ 2.00 = 900 + 400 = 1300 cost basis
        assert_eq!(d.cost_basis, 1300.0);
    }

    #[test]
    fn test_dispose_insufficient() {
        let mut t = make_tracker();
        let err = t.dispose(5000, 1.0, "sell", "bad");
        assert!(err.is_err());
    }

    #[test]
    fn test_dispose_no_lots() {
        let mut t = TaxTracker::new(CostBasisMethod::Fifo);
        let err = t.dispose(100, 1.0, "sell", "bad");
        assert!(err.is_err());
    }

    #[test]
    fn test_multiple_disposals() {
        let mut t = make_tracker();
        t.dispose(500, 2.0, "sell", "s1").unwrap();
        t.dispose(500, 3.0, "sell", "s2").unwrap();
        assert_eq!(t.disposals.len(), 2);
        assert_eq!(t.total_holdings(), 800);
    }

    #[test]
    fn test_energy_cost_tracking() {
        let mut t = make_tracker();
        t.record_energy_cost(100.0, "Refresh obj_42", "ref_1");
        t.record_energy_cost(200.0, "Refresh obj_43", "ref_2");
        assert_eq!(t.energy_costs.len(), 2);
    }

    #[test]
    fn test_gas_cost_tracking() {
        let mut t = make_tracker();
        t.record_gas_cost(21.0, "tx_1");
        assert_eq!(t.gas_costs.len(), 1);
    }

    #[test]
    fn test_disposals_csv() {
        let mut t = make_tracker();
        t.dispose(100, 5.0, "sell", "s1").unwrap();
        let csv = t.disposals_csv();
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 2); // header + 1 disposal
        assert!(lines[0].starts_with("date,"));
    }

    #[test]
    fn test_lots_csv() {
        let t = make_tracker();
        let csv = t.lots_csv();
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 4); // header + 3 lots
    }

    #[test]
    fn test_lot_total_cost() {
        let lot = Lot {
            acquired_at: "2025-01-01T00:00:00Z".into(),
            amount: 100,
            original_amount: 100,
            cost_per_unit: 2.5,
            source: "buy".into(),
            reference: "ref".into(),
        };
        assert_eq!(lot.total_cost(), 250.0);
    }

    #[test]
    fn test_cost_basis_method_from_str() {
        assert_eq!(CostBasisMethod::from_str("fifo"), Some(CostBasisMethod::Fifo));
        assert_eq!(CostBasisMethod::from_str("LIFO"), Some(CostBasisMethod::Lifo));
        assert_eq!(CostBasisMethod::from_str("hifo"), Some(CostBasisMethod::Hifo));
        assert_eq!(CostBasisMethod::from_str("average"), None);
    }

    #[test]
    fn test_set_method() {
        let mut t = make_tracker();
        t.set_method(CostBasisMethod::Hifo);
        assert_eq!(t.method, CostBasisMethod::Hifo);
    }

    #[test]
    fn test_acquire_at_custom_timestamp() {
        let mut t = TaxTracker::new(CostBasisMethod::Fifo);
        t.acquire_at("2024-01-15T00:00:00Z", 1000, 0.50, "early_buy", "old_tx");
        assert_eq!(t.lots[0].acquired_at, "2024-01-15T00:00:00Z");
    }

    #[test]
    fn test_annual_summary() {
        let mut t = TaxTracker::new(CostBasisMethod::Fifo);
        let year = chrono::Utc::now().format("%Y").to_string().parse::<u32>().unwrap();
        t.acquire(1000, 1.0, "faucet", "f1");
        t.dispose(500, 2.0, "sell", "s1").unwrap();
        t.record_energy_cost(50.0, "refresh", "r1");
        t.record_gas_cost(10.0, "tx1");

        let summary = t.annual_summary(year);
        assert_eq!(summary.year, year);
        assert_eq!(summary.disposal_count, 1);
        assert_eq!(summary.total_disposals, 500);
        assert_eq!(summary.total_proceeds, 1000.0);
        assert_eq!(summary.total_cost_basis, 500.0);
        assert_eq!(summary.total_gain_loss, 500.0);
        assert_eq!(summary.energy_costs, 50.0);
        assert_eq!(summary.gas_costs, 10.0);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let dir = std::env::temp_dir().join("evap_tax_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("tax.json");

        let mut t = make_tracker();
        t.dispose(100, 5.0, "sell", "s1").unwrap();
        t.save(&path).unwrap();

        let loaded = TaxTracker::load(&path).unwrap();
        assert_eq!(loaded.total_holdings(), 1700);
        assert_eq!(loaded.disposals.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_default_trait() {
        let t = TaxTracker::default();
        assert_eq!(t.method, CostBasisMethod::Fifo);
        assert!(t.lots.is_empty());
    }
}
