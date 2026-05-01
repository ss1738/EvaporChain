// wallet/src/address_scoring.rs — Address risk scoring system
//
// Score addresses on multiple risk factors with automatic level classification,
// configurable rules engine, blacklist management, and persistent JSON storage.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

// ──────────────────────────── Errors ───────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum AddressScoringError {
    #[error("address not found: {0}")]
    NotFound(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ──────────────────────────── Enums ────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RiskLevel {
    Safe,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskFactor {
    KnownScam,
    FreshWallet,
    HighValueTarget,
    UnusualPattern,
    SanctionedEntity,
    MixerAssociated,
    DustAttack,
    PhishingRelated,
}

impl RiskFactor {
    /// Weight used when computing the aggregate risk score.
    fn weight(&self) -> u32 {
        match self {
            RiskFactor::KnownScam => 90,
            RiskFactor::SanctionedEntity => 85,
            RiskFactor::PhishingRelated => 70,
            RiskFactor::MixerAssociated => 60,
            RiskFactor::DustAttack => 40,
            RiskFactor::UnusualPattern => 30,
            RiskFactor::HighValueTarget => 20,
            RiskFactor::FreshWallet => 15,
        }
    }
}

// ──────────────────────────── AddressProfile ───────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressProfile {
    pub address: String,
    pub risk_level: RiskLevel,
    pub risk_score: u32,
    pub factors: Vec<RiskFactor>,
    pub labels: Vec<String>,
    pub first_seen: String,
    pub last_activity: Option<String>,
    pub tx_count: u64,
    pub total_volume: u64,
    pub notes: String,
    pub verified: bool,
    pub updated_at: String,
}

impl AddressProfile {
    pub fn new(address: &str) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            address: address.to_string(),
            risk_level: RiskLevel::Safe,
            risk_score: 0,
            factors: Vec::new(),
            labels: Vec::new(),
            first_seen: now.clone(),
            last_activity: None,
            tx_count: 0,
            total_volume: 0,
            notes: String::new(),
            verified: false,
            updated_at: now,
        }
    }

    pub fn add_factor(&mut self, factor: RiskFactor) {
        if !self.factors.contains(&factor) {
            self.factors.push(factor);
            self.recalculate();
        }
    }

    pub fn remove_factor(&mut self, factor: &RiskFactor) -> bool {
        let before = self.factors.len();
        self.factors.retain(|f| f != factor);
        let removed = self.factors.len() < before;
        if removed {
            self.recalculate();
        }
        removed
    }

    pub fn add_label(&mut self, label: &str) {
        let s = label.to_string();
        if !self.labels.contains(&s) {
            self.labels.push(s);
        }
    }

    pub fn has_label(&self, label: &str) -> bool {
        self.labels.iter().any(|l| l == label)
    }

    pub fn has_factor(&self, factor: &RiskFactor) -> bool {
        self.factors.contains(factor)
    }

    pub fn recalculate(&mut self) {
        let raw: u32 = self.factors.iter().map(|f| f.weight()).sum();
        self.risk_score = raw.min(100);
        self.risk_level = match self.risk_score {
            0..=20 => RiskLevel::Safe,
            21..=40 => RiskLevel::Low,
            41..=60 => RiskLevel::Medium,
            61..=80 => RiskLevel::High,
            _ => RiskLevel::Critical,
        };
        self.updated_at = chrono::Utc::now().to_rfc3339();
    }

    pub fn record_activity(&mut self, volume: u64) {
        self.tx_count += 1;
        self.total_volume += volume;
        self.last_activity = Some(chrono::Utc::now().to_rfc3339());
    }

    pub fn is_safe(&self) -> bool {
        self.risk_level == RiskLevel::Safe
    }

    pub fn is_risky(&self) -> bool {
        self.risk_level >= RiskLevel::High
    }
}

// ──────────────────────────── Scoring Rules ────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuleCondition {
    TxCountAbove(u64),
    VolumeAbove(u64),
    NoActivityDays(u64),
    LabelContains(String),
    Always,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoringRule {
    pub id: String,
    pub name: String,
    pub description: String,
    pub condition: RuleCondition,
    pub factor_to_add: RiskFactor,
    pub enabled: bool,
}

