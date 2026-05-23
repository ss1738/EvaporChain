//! Energy monitor — the killer feature.
//!
//! No other blockchain wallet has this. Predicts evaporation dates,
//! generates alerts at configurable thresholds, schedules auto-refresh,
//! and calculates resurrection costs.
//!
//! Uses the same exponential decay formula as the chain:
//!   E(t) = E₀ × 2^(-t / half_life)

use evaporchain_types::energy_at_epoch;
use serde::{Deserialize, Serialize};

use crate::assets::{OwnedNft, OwnedObject, Portfolio};

// ──────────────────────────── Alert Types ──────────────────────────────

/// Severity level for energy alerts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AlertSeverity {
    /// Energy below 50% — informational.
    Info,
    /// Energy below 25% — should refresh soon.
    Warning,
    /// Energy below 10% — refresh urgently or lose the asset.
    Critical,
}

/// Type of decaying asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetType {
    Object,
    Nft,
}

/// An energy alert for a decaying asset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnergyAlert {
    pub asset_type: AssetType,
    pub asset_id: String,
    pub asset_name: String,
    pub severity: AlertSeverity,
    /// Current energy as a percentage of max energy.
    pub current_energy_pct: f64,
    /// Current absolute energy.
    pub current_energy: u64,
    /// Max energy.
    pub max_energy: u64,
    /// Estimated epochs until energy reaches zero.
    pub epochs_until_zero: Option<u64>,
}

// ──────────────────────────── Decay Forecast ───────────────────────────

/// A milestone in the decay forecast.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecayMilestone {
    /// Target percentage (e.g., 75, 50, 25, 10, 1).
    pub target_pct: u64,
    /// Epoch at which energy drops to this percentage.
    pub at_epoch: u64,
    /// Absolute energy at this epoch.
    pub energy_at: u64,
}

/// Full decay forecast for an asset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecayForecast {
    pub asset_id: String,
    pub asset_name: String,
    pub asset_type: AssetType,
    pub current_energy: u64,
    pub max_energy: u64,
    pub half_life: u64,
    /// Milestones at 75%, 50%, 25%, 10%, 1%.
    pub milestones: Vec<DecayMilestone>,
    /// Epoch when energy reaches zero.
    pub zero_epoch: Option<u64>,
}

// ──────────────────────────── Auto-Refresh Schedule ────────────────────

/// A scheduled auto-refresh rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshSchedule {
    pub asset_type: AssetType,
    pub asset_id: String,
    /// Trigger refresh when energy drops below this percentage.
    pub trigger_at_pct: f64,
    /// How much energy to deposit.
    pub energy_amount: u64,
    /// Whether to execute automatically (vs. just alert).
    pub auto_execute: bool,
}

/// A refresh that should be executed now based on current state.
#[derive(Debug, Clone)]
pub struct TriggeredRefresh {
    pub schedule: RefreshSchedule,
    pub current_energy_pct: f64,
}

// ──────────────────────────── Energy Monitor ───────────────────────────

/// The energy monitor scans portfolios and generates alerts/forecasts.
pub struct EnergyMonitor {
    schedules: Vec<RefreshSchedule>,
}

impl EnergyMonitor {
    /// Create a new energy monitor.
    pub fn new() -> Self {
        Self {
            schedules: Vec::new(),
        }
    }

    /// Add an auto-refresh schedule.
    pub fn add_schedule(&mut self, schedule: RefreshSchedule) {
        self.schedules.push(schedule);
    }

    /// Remove all schedules for an asset.
    pub fn remove_schedules(&mut self, asset_id: &str) {
        self.schedules.retain(|s| s.asset_id != asset_id);
    }

    /// Get all configured schedules.
    pub fn schedules(&self) -> &[RefreshSchedule] {
        &self.schedules
    }

