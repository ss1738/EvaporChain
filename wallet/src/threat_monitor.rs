//! Real-time Threat Detection — phishing, malicious contracts, dust attacks,
//! address poisoning, and more.
//!
//! Maintains a rolling threat log (max 1 000 entries), a phishing URL
//! blacklist, a malicious-contract registry, and a URL safelist. Provides
//! fast look-ups so the wallet can warn users before they interact with
//! known-bad resources.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

// ──────────────────────────── Error ────────────────────────────────────────

/// Errors produced by the threat monitor.
#[derive(Debug, thiserror::Error)]
pub enum ThreatMonitorError {
    #[error("threat not found: {0}")]
    NotFound(String),
    #[error("threat already resolved: {0}")]
    AlreadyResolved(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ──────────────────────────── Enums ────────────────────────────────────────

/// Severity level of a detected threat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ThreatLevel {
    None,
    Low,
    Medium,
    High,
    Critical,
}

/// Category of a detected threat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThreatType {
    PhishingUrl,
    MaliciousContract,
    SuspiciousTransaction,
    DustAttack,
    AddressPoisoning,
    FakeToken,
    ReplayAttack,
    SocialEngineering,
}

// ──────────────────────────── Threat ───────────────────────────────────────

/// A single detected threat event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Threat {
    pub id: String,
    pub threat_type: ThreatType,
    pub level: ThreatLevel,
    pub source: String,
    pub target: String,
    pub description: String,
    pub detected_at: String,
    pub resolved: bool,
    pub resolved_at: Option<String>,
    pub false_positive: bool,
    pub metadata: HashMap<String, String>,
}

impl Threat {
    pub fn new(
        id: impl Into<String>,
        threat_type: ThreatType,
        level: ThreatLevel,
        source: impl Into<String>,
        target: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            threat_type,
            level,
            source: source.into(),
            target: target.into(),
            description: description.into(),
            detected_at: chrono::Utc::now().to_rfc3339(),
            resolved: false,
            resolved_at: None,
            false_positive: false,
            metadata: HashMap::new(),
        }
    }

    /// Mark this threat as resolved.
    pub fn resolve(&mut self) {
        self.resolved = true;
        self.resolved_at = Some(chrono::Utc::now().to_rfc3339());
    }

    /// Mark this threat as a false positive (also resolves it).
    pub fn mark_false_positive(&mut self) {
        self.false_positive = true;
        self.resolve();
    }

    /// Returns `true` when the threat has not been resolved yet.
    pub fn is_active(&self) -> bool {
        !self.resolved
    }

    /// Builder-style helper to attach metadata.
    pub fn with_metadata(mut self, key: &str, value: &str) -> Self {
        self.metadata.insert(key.to_string(), value.to_string());
        self
    }
}

// ──────────────────────────── PhishingEntry ────────────────────────────────

/// A reported phishing URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhishingEntry {
    pub url: String,
    pub reported_at: String,
    pub confirmed: bool,
    pub reporter: String,
}

impl PhishingEntry {
    pub fn new(url: impl Into<String>, reporter: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            reported_at: chrono::Utc::now().to_rfc3339(),
            confirmed: false,
            reporter: reporter.into(),
        }
    }

    pub fn confirm(&mut self) {
        self.confirmed = true;
    }
}

// ──────────────────────────── MaliciousContract ───────────────────────────

/// A reported malicious smart contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaliciousContract {
    pub address: String,
    pub reason: String,
    pub reported_at: String,
    pub confirmed: bool,
    pub threat_level: ThreatLevel,
}

impl MaliciousContract {
    pub fn new(address: impl Into<String>, reason: impl Into<String>, level: ThreatLevel) -> Self {
        Self {
            address: address.into(),
            reason: reason.into(),
            reported_at: chrono::Utc::now().to_rfc3339(),
            confirmed: false,
            threat_level: level,
        }
    }

    pub fn confirm(&mut self) {
        self.confirmed = true;
    }
}

// ──────────────────────────── ThreatStats ─────────────────────────────────

