//! Portfolio Analytics — PnL tracking, energy spend analytics, cost basis, trends.
//!
//! Tracks all wallet activity over time and computes useful metrics:
//! balance history, energy costs, transfer volume, and period-over-period trends.

use std::path::Path;

use serde::{Deserialize, Serialize};

// ──────────────────────────── Types ──────────────────────────────────────

#[derive(Debug, Clone, thiserror::Error)]
pub enum AnalyticsError {
    #[error("no data for period")]
    NoData,
    #[error("invalid period: {0}")]
    InvalidPeriod(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
}

impl From<std::io::Error> for AnalyticsError {
    fn from(e: std::io::Error) -> Self {
        AnalyticsError::Io(e.to_string())
    }
}
impl From<serde_json::Error> for AnalyticsError {
    fn from(e: serde_json::Error) -> Self {
        AnalyticsError::Json(e.to_string())
    }
}

/// A single data point in the analytics timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataPoint {
    /// Timestamp (RFC3339).
    pub timestamp: String,
    /// Event type.
    pub event: EventType,
    /// Amount of EVAP involved.
    pub amount: u64,
    /// Running balance after this event.
    pub balance_after: u64,
    /// Optional metadata (tx hash, object ID, etc.).
    pub reference: String,
}

/// Types of analytics events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    TransferOut,
    TransferIn,
    EnergySpend,
    StakeDeposit,
    StakeWithdraw,
    StakeReward,
    FaucetReceive,
    GasFee,
    NftMint,
    TokenDeploy,
    BridgeOut,
    BridgeIn,
}

impl EventType {
    pub fn is_outflow(&self) -> bool {
        matches!(
            self,
            EventType::TransferOut
                | EventType::EnergySpend
                | EventType::StakeDeposit
                | EventType::GasFee
                | EventType::NftMint
                | EventType::TokenDeploy
                | EventType::BridgeOut
        )
    }

    pub fn is_inflow(&self) -> bool {
        matches!(
            self,
            EventType::TransferIn
                | EventType::StakeWithdraw
                | EventType::StakeReward
                | EventType::FaucetReceive
                | EventType::BridgeIn
        )
    }

    pub fn label(&self) -> &'static str {
        match self {
            EventType::TransferOut => "Transfer Out",
            EventType::TransferIn => "Transfer In",
            EventType::EnergySpend => "Energy Spend",
            EventType::StakeDeposit => "Stake",
            EventType::StakeWithdraw => "Unstake",
            EventType::StakeReward => "Reward",
            EventType::FaucetReceive => "Faucet",
            EventType::GasFee => "Gas Fee",
            EventType::NftMint => "NFT Mint",
            EventType::TokenDeploy => "Token Deploy",
            EventType::BridgeOut => "Bridge Out",
            EventType::BridgeIn => "Bridge In",
        }
    }
}

/// Time period for aggregation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Period {
    Day,
    Week,
    Month,
    AllTime,
}

impl Period {
    pub fn cutoff_timestamp(&self) -> String {
        let now = chrono::Utc::now();
        let cutoff = match self {
            Period::Day => now - chrono::Duration::days(1),
            Period::Week => now - chrono::Duration::weeks(1),
            Period::Month => now - chrono::Duration::days(30),
            Period::AllTime => chrono::DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        };
        cutoff.to_rfc3339()
    }

    pub fn label(&self) -> &'static str {
        match self {
            Period::Day => "24h",
            Period::Week => "7d",
            Period::Month => "30d",
            Period::AllTime => "all time",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Result<Self, AnalyticsError> {
        match s.to_lowercase().as_str() {
            "day" | "24h" | "1d" => Ok(Period::Day),
            "week" | "7d" => Ok(Period::Week),
            "month" | "30d" => Ok(Period::Month),
            "all" | "all_time" | "alltime" => Ok(Period::AllTime),
            _ => Err(AnalyticsError::InvalidPeriod(s.to_string())),
        }
    }
}

/// Aggregated summary for a period.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeriodSummary {
    pub period: Period,
    pub total_inflow: u64,
    pub total_outflow: u64,
    pub net_flow: i64,
    pub energy_spent: u64,
    pub gas_spent: u64,
    pub transfer_count: usize,
    pub largest_transfer: u64,
    pub events: usize,
}