    /// Scan a portfolio and generate alerts for all decaying assets.
    pub fn scan_portfolio(&self, portfolio: &Portfolio) -> Vec<EnergyAlert> {
        let mut alerts = Vec::new();

        for obj in &portfolio.objects {
            if let Some(alert) = check_object_alert(obj) {
                alerts.push(alert);
            }
        }

        for nft in &portfolio.nfts {
            if let Some(alert) = check_nft_alert(nft) {
                alerts.push(alert);
            }
        }

        // Sort by severity (Critical first)
        alerts.sort_by_key(|a| std::cmp::Reverse(a.severity));
        alerts
    }

    /// Generate a detailed decay forecast for a specific asset.
    pub fn forecast_object(&self, obj: &OwnedObject) -> DecayForecast {
        let milestones = compute_milestones(obj.current_energy, obj.max_energy, obj.half_life);
        DecayForecast {
            asset_id: obj.id.clone(),
            asset_name: obj.name.clone(),
            asset_type: AssetType::Object,
            current_energy: obj.current_energy,
            max_energy: obj.max_energy,
            half_life: obj.half_life,
            milestones,
            zero_epoch: obj.epochs_until_zero,
        }
    }

    /// Generate a detailed decay forecast for an NFT.
    pub fn forecast_nft(&self, nft: &OwnedNft) -> DecayForecast {
        let milestones = compute_milestones(nft.current_energy, nft.max_energy, nft.half_life);
        let zero_epoch = crate::assets::epochs_until_zero(nft.current_energy, nft.half_life);
        DecayForecast {
            asset_id: nft.id.to_string(),
            asset_name: nft.name.clone(),
            asset_type: AssetType::Nft,
            current_energy: nft.current_energy,
            max_energy: nft.max_energy,
            half_life: nft.half_life,
            milestones,
            zero_epoch,
        }
    }

    /// Check which scheduled refreshes should be triggered based on current portfolio state.
    pub fn check_schedules(&self, portfolio: &Portfolio) -> Vec<TriggeredRefresh> {
        let mut triggered = Vec::new();

        for schedule in &self.schedules {
            let pct = match schedule.asset_type {
                AssetType::Object => portfolio
                    .objects
                    .iter()
                    .find(|o| o.id == schedule.asset_id)
                    .map(|o| {
                        if o.max_energy == 0 {
                            0.0
                        } else {
                            o.current_energy as f64 / o.max_energy as f64 * 100.0
                        }
                    }),
                AssetType::Nft => portfolio
                    .nfts
                    .iter()
                    .find(|n| n.id.to_string() == schedule.asset_id)
                    .map(|n| {
                        if n.max_energy == 0 {
                            0.0
                        } else {
                            n.current_energy as f64 / n.max_energy as f64 * 100.0
                        }
                    }),
            };

            if let Some(current_pct) = pct {
                if current_pct <= schedule.trigger_at_pct {
                    triggered.push(TriggeredRefresh {
                        schedule: schedule.clone(),
                        current_energy_pct: current_pct,
                    });
                }
            }
        }

        triggered
    }
}

impl Default for EnergyMonitor {
    fn default() -> Self {
        Self::new()
    }
}

// ──────────────────────────── Cost Calculators ─────────────────────────