/// Aggregate statistics for the threat monitor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreatStats {
    pub total_threats: usize,
    pub active_threats: usize,
    pub resolved: usize,
    pub false_positives: usize,
    pub phishing_urls: usize,
    pub malicious_contracts: usize,
    pub safe_urls: usize,
    pub scan_count: u64,
}

// ──────────────────────────── ThreatMonitor ───────────────────────────────

const MAX_THREATS: usize = 1000;

/// Central threat monitor — tracks threats, phishing URLs, malicious
/// contracts, and a URL safelist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreatMonitor {
    pub threats: Vec<Threat>,
    pub phishing_urls: HashMap<String, PhishingEntry>,
    pub malicious_contracts: HashMap<String, MaliciousContract>,
    pub safe_urls: Vec<String>,
    pub scan_count: u64,
}

impl Default for ThreatMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl ThreatMonitor {
    // ── Construction ──────────────────────────────────────────────────

    pub fn new() -> Self {
        Self {
            threats: Vec::new(),
            phishing_urls: HashMap::new(),
            malicious_contracts: HashMap::new(),
            safe_urls: Vec::new(),
            scan_count: 0,
        }
    }

    // ── Reporting ─────────────────────────────────────────────────────

    /// Record a new threat. Prunes oldest entries when the log exceeds
    /// 1 000 items.
    pub fn report_threat(&mut self, threat: Threat) {
        self.threats.push(threat);
        if self.threats.len() > MAX_THREATS {
            self.threats.remove(0);
        }
        self.scan_count += 1;
    }

    /// Report a phishing URL.
    pub fn report_phishing(&mut self, url: &str, reporter: &str) {
        let entry = PhishingEntry::new(url, reporter);
        self.phishing_urls.insert(url.to_string(), entry);
    }

    /// Report a malicious contract.
    pub fn report_malicious_contract(&mut self, address: &str, reason: &str, level: ThreatLevel) {
        let entry = MaliciousContract::new(address, reason, level);
        self.malicious_contracts.insert(address.to_string(), entry);
    }

    // ── Look-ups ──────────────────────────────────────────────────────

    /// Returns `true` if the URL (or any prefix of it) is in the phishing
    /// database.
    pub fn is_phishing(&self, url: &str) -> bool {
        if self.phishing_urls.contains_key(url) {
            return true;
        }
        // Check if any known phishing URL is a prefix of the given URL.
        for known in self.phishing_urls.keys() {
            if url.starts_with(known.as_str()) {
                return true;
            }
        }
        false
    }

    /// Returns `true` if the address is in the malicious-contract registry.
    pub fn is_malicious_contract(&self, address: &str) -> bool {
        self.malicious_contracts.contains_key(address)
    }

    /// Assess the threat level for a URL.
    pub fn check_url(&self, url: &str) -> ThreatLevel {
        if self.is_phishing(url) {
            return ThreatLevel::High;
        }
        if self.safe_urls.iter().any(|s| s == url) {
            return ThreatLevel::None;
        }
        ThreatLevel::Low
    }

    /// Assess the threat level for a contract address.
    pub fn check_contract(&self, address: &str) -> ThreatLevel {
        match self.malicious_contracts.get(address) {
            Some(mc) => mc.threat_level,
            None => ThreatLevel::None,
        }
    }

    /// Assess the threat level for a transaction.
    pub fn check_transaction(
        &self,
        to: &str,
        amount: u64,
        sender_history_count: u64,
    ) -> ThreatLevel {
        if self.is_malicious_contract(to) {
            return ThreatLevel::Critical;
        }
        if amount < 10 && sender_history_count < 2 {
            return ThreatLevel::Medium;
        }
        ThreatLevel::None
    }

    // ── Safelist management ───────────────────────────────────────────

    /// Add a URL to the safelist.
    pub fn add_safe_url(&mut self, url: &str) {
        if !self.safe_urls.contains(&url.to_string()) {
            self.safe_urls.push(url.to_string());
        }
    }

    /// Remove a URL from the safelist. Returns `true` if it was present.
    pub fn remove_safe_url(&mut self, url: &str) -> bool {
        if let Some(pos) = self.safe_urls.iter().position(|s| s == url) {
            self.safe_urls.remove(pos);
            true
        } else {
            false
        }
    }

