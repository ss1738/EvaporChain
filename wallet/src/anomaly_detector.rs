use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum AnomalyDetectorError {
    #[error("rule not found: {0}")]
    RuleNotFound(String),
    #[error("duplicate rule: {0}")]
    DuplicateRule(String),
    #[error("profile not found: {0}")]
    ProfileNotFound(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Parse(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum AnomalyType {
    UnusualAmount,
    HighVelocity,
    NewRecipient,
    LargeGas,
    OffHoursActivity,
    RapidSequence,
    DustAttack,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum RuleStatus {
    #[default]
    Enabled,
    Disabled,
}

// ---------------------------------------------------------------------------
// Data structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxSample3 {
    pub tx_hash: String,
    pub from: String,
    pub to: String,
    pub amount: u64,
    pub gas: u64,
    pub timestamp: String,
    pub hour_of_day: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorProfile {
    pub address: String,
    pub avg_amount: f64,
    pub max_amount: u64,
    pub tx_count: u64,
    pub unique_recipients: usize,
    pub avg_gas: f64,
    pub common_hours: Vec<u8>,
    pub last_updated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectionRule {
    pub id: String,
    pub anomaly_type: AnomalyType,
    pub threshold: f64,
    pub status: RuleStatus,
    pub description: String,
    pub created_at: String,
    pub triggers: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnomalyAlert {
    pub id: String,
    pub rule_id: String,
    pub anomaly_type: AnomalyType,
    pub risk_level: RiskLevel,
    pub tx_hash: String,
    pub details: String,
    pub detected_at: String,
    pub acknowledged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectorStats {
    pub total_rules: usize,
    pub enabled_rules: usize,
    pub total_alerts: usize,
    pub unacknowledged: usize,
    pub total_samples: usize,
    pub profiles: usize,
    pub by_risk: HashMap<String, usize>,
}

// ---------------------------------------------------------------------------
// Main struct
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnomalyDetector {
    pub rules: HashMap<String, DetectionRule>,
    pub profiles: HashMap<String, BehaviorProfile>,
    pub alerts: Vec<AnomalyAlert>,
    pub samples: Vec<TxSample3>,
    pub max_samples: usize,
}

impl Default for AnomalyDetector {
    fn default() -> Self {
        Self {
            rules: HashMap::new(),
            profiles: HashMap::new(),
            alerts: Vec::new(),
            samples: Vec::new(),
            max_samples: 50_000,
        }
    }
}

impl AnomalyDetector {
    pub fn new() -> Self {
        Self::default()
    }

    // -- Rule management ----------------------------------------------------

    pub fn add_rule(&mut self, rule: DetectionRule) -> Result<(), AnomalyDetectorError> {
        if self.rules.contains_key(&rule.id) {
            return Err(AnomalyDetectorError::DuplicateRule(rule.id.clone()));
        }
        self.rules.insert(rule.id.clone(), rule);
        Ok(())
    }

    pub fn remove_rule(&mut self, id: &str) -> Result<DetectionRule, AnomalyDetectorError> {
        self.rules
            .remove(id)
            .ok_or_else(|| AnomalyDetectorError::RuleNotFound(id.to_string()))
    }

    pub fn enable_rule(&mut self, id: &str) -> Result<(), AnomalyDetectorError> {
        let rule = self
            .rules
            .get_mut(id)
            .ok_or_else(|| AnomalyDetectorError::RuleNotFound(id.to_string()))?;
        rule.status = RuleStatus::Enabled;
        Ok(())
    }

    pub fn disable_rule(&mut self, id: &str) -> Result<(), AnomalyDetectorError> {
        let rule = self
            .rules
            .get_mut(id)
            .ok_or_else(|| AnomalyDetectorError::RuleNotFound(id.to_string()))?;
        rule.status = RuleStatus::Disabled;
        Ok(())
    }

    // -- Samples & profiles -------------------------------------------------

    pub fn record_sample(&mut self, sample: TxSample3) {
        self.update_profile(&sample.from.clone(), &sample);
        self.samples.push(sample);
        if self.samples.len() > self.max_samples {
            let excess = self.samples.len() - self.max_samples;
            self.samples.drain(..excess);
        }
    }

    pub fn update_profile(&mut self, address: &str, sample: &TxSample3) {
        let profile = self
            .profiles
            .entry(address.to_string())
            .or_insert_with(|| BehaviorProfile {
                address: address.to_string(),
                avg_amount: 0.0,
                max_amount: 0,
                tx_count: 0,
                unique_recipients: 0,
                avg_gas: 0.0,
                common_hours: Vec::new(),
                last_updated: Utc::now().to_rfc3339(),
            });

        // Recalculate running average for amount
        let old_total = profile.avg_amount * profile.tx_count as f64;
        profile.tx_count += 1;
        profile.avg_amount = (old_total + sample.amount as f64) / profile.tx_count as f64;

        if sample.amount > profile.max_amount {
            profile.max_amount = sample.amount;
        }

        // Running average for gas
        let old_gas_total = profile.avg_gas * (profile.tx_count - 1) as f64;
        profile.avg_gas = (old_gas_total + sample.gas as f64) / profile.tx_count as f64;

        // Track unique recipients by counting distinct `to` in samples from this address
        if !self
            .samples
            .iter()
            .any(|s| s.from == address && s.to == sample.to)
        {
            profile.unique_recipients += 1;
        }

        // Track common hours
        if !profile.common_hours.contains(&sample.hour_of_day) {
            profile.common_hours.push(sample.hour_of_day);
        }

        profile.last_updated = Utc::now().to_rfc3339();
    }

    // -- Analysis -----------------------------------------------------------

    pub fn analyze_sample(&mut self, sample: &TxSample3) -> Vec<AnomalyAlert> {
        let mut alerts = Vec::new();
        let profile = self.profiles.get(&sample.from).cloned();

        let rule_ids: Vec<String> = self
            .rules
            .values()
            .filter(|r| r.status == RuleStatus::Enabled)
            .map(|r| r.id.clone())
            .collect();

        for rule_id in &rule_ids {
            let rule = self.rules.get(rule_id).unwrap().clone();
            let maybe_alert = match &rule.anomaly_type {
                AnomalyType::UnusualAmount => {
                    if let Some(ref p) = profile {
                        if p.tx_count > 0 && (sample.amount as f64) > p.avg_amount * rule.threshold
                        {
                            Some(self.make_alert(
                                &rule,
                                sample,
                                RiskLevel::High,
                                format!(
                                    "Amount {} exceeds avg {:.0} by {:.1}x",
                                    sample.amount,
                                    p.avg_amount,
                                    sample.amount as f64 / p.avg_amount
                                ),
                            ))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                AnomalyType::HighVelocity => {
                    let count_last_hour = self
                        .samples
                        .iter()
                        .filter(|s| s.from == sample.from && s.hour_of_day == sample.hour_of_day)
                        .count();
                    if count_last_hour as f64 > rule.threshold {
                        Some(self.make_alert(
                            &rule,
                            sample,
                            RiskLevel::Medium,
                            format!(
                                "Velocity {} txs in hour exceeds threshold {}",
                                count_last_hour, rule.threshold
                            ),
                        ))
                    } else {
                        None
                    }
                }
                AnomalyType::NewRecipient => {
                    if let Some(ref p) = profile {
                        if p.tx_count > 0 {
                            let known = self
                                .samples
                                .iter()
                                .any(|s| s.from == sample.from && s.to == sample.to);
                            if !known {
                                Some(self.make_alert(
                                    &rule,
                                    sample,
                                    RiskLevel::Low,
                                    format!(
                                        "New recipient {} for sender {}",
                                        sample.to, sample.from
                                    ),
                                ))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                AnomalyType::LargeGas => {
                    if let Some(ref p) = profile {
                        if p.tx_count > 0 && (sample.gas as f64) > p.avg_gas * rule.threshold {
                            Some(self.make_alert(
                                &rule,
                                sample,
                                RiskLevel::Medium,
                                format!(
                                    "Gas {} exceeds avg {:.0} by {:.1}x",
                                    sample.gas,
                                    p.avg_gas,
                                    sample.gas as f64 / p.avg_gas
                                ),
                            ))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                AnomalyType::OffHoursActivity => {
                    if let Some(ref p) = profile {
                        if p.tx_count > 0 && !p.common_hours.contains(&sample.hour_of_day) {
                            Some(self.make_alert(
                                &rule,
                                sample,
                                RiskLevel::Low,
                                format!("Activity at unusual hour {}", sample.hour_of_day),
                            ))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                AnomalyType::RapidSequence => {
                    // Simplified: count samples in same hour from same sender
                    let recent = self
                        .samples
                        .iter()
                        .rev()
                        .take(20)
                        .filter(|s| s.from == sample.from)
                        .count();
                    if recent as f64 > rule.threshold {
                        Some(self.make_alert(
                            &rule,
                            sample,
                            RiskLevel::High,
                            format!("Rapid sequence: {} recent txs", recent),
                        ))
                    } else {
                        None
                    }
                }
                AnomalyType::DustAttack => {
                    if sample.amount > 0 && (sample.amount as f64) < rule.threshold {
                        Some(self.make_alert(
                            &rule,
                            sample,
                            RiskLevel::Critical,
                            format!(
                                "Possible dust attack: amount {} below {}",
                                sample.amount, rule.threshold
                            ),
                        ))
                    } else {
                        None
                    }
                }
                AnomalyType::Custom(ref _name) => None,
            };

            if let Some(alert) = maybe_alert {
                // Increment trigger count on the rule
                if let Some(r) = self.rules.get_mut(rule_id) {
                    r.triggers += 1;
                }
                self.alerts.push(alert.clone());
                alerts.push(alert);
            }
        }

        alerts
    }

    fn make_alert(
        &self,
        rule: &DetectionRule,
        sample: &TxSample3,
        risk: RiskLevel,
        details: String,
    ) -> AnomalyAlert {
        AnomalyAlert {
            id: format!("alert-{}-{}", rule.id, self.alerts.len()),
            rule_id: rule.id.clone(),
            anomaly_type: rule.anomaly_type.clone(),
            risk_level: risk,
            tx_hash: sample.tx_hash.clone(),
            details,
            detected_at: Utc::now().to_rfc3339(),
            acknowledged: false,
        }
    }

    // -- Query helpers ------------------------------------------------------

    pub fn get_profile(&self, address: &str) -> Option<&BehaviorProfile> {
        self.profiles.get(address)
    }

    pub fn acknowledge_alert(&mut self, alert_id: &str) -> Result<(), AnomalyDetectorError> {
        for alert in &mut self.alerts {
            if alert.id == alert_id {
                alert.acknowledged = true;
                return Ok(());
            }
        }
        Err(AnomalyDetectorError::RuleNotFound(format!(
            "alert {} not found",
            alert_id
        )))
    }

    pub fn unacknowledged_alerts(&self) -> Vec<&AnomalyAlert> {
        self.alerts.iter().filter(|a| !a.acknowledged).collect()
    }

    pub fn alerts_by_risk(&self, risk: &RiskLevel) -> Vec<&AnomalyAlert> {
        self.alerts
            .iter()
            .filter(|a| &a.risk_level == risk)
            .collect()
    }

    pub fn alerts_for_address(&self, address: &str) -> Vec<&AnomalyAlert> {
        self.alerts
            .iter()
            .filter(|a| a.tx_hash.contains(address) || a.details.contains(address))
            .collect()
    }

    pub fn recent_alerts(&self, n: usize) -> Vec<&AnomalyAlert> {
        self.alerts.iter().rev().take(n).collect()
    }

    pub fn risk_score(&self, address: &str) -> f64 {
        let addr_alerts: Vec<&AnomalyAlert> = self
            .alerts
            .iter()
            .filter(|a| {
                !a.acknowledged && (a.tx_hash.contains(address) || a.details.contains(address))
            })
            .collect();

        let mut score: f64 = 0.0;
        for alert in addr_alerts {
            score += match alert.risk_level {
                RiskLevel::Critical => 40.0,
                RiskLevel::High => 20.0,
                RiskLevel::Medium => 10.0,
                RiskLevel::Low => 5.0,
            };
        }

        if score > 100.0 {
            100.0
        } else {
            score
        }
    }

    pub fn stats(&self) -> DetectorStats {
        let enabled_rules = self
            .rules
            .values()
            .filter(|r| r.status == RuleStatus::Enabled)
            .count();
        let unacknowledged = self.alerts.iter().filter(|a| !a.acknowledged).count();

        let mut by_risk: HashMap<String, usize> = HashMap::new();
        for alert in &self.alerts {
            let key = format!("{:?}", alert.risk_level);
            *by_risk.entry(key).or_insert(0) += 1;
        }

        DetectorStats {
            total_rules: self.rules.len(),
            enabled_rules,
            total_alerts: self.alerts.len(),
            unacknowledged,
            total_samples: self.samples.len(),
            profiles: self.profiles.len(),
            by_risk,
        }
    }

    // -- Persistence --------------------------------------------------------

    pub fn save(&self, path: &Path) -> Result<(), AnomalyDetectorError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, AnomalyDetectorError> {
        let data = std::fs::read_to_string(path)?;
        let detector: Self = serde_json::from_str(&data)?;
        Ok(detector)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rule(id: &str, anomaly_type: AnomalyType, threshold: f64) -> DetectionRule {
        DetectionRule {
            id: id.to_string(),
            anomaly_type,
            threshold,
            status: RuleStatus::Enabled,
            description: format!("Rule {}", id),
            created_at: Utc::now().to_rfc3339(),
            triggers: 0,
        }
    }

    fn make_sample(from: &str, to: &str, amount: u64, gas: u64, hour: u8) -> TxSample3 {
        TxSample3 {
            tx_hash: format!("0xhash_{}_{}_{}", from, to, amount),
            from: from.to_string(),
            to: to.to_string(),
            amount,
            gas,
            timestamp: Utc::now().to_rfc3339(),
            hour_of_day: hour,
        }
    }

    fn test_path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir();
        dir.join(format!(
            "anomaly_detector_test_{}_{}_{}.json",
            name,
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ))
    }

    #[test]
    fn test_add_rule() {
        let mut det = AnomalyDetector::new();
        let rule = make_rule("r1", AnomalyType::UnusualAmount, 3.0);
        assert!(det.add_rule(rule).is_ok());
        assert_eq!(det.rules.len(), 1);
    }

    #[test]
    fn test_duplicate_rule() {
        let mut det = AnomalyDetector::new();
        let r1 = make_rule("r1", AnomalyType::UnusualAmount, 3.0);
        let r2 = make_rule("r1", AnomalyType::HighVelocity, 5.0);
        det.add_rule(r1).unwrap();
        assert!(det.add_rule(r2).is_err());
    }

    #[test]
    fn test_remove_rule() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("r1", AnomalyType::UnusualAmount, 3.0))
            .unwrap();
        let removed = det.remove_rule("r1").unwrap();
        assert_eq!(removed.id, "r1");
        assert!(det.rules.is_empty());
    }

    #[test]
    fn test_remove_missing_rule() {
        let mut det = AnomalyDetector::new();
        assert!(det.remove_rule("nope").is_err());
    }

    #[test]
    fn test_enable_rule() {
        let mut det = AnomalyDetector::new();
        let mut rule = make_rule("r1", AnomalyType::UnusualAmount, 3.0);
        rule.status = RuleStatus::Disabled;
        det.add_rule(rule).unwrap();
        det.enable_rule("r1").unwrap();
        assert_eq!(det.rules["r1"].status, RuleStatus::Enabled);
    }

    #[test]
    fn test_disable_rule() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("r1", AnomalyType::UnusualAmount, 3.0))
            .unwrap();
        det.disable_rule("r1").unwrap();
        assert_eq!(det.rules["r1"].status, RuleStatus::Disabled);
    }

    #[test]
    fn test_record_sample_creates_profile() {
        let mut det = AnomalyDetector::new();
        let sample = make_sample("alice", "bob", 100, 21, 10);
        det.record_sample(sample);
        let profile = det.get_profile("alice").unwrap();
        assert_eq!(profile.tx_count, 1);
        assert_eq!(profile.avg_amount, 100.0);
    }

    #[test]
    fn test_record_sample_updates_profile() {
        let mut det = AnomalyDetector::new();
        det.record_sample(make_sample("alice", "bob", 100, 20, 10));
        det.record_sample(make_sample("alice", "carol", 200, 40, 11));
        let profile = det.get_profile("alice").unwrap();
        assert_eq!(profile.tx_count, 2);
        assert!((profile.avg_amount - 150.0).abs() < 0.01);
        assert_eq!(profile.max_amount, 200);
        assert_eq!(profile.unique_recipients, 2);
    }

    #[test]
    fn test_analyze_unusual_amount() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("ua", AnomalyType::UnusualAmount, 2.0))
            .unwrap();

        // Build a profile with low average
        det.record_sample(make_sample("alice", "bob", 100, 20, 10));
        det.record_sample(make_sample("alice", "bob", 100, 20, 10));

        // Now analyze a high-value tx
        let big = make_sample("alice", "bob", 500, 20, 10);
        let alerts = det.analyze_sample(&big);
        assert!(!alerts.is_empty());
        assert_eq!(alerts[0].anomaly_type, AnomalyType::UnusualAmount);
    }

    #[test]
    fn test_analyze_new_recipient() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("nr", AnomalyType::NewRecipient, 1.0))
            .unwrap();

        det.record_sample(make_sample("alice", "bob", 100, 20, 10));

        let new_recip = make_sample("alice", "eve", 100, 20, 10);
        let alerts = det.analyze_sample(&new_recip);
        assert!(alerts
            .iter()
            .any(|a| a.anomaly_type == AnomalyType::NewRecipient));
    }

    #[test]
    fn test_analyze_no_alert_for_normal_tx() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("ua", AnomalyType::UnusualAmount, 3.0))
            .unwrap();

        det.record_sample(make_sample("alice", "bob", 100, 20, 10));
        det.record_sample(make_sample("alice", "bob", 110, 20, 10));

        // Amount within threshold
        let normal = make_sample("alice", "bob", 120, 20, 10);
        let alerts = det.analyze_sample(&normal);
        let unusual: Vec<_> = alerts
            .iter()
            .filter(|a| a.anomaly_type == AnomalyType::UnusualAmount)
            .collect();
        assert!(unusual.is_empty());
    }

    #[test]
    fn test_analyze_large_gas() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("lg", AnomalyType::LargeGas, 2.0))
            .unwrap();

        det.record_sample(make_sample("alice", "bob", 100, 20, 10));

        let high_gas = make_sample("alice", "bob", 100, 100, 10);
        let alerts = det.analyze_sample(&high_gas);
        assert!(alerts
            .iter()
            .any(|a| a.anomaly_type == AnomalyType::LargeGas));
    }

    #[test]
    fn test_acknowledge_alert() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("ua", AnomalyType::UnusualAmount, 1.5))
            .unwrap();

        det.record_sample(make_sample("alice", "bob", 100, 20, 10));
        let big = make_sample("alice", "bob", 500, 20, 10);
        let alerts = det.analyze_sample(&big);
        assert!(!alerts.is_empty());

        let alert_id = alerts[0].id.clone();
        det.acknowledge_alert(&alert_id).unwrap();

        let unack = det.unacknowledged_alerts();
        assert!(unack.iter().all(|a| a.id != alert_id));
    }

    #[test]
    fn test_acknowledge_missing_alert() {
        let mut det = AnomalyDetector::new();
        assert!(det.acknowledge_alert("nonexistent").is_err());
    }

    #[test]
    fn test_unacknowledged_alerts() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("ua", AnomalyType::UnusualAmount, 1.5))
            .unwrap();
        det.record_sample(make_sample("alice", "bob", 100, 20, 10));

        let big = make_sample("alice", "bob", 500, 20, 10);
        det.analyze_sample(&big);

        assert_eq!(det.unacknowledged_alerts().len(), 1);
    }

    #[test]
    fn test_alerts_by_risk() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("ua", AnomalyType::UnusualAmount, 1.5))
            .unwrap();
        det.record_sample(make_sample("alice", "bob", 100, 20, 10));

        let big = make_sample("alice", "bob", 500, 20, 10);
        det.analyze_sample(&big);

        let high = det.alerts_by_risk(&RiskLevel::High);
        assert!(!high.is_empty());
        let low = det.alerts_by_risk(&RiskLevel::Low);
        // UnusualAmount generates High, not Low
        assert!(low.is_empty());
    }

    #[test]
    fn test_alerts_for_address() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("nr", AnomalyType::NewRecipient, 1.0))
            .unwrap();
        det.record_sample(make_sample("alice", "bob", 100, 20, 10));

        let s = make_sample("alice", "eve", 100, 20, 10);
        det.analyze_sample(&s);

        let result = det.alerts_for_address("alice");
        assert!(!result.is_empty());
    }

    #[test]
    fn test_risk_score() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("ua", AnomalyType::UnusualAmount, 1.5))
            .unwrap();
        det.record_sample(make_sample("alice", "bob", 100, 20, 10));

        // Generate a High alert (20 points)
        let big = make_sample("alice", "bob", 500, 20, 10);
        det.analyze_sample(&big);

        let score = det.risk_score("alice");
        assert!((score - 20.0).abs() < 0.01);
    }

    #[test]
    fn test_risk_score_capped() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("dust", AnomalyType::DustAttack, 50.0))
            .unwrap();

        // Each DustAttack alert is Critical (40 points). 3 alerts = 120 -> capped to 100
        for i in 0..3 {
            let s = make_sample("alice", &format!("r{}", i), 1, 20, 10);
            det.analyze_sample(&s);
        }

        let score = det.risk_score("alice");
        assert!((score - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_recent_alerts() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("ua", AnomalyType::UnusualAmount, 1.5))
            .unwrap();
        det.record_sample(make_sample("alice", "bob", 100, 20, 10));

        for i in 0..5 {
            let s = make_sample("alice", "bob", 500 + i, 20, 10);
            det.analyze_sample(&s);
        }

        let recent = det.recent_alerts(3);
        assert_eq!(recent.len(), 3);
    }

    #[test]
    fn test_stats() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("ua", AnomalyType::UnusualAmount, 1.5))
            .unwrap();
        let mut disabled = make_rule("d1", AnomalyType::LargeGas, 2.0);
        disabled.status = RuleStatus::Disabled;
        det.add_rule(disabled).unwrap();

        det.record_sample(make_sample("alice", "bob", 100, 20, 10));
        let big = make_sample("alice", "bob", 500, 20, 10);
        det.analyze_sample(&big);

        let stats = det.stats();
        assert_eq!(stats.total_rules, 2);
        assert_eq!(stats.enabled_rules, 1);
        assert_eq!(stats.total_samples, 1);
        assert_eq!(stats.profiles, 1);
        assert!(stats.total_alerts >= 1);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path("roundtrip");
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("r1", AnomalyType::UnusualAmount, 3.0))
            .unwrap();
        det.record_sample(make_sample("alice", "bob", 100, 20, 10));
        det.save(&path).unwrap();

        let loaded = AnomalyDetector::load(&path).unwrap();
        assert_eq!(loaded.rules.len(), 1);
        assert_eq!(loaded.samples.len(), 1);
        assert_eq!(loaded.profiles.len(), 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing() {
        let path = test_path("missing_file_never_exists");
        let det = AnomalyDetector::load_or_default(&path);
        assert!(det.rules.is_empty());
        assert_eq!(det.max_samples, 50_000);
    }

    #[test]
    fn test_max_samples_trim() {
        let mut det = AnomalyDetector::new();
        det.max_samples = 5;
        for i in 0..10 {
            det.record_sample(make_sample("alice", "bob", i, 20, 10));
        }
        assert_eq!(det.samples.len(), 5);
        // The oldest samples should be trimmed; last sample has amount=9
        assert_eq!(det.samples.last().unwrap().amount, 9);
    }

    // ─── Additional coverage tests ────────────────────────────────────────────

    #[test]
    fn test_analyze_high_velocity_covers_lines_275_292() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("hv", AnomalyType::HighVelocity, 2.0))
            .unwrap();
        // 3 samples from alice at hour=10 loaded into self.samples
        det.record_sample(make_sample("alice", "bob", 100, 20, 10));
        det.record_sample(make_sample("alice", "carol", 100, 20, 10));
        det.record_sample(make_sample("alice", "dave", 100, 20, 10));
        // count_last_hour = 3 > 2.0 → HighVelocity Medium alert
        let s = make_sample("alice", "eve", 100, 20, 10);
        let alerts = det.analyze_sample(&s);
        assert!(alerts
            .iter()
            .any(|a| a.anomaly_type == AnomalyType::HighVelocity));
        let hv: Vec<_> = alerts
            .iter()
            .filter(|a| a.anomaly_type == AnomalyType::HighVelocity)
            .collect();
        assert_eq!(hv[0].risk_level, RiskLevel::Medium);
    }

    #[test]
    fn test_analyze_off_hours_covers_lines_343_357() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("oh", AnomalyType::OffHoursActivity, 1.0))
            .unwrap();
        // Build profile at hour=9 → common_hours=[9]
        det.record_sample(make_sample("alice", "bob", 100, 20, 9));
        det.record_sample(make_sample("alice", "carol", 100, 20, 9));
        // hour=22 not in common_hours → OffHoursActivity Low alert
        let s = make_sample("alice", "bob", 100, 20, 22);
        let alerts = det.analyze_sample(&s);
        assert!(alerts
            .iter()
            .any(|a| a.anomaly_type == AnomalyType::OffHoursActivity));
        let oh: Vec<_> = alerts
            .iter()
            .filter(|a| a.anomaly_type == AnomalyType::OffHoursActivity)
            .collect();
        assert_eq!(oh[0].risk_level, RiskLevel::Low);
    }

    #[test]
    fn test_analyze_rapid_sequence_covers_lines_359_376() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("rs", AnomalyType::RapidSequence, 5.0))
            .unwrap();
        // 20 samples from alice → rev().take(20).filter(from==alice).count() = 20 > 5.0
        for i in 0..20u64 {
            det.record_sample(make_sample("alice", "bob", 100 + i, 20, 10));
        }
        let s = make_sample("alice", "carol", 100, 20, 10);
        let alerts = det.analyze_sample(&s);
        assert!(alerts
            .iter()
            .any(|a| a.anomaly_type == AnomalyType::RapidSequence));
        let rs: Vec<_> = alerts
            .iter()
            .filter(|a| a.anomaly_type == AnomalyType::RapidSequence)
            .collect();
        assert_eq!(rs[0].risk_level, RiskLevel::High);
    }

    #[test]
    fn test_risk_score_medium_covers_line_484() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("hv", AnomalyType::HighVelocity, 2.0))
            .unwrap();
        for _ in 0..3 {
            det.record_sample(make_sample("alice", "bob", 100, 20, 10));
        }
        let s = make_sample("alice", "carol", 100, 20, 10);
        det.analyze_sample(&s);
        // HighVelocity = Medium = 10 pts
        let score = det.risk_score("alice");
        assert!((score - 10.0).abs() < 0.01);
    }

    #[test]
    fn test_risk_score_low_covers_line_485() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("nr", AnomalyType::NewRecipient, 1.0))
            .unwrap();
        det.record_sample(make_sample("alice", "bob", 100, 20, 10));
        // New recipient → Low alert (5 pts); details contain "sender alice"
        let s = make_sample("alice", "eve", 100, 20, 10);
        det.analyze_sample(&s);
        let score = det.risk_score("alice");
        assert!((score - 5.0).abs() < 0.01);
    }

    #[test]
    fn test_analyze_no_profile_none_paths_covers_lines_272_319_340_356() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("ua", AnomalyType::UnusualAmount, 2.0))
            .unwrap();
        det.add_rule(make_rule("nr", AnomalyType::NewRecipient, 1.0))
            .unwrap();
        det.add_rule(make_rule("lg", AnomalyType::LargeGas, 2.0))
            .unwrap();
        det.add_rule(make_rule("oh", AnomalyType::OffHoursActivity, 1.0))
            .unwrap();
        // No record_sample → profile is None → all profile-gated rules return None
        let s = make_sample("nobody", "eve", 100, 20, 10);
        let alerts = det.analyze_sample(&s);
        assert!(alerts.is_empty());
    }

    #[test]
    fn test_high_velocity_none_path_covers_line_292() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("hv", AnomalyType::HighVelocity, 10.0))
            .unwrap();
        // 1 sample → count_last_hour=1 ≤ 10.0 → None
        det.record_sample(make_sample("alice", "bob", 100, 20, 10));
        let s = make_sample("alice", "carol", 100, 20, 10);
        let alerts = det.analyze_sample(&s);
        assert!(alerts
            .iter()
            .all(|a| a.anomaly_type != AnomalyType::HighVelocity));
    }

    #[test]
    fn test_new_recipient_known_covers_line_313() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("nr", AnomalyType::NewRecipient, 1.0))
            .unwrap();
        det.record_sample(make_sample("alice", "bob", 100, 20, 10));
        // Same recipient → already known → None
        let s = make_sample("alice", "bob", 100, 20, 10);
        let alerts = det.analyze_sample(&s);
        assert!(alerts
            .iter()
            .all(|a| a.anomaly_type != AnomalyType::NewRecipient));
    }

    #[test]
    fn test_new_recipient_tx_count_zero_covers_line_316() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("nr", AnomalyType::NewRecipient, 1.0))
            .unwrap();
        // Directly inject a profile with tx_count=0
        det.profiles.insert(
            "alice".to_string(),
            BehaviorProfile {
                address: "alice".to_string(),
                avg_amount: 0.0,
                max_amount: 0,
                tx_count: 0,
                unique_recipients: 0,
                avg_gas: 0.0,
                common_hours: vec![],
                last_updated: "2026-01-01T00:00:00Z".to_string(),
            },
        );
        let s = make_sample("alice", "eve", 100, 20, 10);
        let alerts = det.analyze_sample(&s);
        assert!(alerts
            .iter()
            .all(|a| a.anomaly_type != AnomalyType::NewRecipient));
    }

    #[test]
    fn test_large_gas_none_path_covers_line_337() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("lg", AnomalyType::LargeGas, 5.0))
            .unwrap();
        det.record_sample(make_sample("alice", "bob", 100, 20, 10));
        // avg_gas≈20, threshold=5.0: gas=20 must be > 20*5=100 → not exceeded → None
        let s = make_sample("alice", "bob", 100, 20, 10);
        let alerts = det.analyze_sample(&s);
        assert!(alerts
            .iter()
            .all(|a| a.anomaly_type != AnomalyType::LargeGas));
    }

    #[test]
    fn test_off_hours_known_hour_covers_line_353() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("oh", AnomalyType::OffHoursActivity, 1.0))
            .unwrap();
        det.record_sample(make_sample("alice", "bob", 100, 20, 9));
        // hour=9 IS in common_hours → None
        let s = make_sample("alice", "bob", 100, 20, 9);
        let alerts = det.analyze_sample(&s);
        assert!(alerts
            .iter()
            .all(|a| a.anomaly_type != AnomalyType::OffHoursActivity));
    }

    #[test]
    fn test_rapid_sequence_none_path_covers_line_376() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("rs", AnomalyType::RapidSequence, 5.0))
            .unwrap();
        // Only 1 sample → recent=1 ≤ 5.0 → None
        det.record_sample(make_sample("alice", "bob", 100, 20, 10));
        let s = make_sample("alice", "carol", 100, 20, 10);
        let alerts = det.analyze_sample(&s);
        assert!(alerts
            .iter()
            .all(|a| a.anomaly_type != AnomalyType::RapidSequence));
    }

    #[test]
    fn test_dust_attack_none_path_covers_line_391() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule("dust", AnomalyType::DustAttack, 50.0))
            .unwrap();
        // amount=100 >= threshold=50.0 → condition false → None
        let s = make_sample("alice", "bob", 100, 20, 10);
        let alerts = det.analyze_sample(&s);
        assert!(alerts
            .iter()
            .all(|a| a.anomaly_type != AnomalyType::DustAttack));
    }

    #[test]
    fn test_custom_rule_always_none_covers_line_394() {
        let mut det = AnomalyDetector::new();
        det.add_rule(make_rule(
            "c1",
            AnomalyType::Custom("my_rule".to_string()),
            1.0,
        ))
        .unwrap();
        let s = make_sample("alice", "bob", 100, 20, 10);
        let alerts = det.analyze_sample(&s);
        assert!(alerts.is_empty());
    }
}