/// Calculate how much energy deposit is needed to extend an asset's life
/// by a target number of epochs from the current state.
///
/// This answers: "If I deposit X energy now, the object will last at least
/// `target_epochs` more epochs before reaching zero."
pub fn energy_to_extend(current_energy: u64, half_life: u64, target_epochs: u64) -> u64 {
    if half_life == 0 {
        return 0;
    }
    // We need: energy_at_epoch(current + deposit, half_life, target_epochs) >= 1
    // So: (current + deposit) * 2^(-target_epochs/half_life) >= 1
    // deposit >= 2^(target_epochs/half_life) - current
    //
    // Use binary search for integer precision
    let mut lo: u64 = 0;
    let mut hi: u64 = u64::MAX / 2;

    // Check if current energy already lasts long enough
    if energy_at_epoch(current_energy, half_life, target_epochs) >= 1 {
        return 0;
    }

    while lo + 1 < hi {
        let mid = lo + (hi - lo) / 2;
        let total = current_energy.saturating_add(mid);
        if energy_at_epoch(total, half_life, target_epochs) >= 1 {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

/// Estimate the energy cost to resurrect a ghost object.
/// Resurrection requires enough energy to survive at least the grace period.
pub fn resurrection_cost(half_life: u64, grace_period: u64) -> u64 {
    if half_life == 0 {
        return 0;
    }
    // Need energy such that after grace_period epochs, energy > 0
    // Minimum: energy_at_epoch(deposit, half_life, grace_period) >= 1
    energy_to_extend(0, half_life, grace_period)
}

// ──────────────────────────── Internal ─────────────────────────────────

fn energy_pct(current: u64, max: u64) -> f64 {
    if max == 0 {
        return 0.0;
    }
    current as f64 / max as f64 * 100.0
}

fn severity_from_pct(pct: f64) -> Option<AlertSeverity> {
    if pct < 10.0 {
        Some(AlertSeverity::Critical)
    } else if pct < 25.0 {
        Some(AlertSeverity::Warning)
    } else if pct < 50.0 {
        Some(AlertSeverity::Info)
    } else {
        None
    }
}

fn check_object_alert(obj: &OwnedObject) -> Option<EnergyAlert> {
    if obj.state == "Ghost" || obj.max_energy == 0 {
        return None;
    }
    let pct = energy_pct(obj.current_energy, obj.max_energy);
    severity_from_pct(pct).map(|severity| EnergyAlert {
        asset_type: AssetType::Object,
        asset_id: obj.id.clone(),
        asset_name: obj.name.clone(),
        severity,
        current_energy_pct: (pct * 10.0).round() / 10.0,
        current_energy: obj.current_energy,
        max_energy: obj.max_energy,
        epochs_until_zero: obj.epochs_until_zero,
    })
}

fn check_nft_alert(nft: &OwnedNft) -> Option<EnergyAlert> {
    if nft.state == "Ghost" || nft.max_energy == 0 {
        return None;
    }
    let pct = energy_pct(nft.current_energy, nft.max_energy);
    let zero_epoch = crate::assets::epochs_until_zero(nft.current_energy, nft.half_life);
    severity_from_pct(pct).map(|severity| EnergyAlert {
        asset_type: AssetType::Nft,
        asset_id: nft.id.to_string(),
        asset_name: nft.name.clone(),
        severity,
        current_energy_pct: (pct * 10.0).round() / 10.0,
        current_energy: nft.current_energy,
        max_energy: nft.max_energy,
        epochs_until_zero: zero_epoch,
    })
}

fn compute_milestones(current_energy: u64, max_energy: u64, half_life: u64) -> Vec<DecayMilestone> {
    if half_life == 0 || current_energy == 0 {
        return vec![];
    }

    let targets = [75u64, 50, 25, 10, 1];
    let mut milestones = Vec::new();

    for &pct in &targets {
        let threshold = max_energy * pct / 100;
        if current_energy <= threshold {
            continue; // already past this milestone
        }
        if let Some(epochs) =
            crate::assets::epochs_until_threshold(current_energy, half_life, threshold)
        {
            milestones.push(DecayMilestone {
                target_pct: pct,
                at_epoch: epochs,
                energy_at: energy_at_epoch(current_energy, half_life, epochs),
            });
        }
    }

    milestones
}

// ──────────────────────────── Tests ────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{OwnedNft, OwnedObject};

    fn make_object(current_energy: u64, max_energy: u64, half_life: u64) -> OwnedObject {
        OwnedObject {
            id: "0xobj".to_string(),
            name: "test-obj".to_string(),
            energy: max_energy,
            max_energy,
            current_energy,
            half_life,
            state: "Active".to_string(),
            created_epoch: 0,
            last_refreshed: 0,
            decay_percentage: if max_energy > 0 {
                (max_energy - current_energy) as f64 / max_energy as f64 * 100.0
            } else {
                0.0
            },
            epochs_until_zero: crate::assets::epochs_until_zero(current_energy, half_life),
        }
    }

    fn make_nft(current_energy: u64, max_energy: u64, half_life: u64) -> OwnedNft {
        OwnedNft {
            id: 1,
            name: "test-nft".to_string(),
            collection: "art".to_string(),
            current_energy,
            max_energy,
            half_life,
            state: "Active".to_string(),
            decay_percentage: if max_energy > 0 {
                (max_energy - current_energy) as f64 / max_energy as f64 * 100.0
            } else {
                0.0
            },
            epochs_remaining: 0,
            grace_epoch: None,
            evaporated_epoch: None,
        }
    }

    #[test]
    fn test_alert_critical() {
        let obj = make_object(50, 1000, 100); // 5%
        let alert = check_object_alert(&obj).unwrap();
        assert_eq!(alert.severity, AlertSeverity::Critical);
    }

    #[test]
    fn test_alert_warning() {
        let obj = make_object(200, 1000, 100); // 20%
        let alert = check_object_alert(&obj).unwrap();
        assert_eq!(alert.severity, AlertSeverity::Warning);
    }

    #[test]
    fn test_alert_info() {
        let obj = make_object(400, 1000, 100); // 40%
        let alert = check_object_alert(&obj).unwrap();
        assert_eq!(alert.severity, AlertSeverity::Info);
    }

    #[test]
    fn test_no_alert_above_50() {
        let obj = make_object(600, 1000, 100); // 60%
        assert!(check_object_alert(&obj).is_none());
    }

    #[test]
    fn test_no_alert_for_ghosts() {
        let mut obj = make_object(50, 1000, 100);
        obj.state = "Ghost".to_string();
        assert!(check_object_alert(&obj).is_none());
    }

    #[test]
    fn test_nft_alert() {
        let nft = make_nft(80, 1000, 100); // 8%
        let alert = check_nft_alert(&nft).unwrap();
        assert_eq!(alert.severity, AlertSeverity::Critical);
        assert_eq!(alert.asset_type, AssetType::Nft);
    }

    #[test]
    fn test_scan_portfolio_sorted_by_severity() {
        let monitor = EnergyMonitor::new();
        let portfolio = Portfolio {
            address: "0xaa".to_string(),
            balance: 10000,
            nonce: 0,
            objects: vec![
                make_object(400, 1000, 100), // Info (40%)
                make_object(50, 1000, 100),  // Critical (5%)
            ],
            nfts: vec![
                make_nft(200, 1000, 100), // Warning (20%)
            ],
            tokens: vec![],
            total_energy_at_risk: 650,
        };

        let alerts = monitor.scan_portfolio(&portfolio);
        assert_eq!(alerts.len(), 3);
        assert_eq!(alerts[0].severity, AlertSeverity::Critical);
        assert_eq!(alerts[1].severity, AlertSeverity::Warning);
        assert_eq!(alerts[2].severity, AlertSeverity::Info);
    }

    #[test]
    fn test_forecast_milestones() {
        let monitor = EnergyMonitor::new();
        let obj = make_object(1000, 1000, 100);
        let forecast = monitor.forecast_object(&obj);

        assert!(!forecast.milestones.is_empty());
        // Should have milestones for 75%, 50%, 25%, 10%, 1%
        assert_eq!(forecast.milestones.len(), 5);
        // First milestone should be 75%
        assert_eq!(forecast.milestones[0].target_pct, 75);
        // 50% milestone should be at ~100 epochs (1 half-life)
        assert_eq!(forecast.milestones[1].target_pct, 50);
        assert_eq!(forecast.milestones[1].at_epoch, 100);
    }

    #[test]
    fn test_forecast_already_decayed() {
        let monitor = EnergyMonitor::new();
        let obj = make_object(200, 1000, 100); // Already at 20%, past 75% and 50% milestones
        let forecast = monitor.forecast_object(&obj);

        // Should only have milestones for 10% and 1% (200 > 100 so 10% is still ahead)
        for m in &forecast.milestones {
            assert!(m.target_pct < 20); // only milestones below current %
        }
    }

    #[test]
    fn test_energy_to_extend() {
        // 500 energy, half_life 100. Want to last 200 more epochs.
        // After 200 epochs: 500 * 2^(-2) = 125. Still > 0, so deposit = 0
        assert_eq!(energy_to_extend(500, 100, 200), 0);

        // 500 energy, half_life 100. Want to last 1000 epochs.
        // After 1000 epochs: 500 * 2^(-10) ≈ 0. Need deposit.
        let deposit = energy_to_extend(500, 100, 1000);
        assert!(deposit > 0);
        // Verify: (500 + deposit) after 1000 epochs >= 1
        assert!(energy_at_epoch(500 + deposit, 100, 1000) >= 1);
    }

    #[test]
    fn test_energy_to_extend_zero_half_life() {
        assert_eq!(energy_to_extend(500, 0, 100), 0);
    }

    #[test]
    fn test_resurrection_cost() {
        let cost = resurrection_cost(100, 5);
        assert!(cost > 0);
        // The deposit should survive the grace period
        assert!(energy_at_epoch(cost, 100, 5) >= 1);
    }

    #[test]
    fn test_resurrection_cost_zero_half_life() {
        assert_eq!(resurrection_cost(0, 5), 0);
    }

    #[test]
    fn test_schedule_trigger() {
        let mut monitor = EnergyMonitor::new();
        monitor.add_schedule(RefreshSchedule {
            asset_type: AssetType::Object,
            asset_id: "0xobj".to_string(),
            trigger_at_pct: 25.0,
            energy_amount: 500,
            auto_execute: true,
        });

        let portfolio = Portfolio {
            address: "0xaa".to_string(),
            balance: 10000,
            nonce: 0,
            objects: vec![make_object(200, 1000, 100)], // 20% — below 25% trigger
            nfts: vec![],
            tokens: vec![],
            total_energy_at_risk: 200,
        };

        let triggered = monitor.check_schedules(&portfolio);
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered[0].schedule.energy_amount, 500);
        assert!(triggered[0].current_energy_pct < 25.0);
    }

    #[test]
    fn test_schedule_not_triggered_above_threshold() {
        let mut monitor = EnergyMonitor::new();
        monitor.add_schedule(RefreshSchedule {
            asset_type: AssetType::Object,
            asset_id: "0xobj".to_string(),
            trigger_at_pct: 25.0,
            energy_amount: 500,
            auto_execute: true,
        });

        let portfolio = Portfolio {
            address: "0xaa".to_string(),
            balance: 10000,
            nonce: 0,
            objects: vec![make_object(800, 1000, 100)], // 80% — above 25% trigger
            nfts: vec![],
            tokens: vec![],
            total_energy_at_risk: 800,
        };

        let triggered = monitor.check_schedules(&portfolio);
        assert!(triggered.is_empty());
    }

    #[test]
    fn test_remove_schedules() {
        let mut monitor = EnergyMonitor::new();
        monitor.add_schedule(RefreshSchedule {
            asset_type: AssetType::Object,
            asset_id: "0xobj1".to_string(),
            trigger_at_pct: 25.0,
            energy_amount: 500,
            auto_execute: true,
        });
        monitor.add_schedule(RefreshSchedule {
            asset_type: AssetType::Object,
            asset_id: "0xobj2".to_string(),
            trigger_at_pct: 10.0,
            energy_amount: 1000,
            auto_execute: false,
        });

        assert_eq!(monitor.schedules().len(), 2);
        monitor.remove_schedules("0xobj1");
        assert_eq!(monitor.schedules().len(), 1);
        assert_eq!(monitor.schedules()[0].asset_id, "0xobj2");
    }

    // ─── Additional coverage tests ─────────────────────────────────────────

    #[test]
    fn test_forecast_nft_covers_lines_169_182() {
        let monitor = EnergyMonitor::new();
        let nft = make_nft(800, 1000, 100);
        let forecast = monitor.forecast_nft(&nft);
        assert_eq!(forecast.asset_type, AssetType::Nft);
        assert_eq!(forecast.current_energy, 800);
        assert!(!forecast.milestones.is_empty());
    }

    #[test]
    fn test_check_schedules_nft_covers_lines_201_211() {
        let mut monitor = EnergyMonitor::new();
        monitor.add_schedule(RefreshSchedule {
            asset_type: AssetType::Nft,
            asset_id: "1".to_string(), // matches nft.id.to_string()
            trigger_at_pct: 50.0,
            energy_amount: 500,
            auto_execute: true,
        });
        let portfolio = Portfolio {
            address: "0xaa".to_string(),
            balance: 10000,
            nonce: 0,
            objects: vec![],
            nfts: vec![make_nft(200, 1000, 100)], // 20% — below 50% trigger
            tokens: vec![],
            total_energy_at_risk: 200,
        };
        let triggered = monitor.check_schedules(&portfolio);
        assert_eq!(triggered.len(), 1);
        assert!(triggered[0].current_energy_pct < 50.0);
    }

    #[test]
    fn test_check_schedules_object_zero_max_energy_covers_line_196() {
        let mut monitor = EnergyMonitor::new();
        monitor.add_schedule(RefreshSchedule {
            asset_type: AssetType::Object,
            asset_id: "0xobj".to_string(),
            trigger_at_pct: 10.0,
            energy_amount: 100,
            auto_execute: false,
        });
        let mut obj = make_object(0, 0, 100); // max_energy == 0
        obj.max_energy = 0;
        let portfolio = Portfolio {
            address: "0xaa".to_string(),
            balance: 0,
            nonce: 0,
            objects: vec![obj],
            nfts: vec![],
            tokens: vec![],
            total_energy_at_risk: 0,
        };
        let triggered = monitor.check_schedules(&portfolio);
        // pct = 0.0 ≤ 10.0 → should trigger
        assert_eq!(triggered.len(), 1);
    }

    #[test]
    fn test_check_schedules_asset_not_in_portfolio_covers_line_221() {
        let mut monitor = EnergyMonitor::new();
        monitor.add_schedule(RefreshSchedule {
            asset_type: AssetType::Object,
            asset_id: "0xmissing".to_string(), // not in portfolio
            trigger_at_pct: 50.0,
            energy_amount: 100,
            auto_execute: false,
        });
        let portfolio = Portfolio {
            address: "0xaa".to_string(),
            balance: 0,
            nonce: 0,
            objects: vec![make_object(200, 1000, 100)], // different id
            nfts: vec![],
            tokens: vec![],
            total_energy_at_risk: 0,
        };
        let triggered = monitor.check_schedules(&portfolio);
        assert!(triggered.is_empty()); // asset not found → pct = None → skip
    }

    #[test]
    fn test_energy_monitor_default_covers_lines_229_231() {
        let monitor = EnergyMonitor::default();
        assert!(monitor.schedules().is_empty());
    }

    #[test]
    fn test_energy_pct_zero_max_covers_line_285() {
        let obj = make_object(0, 0, 100); // max_energy = 0 → energy_pct returns 0.0
        assert!(check_object_alert(&obj).is_none()); // max_energy==0 → early return None
    }

    #[test]
    fn test_check_nft_alert_ghost_covers_line_321() {
        let mut nft = make_nft(50, 1000, 100);
        nft.state = "Ghost".to_string();
        assert!(check_nft_alert(&nft).is_none());
    }

    #[test]
    fn test_check_nft_alert_zero_max_energy_covers_line_321() {
        let nft = make_nft(0, 0, 100); // max_energy == 0
        assert!(check_nft_alert(&nft).is_none());
    }

    #[test]
    fn test_compute_milestones_zero_half_life_covers_line_339() {
        let milestones = compute_milestones(1000, 1000, 0); // half_life == 0
        assert!(milestones.is_empty());
    }

    #[test]
    fn test_compute_milestones_zero_energy_covers_line_339() {
        let milestones = compute_milestones(0, 1000, 100); // current_energy == 0
        assert!(milestones.is_empty());
    }

    #[test]
    fn test_check_nft_alert_no_alert_above_50_covers_line_385() {
        let nft = make_nft(700, 1000, 100); // 70% — no alert
        assert!(check_nft_alert(&nft).is_none());
    }
}