    // ── Queries ───────────────────────────────────────────────────────

    /// All threats that have not been resolved.
    pub fn active_threats(&self) -> Vec<&Threat> {
        self.threats.iter().filter(|t| t.is_active()).collect()
    }

    /// Filter threats by type.
    pub fn threats_by_type(&self, tt: &ThreatType) -> Vec<&Threat> {
        self.threats
            .iter()
            .filter(|t| &t.threat_type == tt)
            .collect()
    }

    /// Filter threats by level.
    pub fn threats_by_level(&self, level: &ThreatLevel) -> Vec<&Threat> {
        self.threats.iter().filter(|t| &t.level == level).collect()
    }

    /// Resolve a threat by id.
    pub fn resolve_threat(&mut self, id: &str) -> Result<(), ThreatMonitorError> {
        let threat = self
            .threats
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or_else(|| ThreatMonitorError::NotFound(id.to_string()))?;
        if threat.resolved {
            return Err(ThreatMonitorError::AlreadyResolved(id.to_string()));
        }
        threat.resolve();
        Ok(())
    }

    /// Mark a threat as a false positive by id.
    pub fn false_positive(&mut self, id: &str) -> Result<(), ThreatMonitorError> {
        let threat = self
            .threats
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or_else(|| ThreatMonitorError::NotFound(id.to_string()))?;
        if threat.resolved {
            return Err(ThreatMonitorError::AlreadyResolved(id.to_string()));
        }
        threat.mark_false_positive();
        Ok(())
    }

    /// Return the most recent `n` threats (newest first).
    pub fn recent_threats(&self, n: usize) -> Vec<&Threat> {
        self.threats.iter().rev().take(n).collect()
    }

    /// Count of threats grouped by type (debug string key).
    pub fn threat_summary(&self) -> HashMap<String, usize> {
        let mut map: HashMap<String, usize> = HashMap::new();
        for t in &self.threats {
            let key = format!("{:?}", t.threat_type);
            *map.entry(key).or_insert(0) += 1;
        }
        map
    }

    /// Aggregate statistics.
    pub fn stats(&self) -> ThreatStats {
        let active = self.threats.iter().filter(|t| t.is_active()).count();
        let resolved = self.threats.iter().filter(|t| t.resolved).count();
        let fp = self.threats.iter().filter(|t| t.false_positive).count();

        ThreatStats {
            total_threats: self.threats.len(),
            active_threats: active,
            resolved,
            false_positives: fp,
            phishing_urls: self.phishing_urls.len(),
            malicious_contracts: self.malicious_contracts.len(),
            safe_urls: self.safe_urls.len(),
            scan_count: self.scan_count,
        }
    }

    // ── Persistence ───────────────────────────────────────────────────