impl ScoringRule {
    pub fn new(id: &str, name: &str, factor: RiskFactor, condition: RuleCondition) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            description: String::new(),
            condition,
            factor_to_add: factor,
            enabled: true,
        }
    }

    pub fn matches(&self, profile: &AddressProfile) -> bool {
        if !self.enabled {
            return false;
        }
        match &self.condition {
            RuleCondition::TxCountAbove(threshold) => profile.tx_count > *threshold,
            RuleCondition::VolumeAbove(threshold) => profile.total_volume > *threshold,
            RuleCondition::NoActivityDays(days) => match &profile.last_activity {
                None => true,
                Some(ts) => {
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
                        let delta = chrono::Utc::now().signed_duration_since(dt);
                        delta.num_days() > *days as i64
                    } else {
                        true
                    }
                }
            },
            RuleCondition::LabelContains(label) => {
                let lower = label.to_lowercase();
                profile
                    .labels
                    .iter()
                    .any(|l| l.to_lowercase().contains(&lower))
            }
            RuleCondition::Always => true,
        }
    }
}

// ──────────────────────────── ScorerStats ──────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScorerStats {
    pub total_profiles: usize,
    pub safe: usize,
    pub low: usize,
    pub medium: usize,
    pub high: usize,
    pub critical: usize,
    pub blacklisted: usize,
    pub rules: usize,
}

// ──────────────────────────── AddressScorer ─────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AddressScorer {
    pub profiles: HashMap<String, AddressProfile>,
    pub rules: Vec<ScoringRule>,
    pub blacklist: Vec<String>,
}

impl AddressScorer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn score_address(&mut self, address: &str) -> &AddressProfile {
        // Create profile if it doesn't exist.
        if !self.profiles.contains_key(address) {
            let profile = AddressProfile::new(address);
            self.profiles.insert(address.to_string(), profile);
        }

        // If blacklisted, ensure KnownScam is present.
        if self.blacklist.contains(&address.to_string()) {
            let p = self.profiles.get_mut(address).unwrap();
            p.add_factor(RiskFactor::KnownScam);
        }

        // Collect matching factors from rules.
        let factors_to_add: Vec<RiskFactor> = self
            .rules
            .iter()
            .filter(|r| r.matches(self.profiles.get(address).unwrap()))
            .map(|r| r.factor_to_add.clone())
            .collect();

        let profile = self.profiles.get_mut(address).unwrap();
        for factor in factors_to_add {
            profile.add_factor(factor);
        }