/// Breakdown by event type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryBreakdown {
    pub category: String,
    pub count: usize,
    pub total_amount: u64,
    pub percentage: f64,
}

/// Trend comparison between two periods.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendReport {
    pub current: PeriodSummary,
    pub previous: PeriodSummary,
    pub outflow_change_pct: f64,
    pub inflow_change_pct: f64,
    pub energy_change_pct: f64,
    pub volume_change_pct: f64,
}

// ──────────────────────────── Tracker ────────────────────────────────────

/// The analytics engine — records events and computes metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyticsTracker {
    pub data_points: Vec<DataPoint>,
    /// Maximum data points to retain.
    pub capacity: usize,
}

impl AnalyticsTracker {
    pub fn new() -> Self {
        Self {
            data_points: Vec::new(),
            capacity: 10000,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data_points: Vec::new(),
            capacity,
        }
    }

    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, AnalyticsError> {
        let data = std::fs::read_to_string(path)?;
        let tracker: AnalyticsTracker = serde_json::from_str(&data)?;
        Ok(tracker)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), AnalyticsError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Record a new event.
    pub fn record(&mut self, event: EventType, amount: u64, balance_after: u64, reference: &str) {
        let dp = DataPoint {
            timestamp: chrono::Utc::now().to_rfc3339(),
            event,
            amount,
            balance_after,
            reference: reference.to_string(),
        };
        self.data_points.push(dp);

        // Evict oldest if over capacity
        if self.data_points.len() > self.capacity {
            self.data_points.remove(0);
        }
    }

    /// Record with a custom timestamp (for imports/backfill).
    pub fn record_at(
        &mut self,
        timestamp: &str,
        event: EventType,
        amount: u64,
        balance_after: u64,
        reference: &str,
    ) {
        let dp = DataPoint {
            timestamp: timestamp.to_string(),
            event,
            amount,
            balance_after,
            reference: reference.to_string(),
        };
        self.data_points.push(dp);
        if self.data_points.len() > self.capacity {
            self.data_points.remove(0);
        }
    }

    /// Get data points within a period.
    pub fn in_period(&self, period: Period) -> Vec<&DataPoint> {
        let cutoff = period.cutoff_timestamp();
        self.data_points
            .iter()
            .filter(|dp| dp.timestamp >= cutoff)
            .collect()
    }

    /// Compute period summary.
    pub fn summarize(&self, period: Period) -> PeriodSummary {
        let points = self.in_period(period);
        let mut total_inflow: u64 = 0;
        let mut total_outflow: u64 = 0;
        let mut energy_spent: u64 = 0;
        let mut gas_spent: u64 = 0;
        let mut transfer_count: usize = 0;
        let mut largest_transfer: u64 = 0;

        for dp in &points {
            if dp.event.is_inflow() {
                total_inflow += dp.amount;
            }
            if dp.event.is_outflow() {
                total_outflow += dp.amount;
            }
            if dp.event == EventType::EnergySpend {
                energy_spent += dp.amount;
            }
            if dp.event == EventType::GasFee {
                gas_spent += dp.amount;
            }
            if dp.event == EventType::TransferOut || dp.event == EventType::TransferIn {
                transfer_count += 1;
                if dp.amount > largest_transfer {
                    largest_transfer = dp.amount;
                }
            }
        }

        PeriodSummary {
            period,
            total_inflow,
            total_outflow,
            net_flow: total_inflow as i64 - total_outflow as i64,
            energy_spent,
            gas_spent,
            transfer_count,
            largest_transfer,
            events: points.len(),
        }
    }