    pub fn save(&self, path: &Path) -> Result<(), ThreatMonitorError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, ThreatMonitorError> {
        let data = std::fs::read_to_string(path)?;
        let monitor: ThreatMonitor = serde_json::from_str(&data)?;
        Ok(monitor)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ──────────────────────────── Tests ───────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("threat_mon_test_{}_{name}", std::process::id()))
    }

    fn sample_threat(id: &str, tt: ThreatType, level: ThreatLevel) -> Threat {
        Threat::new(id, tt, level, "src", "tgt", "desc")
    }

    // ── Basics ────────────────────────────────────────────────────────

    #[test]
    fn test_report_threat() {
        let mut m = ThreatMonitor::new();
        let t = sample_threat("t1", ThreatType::PhishingUrl, ThreatLevel::High);
        m.report_threat(t);
        assert_eq!(m.threats.len(), 1);
        assert_eq!(m.scan_count, 1);
    }

    #[test]
    fn test_resolve_threat() {
        let mut m = ThreatMonitor::new();
        m.report_threat(sample_threat(
            "t1",
            ThreatType::PhishingUrl,
            ThreatLevel::High,
        ));
        assert!(m.threats[0].is_active());
        m.resolve_threat("t1").unwrap();
        assert!(!m.threats[0].is_active());
        assert!(m.threats[0].resolved_at.is_some());
    }

    #[test]
    fn test_false_positive() {
        let mut m = ThreatMonitor::new();
        m.report_threat(sample_threat(
            "t1",
            ThreatType::DustAttack,
            ThreatLevel::Medium,
        ));
        m.false_positive("t1").unwrap();
        assert!(m.threats[0].false_positive);
        assert!(m.threats[0].resolved);
    }

    #[test]
    fn test_active_threats() {
        let mut m = ThreatMonitor::new();
        m.report_threat(sample_threat(
            "t1",
            ThreatType::PhishingUrl,
            ThreatLevel::High,
        ));
        m.report_threat(sample_threat(
            "t2",
            ThreatType::DustAttack,
            ThreatLevel::Low,
        ));
        m.resolve_threat("t1").unwrap();
        let active = m.active_threats();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "t2");
    }

    #[test]
    fn test_threats_by_type() {
        let mut m = ThreatMonitor::new();
        m.report_threat(sample_threat(
            "t1",
            ThreatType::PhishingUrl,
            ThreatLevel::High,
        ));
        m.report_threat(sample_threat(
            "t2",
            ThreatType::DustAttack,
            ThreatLevel::Medium,
        ));
        m.report_threat(sample_threat(
            "t3",
            ThreatType::PhishingUrl,
            ThreatLevel::Low,
        ));
        let phishing = m.threats_by_type(&ThreatType::PhishingUrl);
        assert_eq!(phishing.len(), 2);
    }

    #[test]
    fn test_threats_by_level() {
        let mut m = ThreatMonitor::new();
        m.report_threat(sample_threat(
            "t1",
            ThreatType::PhishingUrl,
            ThreatLevel::High,
        ));
        m.report_threat(sample_threat(
            "t2",
            ThreatType::DustAttack,
            ThreatLevel::High,
        ));
        m.report_threat(sample_threat("t3", ThreatType::FakeToken, ThreatLevel::Low));
        let high = m.threats_by_level(&ThreatLevel::High);
        assert_eq!(high.len(), 2);
    }

    // ── Phishing ──────────────────────────────────────────────────────

    #[test]
    fn test_report_phishing() {
        let mut m = ThreatMonitor::new();
        m.report_phishing("https://evil.com", "alice");
        assert_eq!(m.phishing_urls.len(), 1);
        assert_eq!(m.phishing_urls["https://evil.com"].reporter, "alice");
    }

    #[test]
    fn test_is_phishing() {
        let mut m = ThreatMonitor::new();
        m.report_phishing("https://evil.com", "alice");
        assert!(m.is_phishing("https://evil.com"));
        assert!(!m.is_phishing("https://good.com"));
    }

    #[test]
    fn test_is_phishing_prefix_match() {
        let mut m = ThreatMonitor::new();
        m.report_phishing("https://evil.com", "alice");
        assert!(m.is_phishing("https://evil.com/login"));
        assert!(!m.is_phishing("https://good.com/evil.com"));
    }

    // ── Malicious contracts ───────────────────────────────────────────

    #[test]
    fn test_report_malicious_contract() {
        let mut m = ThreatMonitor::new();
        m.report_malicious_contract("0xdead", "rug pull", ThreatLevel::Critical);
        assert_eq!(m.malicious_contracts.len(), 1);
        assert_eq!(m.malicious_contracts["0xdead"].reason, "rug pull");
    }

    #[test]
    fn test_is_malicious_contract() {
        let mut m = ThreatMonitor::new();
        m.report_malicious_contract("0xdead", "scam", ThreatLevel::High);
        assert!(m.is_malicious_contract("0xdead"));
        assert!(!m.is_malicious_contract("0xbeef"));
    }

    // ── URL checking ──────────────────────────────────────────────────

    #[test]
    fn test_check_url_phishing() {
        let mut m = ThreatMonitor::new();
        m.report_phishing("https://evil.com", "bob");
        assert_eq!(m.check_url("https://evil.com"), ThreatLevel::High);
    }

    #[test]
    fn test_check_url_safe() {
        let mut m = ThreatMonitor::new();
        m.add_safe_url("https://safe.io");
        assert_eq!(m.check_url("https://safe.io"), ThreatLevel::None);
    }

    #[test]
    fn test_check_url_unknown() {
        let m = ThreatMonitor::new();
        assert_eq!(m.check_url("https://unknown.org"), ThreatLevel::Low);
    }

    // ── Contract checking ─────────────────────────────────────────────

    #[test]
    fn test_check_contract() {
        let mut m = ThreatMonitor::new();
        m.report_malicious_contract("0xbad", "exploit", ThreatLevel::Critical);
        assert_eq!(m.check_contract("0xbad"), ThreatLevel::Critical);
        assert_eq!(m.check_contract("0xgood"), ThreatLevel::None);
    }

    // ── Transaction checking ──────────────────────────────────────────

    #[test]
    fn test_check_transaction_malicious() {
        let mut m = ThreatMonitor::new();
        m.report_malicious_contract("0xbad", "scam", ThreatLevel::High);
        assert_eq!(
            m.check_transaction("0xbad", 1000, 50),
            ThreatLevel::Critical
        );
    }

    #[test]
    fn test_check_transaction_dust() {
        let m = ThreatMonitor::new();
        assert_eq!(m.check_transaction("0xabc", 5, 1), ThreatLevel::Medium);
    }

    #[test]
    fn test_check_transaction_normal() {
        let m = ThreatMonitor::new();
        assert_eq!(m.check_transaction("0xabc", 1000, 50), ThreatLevel::None);
    }

    // ── Safe URLs ─────────────────────────────────────────────────────

    #[test]
    fn test_add_safe_url() {
        let mut m = ThreatMonitor::new();
        m.add_safe_url("https://safe.io");
        m.add_safe_url("https://safe.io"); // duplicate — ignored
        assert_eq!(m.safe_urls.len(), 1);
        assert!(m.remove_safe_url("https://safe.io"));
        assert!(!m.remove_safe_url("https://safe.io"));
        assert_eq!(m.safe_urls.len(), 0);
    }

    // ── Summary & stats ───────────────────────────────────────────────

    #[test]
    fn test_threat_summary() {
        let mut m = ThreatMonitor::new();
        m.report_threat(sample_threat(
            "t1",
            ThreatType::PhishingUrl,
            ThreatLevel::High,
        ));
        m.report_threat(sample_threat(
            "t2",
            ThreatType::PhishingUrl,
            ThreatLevel::Low,
        ));
        m.report_threat(sample_threat(
            "t3",
            ThreatType::DustAttack,
            ThreatLevel::Medium,
        ));
        let summary = m.threat_summary();
        assert_eq!(summary["PhishingUrl"], 2);
        assert_eq!(summary["DustAttack"], 1);
    }

    #[test]
    fn test_stats() {
        let mut m = ThreatMonitor::new();
        m.report_threat(sample_threat(
            "t1",
            ThreatType::PhishingUrl,
            ThreatLevel::High,
        ));
        m.report_threat(sample_threat(
            "t2",
            ThreatType::DustAttack,
            ThreatLevel::Medium,
        ));
        m.resolve_threat("t1").unwrap();
        m.report_phishing("https://evil.com", "bob");
        m.report_malicious_contract("0xbad", "scam", ThreatLevel::Critical);
        m.add_safe_url("https://safe.io");

        let s = m.stats();
        assert_eq!(s.total_threats, 2);
        assert_eq!(s.active_threats, 1);
        assert_eq!(s.resolved, 1);
        assert_eq!(s.false_positives, 0);
        assert_eq!(s.phishing_urls, 1);
        assert_eq!(s.malicious_contracts, 1);
        assert_eq!(s.safe_urls, 1);
        assert_eq!(s.scan_count, 2);
    }

    // ── Persistence ───────────────────────────────────────────────────

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path("roundtrip.json");

        let mut m = ThreatMonitor::new();
        m.report_threat(sample_threat(
            "t1",
            ThreatType::PhishingUrl,
            ThreatLevel::High,
        ));
        m.report_phishing("https://evil.com", "alice");
        m.report_malicious_contract("0xdead", "rug", ThreatLevel::Critical);
        m.add_safe_url("https://safe.io");
        m.save(&path).unwrap();

        let loaded = ThreatMonitor::load(&path).unwrap();
        assert_eq!(loaded.threats.len(), 1);
        assert_eq!(loaded.phishing_urls.len(), 1);
        assert_eq!(loaded.malicious_contracts.len(), 1);
        assert_eq!(loaded.safe_urls.len(), 1);
        assert_eq!(loaded.scan_count, 1);

        // Clean up.
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_with_metadata_covers_lines_114_117() {
        let t = Threat::new("t1", ThreatType::PhishingUrl, ThreatLevel::High, "src", "tgt", "desc")
            .with_metadata("source_ip", "1.2.3.4")
            .with_metadata("region", "EU");
        assert_eq!(t.metadata.get("source_ip").unwrap(), "1.2.3.4");
        assert_eq!(t.metadata.get("region").unwrap(), "EU");
    }

    #[test]
    fn test_phishing_entry_confirm_covers_lines_141_143() {
        let mut entry = PhishingEntry::new("https://evil.com", "alice");
        assert!(!entry.confirmed);
        entry.confirm();
        assert!(entry.confirmed);
    }

    #[test]
    fn test_malicious_contract_confirm_covers_lines_169_171() {
        let mut mc = MaliciousContract::new("0xbad", "rug pull", ThreatLevel::Critical);
        assert!(!mc.confirmed);
        mc.confirm();
        assert!(mc.confirmed);
    }

    #[test]
    fn test_threat_monitor_default_covers_lines_205_207() {
        let m = ThreatMonitor::default();
        assert!(m.threats.is_empty());
        assert_eq!(m.scan_count, 0);
    }

    #[test]
    fn test_report_threat_prune_covers_line_230() {
        let mut m = ThreatMonitor::new();
        for i in 0..=MAX_THREATS {
            m.report_threat(sample_threat(&format!("t{i}"), ThreatType::DustAttack, ThreatLevel::Low));
        }
        assert_eq!(m.threats.len(), MAX_THREATS);
    }

    #[test]
    fn test_resolve_already_resolved_covers_line_351() {
        let mut m = ThreatMonitor::new();
        m.report_threat(sample_threat("t1", ThreatType::PhishingUrl, ThreatLevel::High));
        m.resolve_threat("t1").unwrap();
        let err = m.resolve_threat("t1").unwrap_err();
        assert!(matches!(err, ThreatMonitorError::AlreadyResolved(_)));
    }

    #[test]
    fn test_false_positive_not_found_covers_line_365() {
        let mut m = ThreatMonitor::new();
        let err = m.false_positive("nonexistent").unwrap_err();
        assert!(matches!(err, ThreatMonitorError::NotFound(_)));
    }

    #[test]
    fn test_false_positive_already_resolved_covers_lines_372_374() {
        let mut m = ThreatMonitor::new();
        m.report_threat(sample_threat("t1", ThreatType::PhishingUrl, ThreatLevel::High));
        m.resolve_threat("t1").unwrap();
        let err = m.false_positive("t1").unwrap_err();
        assert!(matches!(err, ThreatMonitorError::AlreadyResolved(_)));
    }

    #[test]
    fn test_load_or_default_covers_lines_421_423() {
        let path = test_path("load_or_default_missing.json");
        let _ = std::fs::remove_file(&path);
        let m = ThreatMonitor::load_or_default(&path);
        assert!(m.threats.is_empty());
    }

    #[test]
    fn test_recent_threats_covers_lines_372_374() {
        let mut m = ThreatMonitor::new();
        m.report_threat(sample_threat("t1", ThreatType::PhishingUrl, ThreatLevel::High));
        m.report_threat(sample_threat("t2", ThreatType::DustAttack, ThreatLevel::Medium));
        m.report_threat(sample_threat("t3", ThreatType::SocialEngineering, ThreatLevel::Critical));
        let recent = m.recent_threats(2);
        assert_eq!(recent.len(), 2);
        // Most recent first
        assert_eq!(recent[0].id, "t3");
        assert_eq!(recent[1].id, "t2");
    }
}