        self.profiles.get(address).unwrap()
    }

    pub fn get_profile(&self, address: &str) -> Option<&AddressProfile> {
        self.profiles.get(address)
    }

    pub fn get_profile_mut(&mut self, address: &str) -> Option<&mut AddressProfile> {
        self.profiles.get_mut(address)
    }

    pub fn add_profile(&mut self, profile: AddressProfile) {
        self.profiles.insert(profile.address.clone(), profile);
    }

    pub fn add_rule(&mut self, rule: ScoringRule) {
        self.rules.push(rule);
    }

    pub fn remove_rule(&mut self, id: &str) -> bool {
        let before = self.rules.len();
        self.rules.retain(|r| r.id != id);
        self.rules.len() < before
    }

    pub fn add_to_blacklist(&mut self, address: &str) {
        let addr = address.to_string();
        if !self.blacklist.contains(&addr) {
            self.blacklist.push(addr);
        }
        if let Some(profile) = self.profiles.get_mut(address) {
            profile.add_factor(RiskFactor::KnownScam);
        }
    }

    pub fn is_blacklisted(&self, address: &str) -> bool {
        self.blacklist.contains(&address.to_string())
    }

    pub fn remove_from_blacklist(&mut self, address: &str) -> bool {
        let before = self.blacklist.len();
        self.blacklist.retain(|a| a != address);
        self.blacklist.len() < before
    }

    pub fn risky_addresses(&self) -> Vec<&AddressProfile> {
        self.profiles.values().filter(|p| p.is_risky()).collect()
    }

    pub fn safe_addresses(&self) -> Vec<&AddressProfile> {
        self.profiles.values().filter(|p| p.is_safe()).collect()
    }

    pub fn by_risk_level(&self, level: &RiskLevel) -> Vec<&AddressProfile> {
        self.profiles
            .values()
            .filter(|p| p.risk_level == *level)
            .collect()
    }

    pub fn search(&self, query: &str) -> Vec<&AddressProfile> {
        let q = query.to_lowercase();
        self.profiles
            .values()
            .filter(|p| {
                p.address.to_lowercase().contains(&q)
                    || p.labels.iter().any(|l| l.to_lowercase().contains(&q))
                    || p.notes.to_lowercase().contains(&q)
            })
            .collect()
    }

    pub fn batch_score(&mut self, addresses: &[&str]) -> Vec<RiskLevel> {
        let mut levels = Vec::with_capacity(addresses.len());
        for addr in addresses {
            let profile = self.score_address(addr);
            levels.push(profile.risk_level);
        }
        levels
    }

    pub fn stats(&self) -> ScorerStats {
        let mut s = ScorerStats {
            total_profiles: self.profiles.len(),
            safe: 0,
            low: 0,
            medium: 0,
            high: 0,
            critical: 0,
            blacklisted: self.blacklist.len(),
            rules: self.rules.len(),
        };
        for p in self.profiles.values() {
            match p.risk_level {
                RiskLevel::Safe => s.safe += 1,
                RiskLevel::Low => s.low += 1,
                RiskLevel::Medium => s.medium += 1,
                RiskLevel::High => s.high += 1,
                RiskLevel::Critical => s.critical += 1,
            }
        }
        s
    }

    // ── Persistence ────────────────────────────────────────────

    pub fn save(&self, path: &Path) -> Result<(), AddressScoringError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, AddressScoringError> {
        let data = std::fs::read_to_string(path)?;
        let scorer: Self = serde_json::from_str(&data)?;
        Ok(scorer)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ──────────────────────────── Tests ────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("addr_score_test_{}_{name}", std::process::id()))
    }

    #[test]
    fn test_new_profile_is_safe() {
        let p = AddressProfile::new("evap1abc");
        assert!(p.is_safe());
        assert_eq!(p.risk_score, 0);
        assert_eq!(p.risk_level, RiskLevel::Safe);
        assert!(p.factors.is_empty());
        assert!(p.labels.is_empty());
        assert_eq!(p.tx_count, 0);
        assert_eq!(p.total_volume, 0);
    }

    #[test]
    fn test_add_factor_recalculates() {
        let mut p = AddressProfile::new("evap1abc");
        p.add_factor(RiskFactor::FreshWallet);
        assert_eq!(p.risk_score, 15);
        assert_eq!(p.risk_level, RiskLevel::Safe);

        p.add_factor(RiskFactor::UnusualPattern);
        assert_eq!(p.risk_score, 45); // 15 + 30
        assert_eq!(p.risk_level, RiskLevel::Medium);
    }

    #[test]
    fn test_remove_factor() {
        let mut p = AddressProfile::new("evap1abc");
        p.add_factor(RiskFactor::DustAttack);
        p.add_factor(RiskFactor::FreshWallet);
        assert_eq!(p.risk_score, 55); // 40 + 15

        assert!(p.remove_factor(&RiskFactor::DustAttack));
        assert_eq!(p.risk_score, 15);
        assert!(!p.has_factor(&RiskFactor::DustAttack));

        // Removing non-existent returns false.
        assert!(!p.remove_factor(&RiskFactor::KnownScam));
    }

    #[test]
    fn test_known_scam_is_critical() {
        let mut p = AddressProfile::new("scammer");
        p.add_factor(RiskFactor::KnownScam);
        assert_eq!(p.risk_score, 90);
        assert_eq!(p.risk_level, RiskLevel::Critical);
    }

    #[test]
    fn test_multiple_factors_stack() {
        let mut p = AddressProfile::new("evap1xyz");
        p.add_factor(RiskFactor::FreshWallet); // 15
        p.add_factor(RiskFactor::HighValueTarget); // 20
        p.add_factor(RiskFactor::UnusualPattern); // 30
        assert_eq!(p.risk_score, 65); // 15+20+30
        assert_eq!(p.risk_level, RiskLevel::High);
    }

    #[test]
    fn test_score_capped_at_100() {
        let mut p = AddressProfile::new("evap1max");
        p.add_factor(RiskFactor::KnownScam); // 90
        p.add_factor(RiskFactor::SanctionedEntity); // 85
        p.add_factor(RiskFactor::PhishingRelated); // 70
                                                   // Raw = 245, capped to 100.
        assert_eq!(p.risk_score, 100);
        assert_eq!(p.risk_level, RiskLevel::Critical);
    }

    #[test]
    fn test_record_activity() {
        let mut p = AddressProfile::new("evap1act");
        assert!(p.last_activity.is_none());
        p.record_activity(500);
        assert_eq!(p.tx_count, 1);
        assert_eq!(p.total_volume, 500);
        assert!(p.last_activity.is_some());

        p.record_activity(300);
        assert_eq!(p.tx_count, 2);
        assert_eq!(p.total_volume, 800);
    }

    #[test]
    fn test_labels() {
        let mut p = AddressProfile::new("evap1lbl");
        p.add_label("exchange");
        p.add_label("known whale");
        assert!(p.has_label("exchange"));
        assert!(p.has_label("known whale"));
        assert!(!p.has_label("bridge"));

        // Duplicate is ignored.
        p.add_label("exchange");
        assert_eq!(p.labels.len(), 2);
    }

    #[test]
    fn test_is_risky() {
        let mut p = AddressProfile::new("evap1r");
        assert!(!p.is_risky());

        p.add_factor(RiskFactor::MixerAssociated); // 60
        assert!(!p.is_risky()); // Medium

        p.add_factor(RiskFactor::FreshWallet); // +15 = 75
        assert!(p.is_risky()); // High
    }

    #[test]
    fn test_scoring_rule_matches() {
        let rule = ScoringRule::new(
            "always",
            "Always flag",
            RiskFactor::FreshWallet,
            RuleCondition::Always,
        );
        let p = AddressProfile::new("evap1any");
        assert!(rule.matches(&p));

        let mut disabled = rule.clone();
        disabled.enabled = false;
        assert!(!disabled.matches(&p));
    }

    #[test]
    fn test_scoring_rule_tx_count() {
        let rule = ScoringRule::new(
            "high-tx",
            "High tx count",
            RiskFactor::HighValueTarget,
            RuleCondition::TxCountAbove(100),
        );
        let mut p = AddressProfile::new("evap1tx");
        assert!(!rule.matches(&p));

        p.tx_count = 101;
        assert!(rule.matches(&p));
    }

    #[test]
    fn test_scoring_rule_volume() {
        let rule = ScoringRule::new(
            "high-vol",
            "High volume",
            RiskFactor::HighValueTarget,
            RuleCondition::VolumeAbove(1_000_000),
        );
        let mut p = AddressProfile::new("evap1vol");
        p.total_volume = 999_999;
        assert!(!rule.matches(&p));

        p.total_volume = 1_000_001;
        assert!(rule.matches(&p));
    }

    #[test]
    fn test_score_address_creates_profile() {
        let mut scorer = AddressScorer::new();
        assert!(scorer.get_profile("evap1new").is_none());
        scorer.score_address("evap1new");
        assert!(scorer.get_profile("evap1new").is_some());
    }

    #[test]
    fn test_score_address_applies_rules() {
        let mut scorer = AddressScorer::new();
        scorer.add_rule(ScoringRule::new(
            "flag-all",
            "Flag everything",
            RiskFactor::FreshWallet,
            RuleCondition::Always,
        ));

        let profile = scorer.score_address("evap1rule");
        assert!(profile.has_factor(&RiskFactor::FreshWallet));
        assert_eq!(profile.risk_score, 15);
    }

    #[test]
    fn test_blacklist() {
        let mut scorer = AddressScorer::new();
        scorer.add_to_blacklist("evap1bad");
        assert!(scorer.is_blacklisted("evap1bad"));

        assert!(scorer.remove_from_blacklist("evap1bad"));
        assert!(!scorer.is_blacklisted("evap1bad"));

        // Removing non-existent returns false.
        assert!(!scorer.remove_from_blacklist("evap1nobody"));
    }

    #[test]
    fn test_blacklist_auto_flags() {
        let mut scorer = AddressScorer::new();
        // Pre-create a profile, then blacklist it.
        scorer.add_profile(AddressProfile::new("evap1evil"));
        scorer.add_to_blacklist("evap1evil");

        let p = scorer.get_profile("evap1evil").unwrap();
        assert!(p.has_factor(&RiskFactor::KnownScam));
        assert_eq!(p.risk_level, RiskLevel::Critical);
    }

    #[test]
    fn test_risky_and_safe_addresses() {
        let mut scorer = AddressScorer::new();

        let safe = AddressProfile::new("evap1safe");
        let mut risky = AddressProfile::new("evap1risky");
        risky.add_factor(RiskFactor::KnownScam);

        scorer.add_profile(safe);
        scorer.add_profile(risky);

        assert_eq!(scorer.safe_addresses().len(), 1);
        assert_eq!(scorer.risky_addresses().len(), 1);
        assert_eq!(scorer.safe_addresses()[0].address, "evap1safe");
        assert_eq!(scorer.risky_addresses()[0].address, "evap1risky");
    }

    #[test]
    fn test_by_risk_level() {
        let mut scorer = AddressScorer::new();
        scorer.add_profile(AddressProfile::new("a1")); // Safe

        let mut med = AddressProfile::new("a2");
        med.add_factor(RiskFactor::DustAttack); // 40 -> Low
        med.add_factor(RiskFactor::FreshWallet); // +15 = 55 -> Medium
        scorer.add_profile(med);

        assert_eq!(scorer.by_risk_level(&RiskLevel::Safe).len(), 1);
        assert_eq!(scorer.by_risk_level(&RiskLevel::Medium).len(), 1);
        assert_eq!(scorer.by_risk_level(&RiskLevel::Critical).len(), 0);
    }

    #[test]
    fn test_search() {
        let mut scorer = AddressScorer::new();

        let mut p1 = AddressProfile::new("evap1alice");
        p1.add_label("exchange");
        scorer.add_profile(p1);

        let mut p2 = AddressProfile::new("evap1bob");
        p2.notes = "suspicious exchange activity".to_string();
        scorer.add_profile(p2);

        let p3 = AddressProfile::new("evap1charlie");
        scorer.add_profile(p3);

        // Search by address.
        assert_eq!(scorer.search("alice").len(), 1);
        // Search by label and notes (case-insensitive).
        assert_eq!(scorer.search("EXCHANGE").len(), 2);
        // No match.
        assert_eq!(scorer.search("zzzz").len(), 0);
    }

    #[test]
    fn test_batch_score() {
        let mut scorer = AddressScorer::new();
        scorer.add_rule(ScoringRule::new(
            "fresh",
            "Fresh",
            RiskFactor::FreshWallet,
            RuleCondition::Always,
        ));

        let addrs = vec!["a1", "a2", "a3"];
        let levels = scorer.batch_score(&addrs);
        assert_eq!(levels.len(), 3);
        // FreshWallet = 15 -> Safe
        assert!(levels.iter().all(|l| *l == RiskLevel::Safe));
    }

    #[test]
    fn test_stats() {
        let mut scorer = AddressScorer::new();
        scorer.add_profile(AddressProfile::new("s1")); // Safe

        let mut critical = AddressProfile::new("c1");
        critical.add_factor(RiskFactor::KnownScam);
        scorer.add_profile(critical);

        scorer.add_to_blacklist("c1");
        scorer.add_rule(ScoringRule::new(
            "r1",
            "Rule",
            RiskFactor::FreshWallet,
            RuleCondition::Always,
        ));

        let st = scorer.stats();
        assert_eq!(st.total_profiles, 2);
        assert_eq!(st.safe, 1);
        assert_eq!(st.critical, 1);
        assert_eq!(st.blacklisted, 1);
        assert_eq!(st.rules, 1);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path("roundtrip.json");

        let mut scorer = AddressScorer::new();
        let mut p = AddressProfile::new("evap1persist");
        p.add_factor(RiskFactor::DustAttack);
        p.add_label("exchange");
        p.record_activity(1000);
        scorer.add_profile(p);
        scorer.add_to_blacklist("evap1scam");
        scorer.add_rule(ScoringRule::new(
            "r1",
            "Test rule",
            RiskFactor::FreshWallet,
            RuleCondition::TxCountAbove(50),
        ));

        scorer.save(&path).unwrap();

        let loaded = AddressScorer::load(&path).unwrap();
        assert_eq!(loaded.profiles.len(), 1);
        let lp = loaded.get_profile("evap1persist").unwrap();
        assert_eq!(lp.risk_score, 40);
        assert!(lp.has_factor(&RiskFactor::DustAttack));
        assert!(lp.has_label("exchange"));
        assert_eq!(lp.tx_count, 1);
        assert_eq!(lp.total_volume, 1000);
        assert!(loaded.is_blacklisted("evap1scam"));
        assert_eq!(loaded.rules.len(), 1);

        // Clean up.
        let _ = std::fs::remove_file(&path);

        // load_or_default on missing file returns default.
        let def = AddressScorer::load_or_default(&path);
        assert_eq!(def.profiles.len(), 0);
    }
}