    /// Breakdown by category for a period.
    pub fn breakdown(&self, period: Period) -> Vec<CategoryBreakdown> {
        let points = self.in_period(period);
        let total: u64 = points.iter().map(|dp| dp.amount).sum();
        if total == 0 {
            return vec![];
        }

        let mut counts: std::collections::HashMap<String, (usize, u64)> =
            std::collections::HashMap::new();
        for dp in &points {
            let entry = counts.entry(dp.event.label().to_string()).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += dp.amount;
        }

        let mut result: Vec<CategoryBreakdown> = counts
            .into_iter()
            .map(|(cat, (count, amt))| CategoryBreakdown {
                category: cat,
                count,
                total_amount: amt,
                percentage: (amt as f64 / total as f64) * 100.0,
            })
            .collect();
        result.sort_by_key(|a| std::cmp::Reverse(a.total_amount));
        result
    }

    /// Compute trend: compare current period to previous equivalent period.
    pub fn trend(&self, period: Period) -> TrendReport {
        let current = self.summarize(period);

        // Get previous period data by filtering for earlier window
        let now = chrono::Utc::now();
        let period_duration = match period {
            Period::Day => chrono::Duration::days(1),
            Period::Week => chrono::Duration::weeks(1),
            Period::Month => chrono::Duration::days(30),
            Period::AllTime => chrono::Duration::days(365 * 10),
        };
        let prev_end = now - period_duration;
        let prev_start = prev_end - period_duration;
        let prev_start_str = prev_start.to_rfc3339();
        let prev_end_str = prev_end.to_rfc3339();

        let prev_points: Vec<&DataPoint> = self
            .data_points
            .iter()
            .filter(|dp| dp.timestamp >= prev_start_str && dp.timestamp < prev_end_str)
            .collect();

        let mut prev_inflow: u64 = 0;
        let mut prev_outflow: u64 = 0;
        let mut prev_energy: u64 = 0;
        let mut prev_gas: u64 = 0;
        let mut prev_transfers: usize = 0;
        let mut prev_largest: u64 = 0;

        for dp in &prev_points {
            if dp.event.is_inflow() {
                prev_inflow += dp.amount;
            }
            if dp.event.is_outflow() {
                prev_outflow += dp.amount;
            }
            if dp.event == EventType::EnergySpend {
                prev_energy += dp.amount;
            }
            if dp.event == EventType::GasFee {
                prev_gas += dp.amount;
            }
            if dp.event == EventType::TransferOut || dp.event == EventType::TransferIn {
                prev_transfers += 1;
                if dp.amount > prev_largest {
                    prev_largest = dp.amount;
                }
            }
        }

        let previous = PeriodSummary {
            period,
            total_inflow: prev_inflow,
            total_outflow: prev_outflow,
            net_flow: prev_inflow as i64 - prev_outflow as i64,
            energy_spent: prev_energy,
            gas_spent: prev_gas,
            transfer_count: prev_transfers,
            largest_transfer: prev_largest,
            events: prev_points.len(),
        };

        let pct_change = |curr: u64, prev: u64| -> f64 {
            if prev == 0 {
                if curr > 0 {
                    100.0
                } else {
                    0.0
                }
            } else {
                ((curr as f64 - prev as f64) / prev as f64) * 100.0
            }
        };

        TrendReport {
            outflow_change_pct: pct_change(current.total_outflow, previous.total_outflow),
            inflow_change_pct: pct_change(current.total_inflow, previous.total_inflow),
            energy_change_pct: pct_change(current.energy_spent, previous.energy_spent),
            volume_change_pct: pct_change(
                current.total_inflow + current.total_outflow,
                previous.total_inflow + previous.total_outflow,
            ),
            current,
            previous,
        }
    }

    /// Current balance (from latest data point).
    pub fn latest_balance(&self) -> Option<u64> {
        self.data_points.last().map(|dp| dp.balance_after)
    }

    /// Total data points.
    pub fn len(&self) -> usize {
        self.data_points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data_points.is_empty()
    }

    /// Clear all data.
    pub fn clear(&mut self) {
        self.data_points.clear();
    }
}

impl Default for AnalyticsTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Default path.
pub fn default_analytics_path() -> std::path::PathBuf {
    crate::config::default_data_dir().join("analytics.json")
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tracker() -> AnalyticsTracker {
        let mut t = AnalyticsTracker::new();
        t.record(EventType::FaucetReceive, 100_000, 100_000, "faucet_1");
        t.record(EventType::TransferOut, 5_000, 95_000, "tx_001");
        t.record(EventType::EnergySpend, 1_000, 94_000, "obj_42");
        t.record(EventType::GasFee, 21, 93_979, "tx_001_gas");
        t.record(EventType::TransferIn, 10_000, 103_979, "tx_002");
        t.record(EventType::StakeDeposit, 20_000, 83_979, "pool_1");
        t.record(EventType::StakeReward, 500, 84_479, "pool_1_reward");
        t
    }

    #[test]
    fn test_record() {
        let t = make_tracker();
        assert_eq!(t.len(), 7);
        assert!(!t.is_empty());
    }

    #[test]
    fn test_latest_balance() {
        let t = make_tracker();
        assert_eq!(t.latest_balance(), Some(84_479));
    }

    #[test]
    fn test_summarize_all_time() {
        let t = make_tracker();
        let s = t.summarize(Period::AllTime);
        assert_eq!(s.total_inflow, 110_500); // 100000 + 10000 + 500
        assert_eq!(s.total_outflow, 26_021); // 5000 + 1000 + 21 + 20000
        assert_eq!(s.net_flow, 84_479);
        assert_eq!(s.energy_spent, 1_000);
        assert_eq!(s.gas_spent, 21);
        assert_eq!(s.transfer_count, 2); // 1 out + 1 in
        assert_eq!(s.largest_transfer, 10_000);
        assert_eq!(s.events, 7);
    }

    #[test]
    fn test_summarize_day() {
        let t = make_tracker();
        let s = t.summarize(Period::Day);
        // All events were recorded "now", so they all fall within 24h
        assert_eq!(s.events, 7);
    }

    #[test]
    fn test_breakdown() {
        let t = make_tracker();
        let bd = t.breakdown(Period::AllTime);
        assert!(!bd.is_empty());
        // Highest amount category should be first
        assert!(bd[0].total_amount >= bd.last().unwrap().total_amount);
        // Percentages should sum to ~100
        let total_pct: f64 = bd.iter().map(|b| b.percentage).sum();
        assert!((total_pct - 100.0).abs() < 0.1);
    }

    #[test]
    fn test_trend_no_previous() {
        let t = make_tracker();
        let trend = t.trend(Period::Day);
        // All data is in current period, previous should be empty
        assert_eq!(trend.previous.events, 0);
        assert_eq!(trend.current.events, 7);
        // With no previous data, changes should be 100% (since current > 0)
        assert!(trend.outflow_change_pct > 0.0);
    }

    #[test]
    fn test_event_type_flow() {
        assert!(EventType::TransferOut.is_outflow());
        assert!(!EventType::TransferOut.is_inflow());
        assert!(EventType::TransferIn.is_inflow());
        assert!(!EventType::TransferIn.is_outflow());
        assert!(EventType::EnergySpend.is_outflow());
        assert!(EventType::StakeReward.is_inflow());
    }

    #[test]
    fn test_period_from_str() {
        assert_eq!(Period::from_str("day").unwrap(), Period::Day);
        assert_eq!(Period::from_str("24h").unwrap(), Period::Day);
        assert_eq!(Period::from_str("week").unwrap(), Period::Week);
        assert_eq!(Period::from_str("7d").unwrap(), Period::Week);
        assert_eq!(Period::from_str("month").unwrap(), Period::Month);
        assert_eq!(Period::from_str("all").unwrap(), Period::AllTime);
        assert!(Period::from_str("invalid").is_err());
    }

    #[test]
    fn test_period_label() {
        assert_eq!(Period::Day.label(), "24h");
        assert_eq!(Period::Week.label(), "7d");
        assert_eq!(Period::Month.label(), "30d");
        assert_eq!(Period::AllTime.label(), "all time");
    }

    #[test]
    fn test_capacity_eviction() {
        let mut t = AnalyticsTracker::with_capacity(5);
        for i in 0..10 {
            t.record(
                EventType::TransferOut,
                100,
                1000 - i * 100,
                &format!("tx_{}", i),
            );
        }
        assert_eq!(t.len(), 5);
        // Oldest should have been evicted
        assert_eq!(t.data_points[0].reference, "tx_5");
    }

    #[test]
    fn test_clear() {
        let mut t = make_tracker();
        t.clear();
        assert!(t.is_empty());
        assert_eq!(t.latest_balance(), None);
    }

    #[test]
    fn test_empty_summary() {
        let t = AnalyticsTracker::new();
        let s = t.summarize(Period::AllTime);
        assert_eq!(s.events, 0);
        assert_eq!(s.net_flow, 0);
    }

    #[test]
    fn test_empty_breakdown() {
        let t = AnalyticsTracker::new();
        let bd = t.breakdown(Period::AllTime);
        assert!(bd.is_empty());
    }

    #[test]
    fn test_record_at_custom_timestamp() {
        let mut t = AnalyticsTracker::new();
        t.record_at(
            "2025-01-01T00:00:00Z",
            EventType::TransferIn,
            50000,
            50000,
            "old_tx",
        );
        t.record(EventType::TransferOut, 1000, 49000, "new_tx");
        assert_eq!(t.len(), 2);
        // Only the recent one should appear in Day period
        let day_points = t.in_period(Period::Day);
        assert_eq!(day_points.len(), 1);
        assert_eq!(day_points[0].reference, "new_tx");
    }

    #[test]
    fn test_persistence_roundtrip() {
        let dir = std::env::temp_dir().join("evap_analytics_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("analytics.json");

        let t = make_tracker();
        t.save(&path).unwrap();

        let loaded = AnalyticsTracker::load(&path).unwrap();
        assert_eq!(loaded.len(), 7);
        assert_eq!(loaded.latest_balance(), Some(84_479));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_event_labels() {
        assert_eq!(EventType::TransferOut.label(), "Transfer Out");
        assert_eq!(EventType::EnergySpend.label(), "Energy Spend");
        assert_eq!(EventType::GasFee.label(), "Gas Fee");
        assert_eq!(EventType::BridgeIn.label(), "Bridge In");
    }

    #[test]
    fn test_net_flow_negative() {
        let mut t = AnalyticsTracker::new();
        t.record(EventType::FaucetReceive, 1000, 1000, "faucet");
        t.record(EventType::TransferOut, 5000, 0, "big_send");
        let s = t.summarize(Period::AllTime);
        assert_eq!(s.net_flow, -4000);
    }

    #[test]
    fn test_multiple_energy_spends() {
        let mut t = AnalyticsTracker::new();
        t.record(EventType::EnergySpend, 500, 9500, "obj_1");
        t.record(EventType::EnergySpend, 300, 9200, "obj_2");
        t.record(EventType::EnergySpend, 200, 9000, "obj_3");
        let s = t.summarize(Period::AllTime);
        assert_eq!(s.energy_spent, 1000);
    }

    #[test]
    fn test_default_trait() {
        let t = AnalyticsTracker::default();
        assert!(t.is_empty());
        assert_eq!(t.capacity, 10000);
    }

    // ─── Additional coverage tests ────────────────────────────────────────────

    #[test]
    fn test_from_io_error_covers_lines_25_27() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let ae = AnalyticsError::from(io_err);
        assert!(matches!(ae, AnalyticsError::Io(_)));
        assert!(ae.to_string().contains("not found"));
    }

    #[test]
    fn test_from_json_error_covers_lines_30_32() {
        let json_err: serde_json::Error =
            serde_json::from_str::<serde_json::Value>("{bad}").unwrap_err();
        let ae = AnalyticsError::from(json_err);
        assert!(matches!(ae, AnalyticsError::Json(_)));
    }

    #[test]
    fn test_event_labels_stake_withdraw_nft_mint_covers_lines_99_103() {
        assert_eq!(EventType::StakeWithdraw.label(), "Unstake");
        assert_eq!(EventType::NftMint.label(), "NFT Mint");
    }

    #[test]
    fn test_trend_with_previous_period_data_covers_lines_382_398() {
        let mut t = AnalyticsTracker::new();
        // 36h ago falls in the previous-period window for Period::Day (24h–48h back)
        let ts_36h = (chrono::Utc::now() - chrono::Duration::hours(36)).to_rfc3339();
        t.record_at(&ts_36h, EventType::TransferIn, 5_000, 5_000, "prev_in");
        t.record_at(&ts_36h, EventType::TransferOut, 1_000, 4_000, "prev_out");
        t.record_at(&ts_36h, EventType::EnergySpend, 200, 3_800, "prev_energy");
        t.record_at(&ts_36h, EventType::GasFee, 50, 3_750, "prev_gas");
        let trend = t.trend(Period::Day);
        assert!(trend.previous.total_inflow > 0);
        assert!(trend.previous.total_outflow > 0);
        assert!(trend.previous.energy_spent > 0);
        assert!(trend.previous.gas_spent > 0);
    }

    #[test]
    fn test_trend_prev_transfer_covers_lines_394_398() {
        let mut t = AnalyticsTracker::new();
        let ts_36h = (chrono::Utc::now() - chrono::Duration::hours(36)).to_rfc3339();
        t.record_at(&ts_36h, EventType::TransferIn, 10_000, 10_000, "prev_txin");
        let trend = t.trend(Period::Day);
        assert_eq!(trend.previous.transfer_count, 1);
        assert_eq!(trend.previous.largest_transfer, 10_000);
    }

    #[test]
    fn test_trend_empty_tracker_zero_pct_covers_line_419() {
        let t = AnalyticsTracker::new();
        let trend = t.trend(Period::Day);
        assert_eq!(trend.outflow_change_pct, 0.0);
        assert_eq!(trend.inflow_change_pct, 0.0);
        assert_eq!(trend.energy_change_pct, 0.0);
        assert_eq!(trend.volume_change_pct, 0.0);
    }

    #[test]
    fn test_event_labels_token_deploy_bridge_out_covers_lines_104_105() {
        assert_eq!(EventType::TokenDeploy.label(), "Token Deploy");
        assert_eq!(EventType::BridgeOut.label(), "Bridge Out");
    }

    #[test]
    fn test_period_cutoff_week_month_covers_lines_125_126() {
        let t = AnalyticsTracker::new();
        let _ = t.in_period(Period::Week);
        let _ = t.in_period(Period::Month);
    }

    #[test]
    fn test_record_at_capacity_eviction_covers_line_265() {
        let mut t = AnalyticsTracker::with_capacity(3);
        let ts = "2025-01-01T00:00:00Z";
        for i in 0u64..5 {
            t.record_at(ts, EventType::TransferOut, 100, 1000 - i, &format!("tx_{}", i));
        }
        assert_eq!(t.len(), 3);
        assert_eq!(t.data_points[0].reference, "tx_2");
    }

    #[test]
    fn test_trend_week_month_alltime_covers_lines_359_361() {
        let t = AnalyticsTracker::new();
        let _ = t.trend(Period::Week);
        let _ = t.trend(Period::Month);
        let _ = t.trend(Period::AllTime);
    }

    #[test]
    fn test_default_analytics_path_covers_lines_466_468() {
        let p = default_analytics_path();
        assert!(p.to_string_lossy().contains("analytics.json"));
    }
}
