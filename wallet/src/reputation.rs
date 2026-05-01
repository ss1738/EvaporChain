//! Address Reputation — risk scoring, scam detection, trust levels.
//!
//! Scores addresses on multiple signals: age, transaction history, known
//! flags (scam/exchange/verified), and pattern analysis. Warns users before
//! they send funds to risky addresses.

use std::path::Path;

use serde::{Deserialize, Serialize};

// ──────────────────────────── Types ──────────────────────────────────────

#[derive(Debug, Clone, thiserror::Error)]
pub enum ReputationError {
    #[error("address not found: {0}")]
    NotFound(String),
    #[error("already flagged: {0}")]
    AlreadyFlagged(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
}

impl From<std::io::Error> for ReputationError {
    fn from(e: std::io::Error) -> Self {
        ReputationError::Io(e.to_string())
    }
}
impl From<serde_json::Error> for ReputationError {
    fn from(e: serde_json::Error) -> Self {
        ReputationError::Json(e.to_string())
    }
}

/// Trust level — overall assessment of an address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TrustLevel {
    /// Known scam / blocklisted.
    Dangerous,
    /// Suspicious patterns detected.
    Suspicious,
    /// Unknown address, no history.
    Unknown,
    /// Some history, not flagged.
    Neutral,
    /// Verified or well-known address.
    Trusted,
    /// Verified by user (whitelisted).
    Verified,
}

impl TrustLevel {
    pub fn label(&self) -> &'static str {
        match self {
            TrustLevel::Dangerous => "DANGEROUS",
            TrustLevel::Suspicious => "Suspicious",
            TrustLevel::Unknown => "Unknown",
            TrustLevel::Neutral => "Neutral",
            TrustLevel::Trusted => "Trusted",
            TrustLevel::Verified => "Verified",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            TrustLevel::Dangerous => "🚫",
            TrustLevel::Suspicious => "⚠️",
            TrustLevel::Unknown => "❓",
            TrustLevel::Neutral => "➖",
            TrustLevel::Trusted => "✅",
            TrustLevel::Verified => "🛡️",
        }
    }

    pub fn should_warn(&self) -> bool {
        matches!(
            self,
            TrustLevel::Dangerous | TrustLevel::Suspicious | TrustLevel::Unknown
        )
    }
}

/// Specific risk flags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskFlag {
    /// Known scam address.
    Scam,
    /// Address involved in phishing.
    Phishing,
    /// Reported by community.
    CommunityReport,
    /// Very new address (< 24h).
    FreshWallet,
    /// High-frequency small transactions (dust attack pattern).
    DustAttack,
    /// Received from known scam.
    TaintedFunds,
    /// Contract with unverified code.
    UnverifiedContract,
    /// Mixer / tumbler.
    Mixer,
    /// Custom user-defined flag.
    Custom(String),
}

impl RiskFlag {
    pub fn label(&self) -> String {
        match self {
            RiskFlag::Scam => "Scam".to_string(),
            RiskFlag::Phishing => "Phishing".to_string(),
            RiskFlag::CommunityReport => "Community Report".to_string(),
            RiskFlag::FreshWallet => "Fresh Wallet".to_string(),
            RiskFlag::DustAttack => "Dust Attack".to_string(),
            RiskFlag::TaintedFunds => "Tainted Funds".to_string(),
            RiskFlag::UnverifiedContract => "Unverified Contract".to_string(),
            RiskFlag::Mixer => "Mixer/Tumbler".to_string(),
            RiskFlag::Custom(s) => format!("Custom: {}", s),
        }
    }

    pub fn severity(&self) -> u8 {
        match self {
            RiskFlag::Scam => 10,
            RiskFlag::Phishing => 9,
            RiskFlag::Mixer => 8,
            RiskFlag::TaintedFunds => 7,
            RiskFlag::DustAttack => 6,
            RiskFlag::UnverifiedContract => 5,
            RiskFlag::CommunityReport => 4,
            RiskFlag::FreshWallet => 3,
            RiskFlag::Custom(_) => 2,
        }
    }
}

/// Address category for reputation context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressType {
    Eoa, // Externally Owned Account
    Contract,
    Exchange,
    Defi,
    Bridge,
    Faucet,
    Validator,
    Unknown,
}

/// An address reputation record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressReputation {
    /// Address (0x hex).
    pub address: String,
    /// Overall trust level.
    pub trust_level: TrustLevel,
    /// Risk flags.
    pub flags: Vec<RiskFlag>,
    /// Risk score (0-100, higher = more risky).
    pub risk_score: u8,
    /// Address type.
    pub address_type: AddressType,
    /// Optional label.
    pub label: Option<String>,
    /// First seen timestamp.
    pub first_seen: Option<String>,
    /// Number of known transactions.
    pub tx_count: u64,
    /// Notes.
    pub notes: Vec<String>,
    /// Last updated.
    pub updated_at: String,
}

/// Risk assessment result returned to the caller.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskAssessment {
    pub address: String,
    pub trust_level: TrustLevel,
    pub risk_score: u8,
    pub warnings: Vec<String>,
    pub should_block: bool,
    pub should_warn: bool,
}

// ──────────────────────────── Store ──────────────────────────────────────

/// Persistent reputation database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationStore {
    pub records: Vec<AddressReputation>,
    /// Risk score threshold for blocking (default 80).
    pub block_threshold: u8,
    /// Risk score threshold for warning (default 40).
    pub warn_threshold: u8,
}

impl ReputationStore {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            block_threshold: 80,
            warn_threshold: 40,
        }
    }

    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ReputationError> {
        let data = std::fs::read_to_string(path)?;
        let store: ReputationStore = serde_json::from_str(&data)?;
        Ok(store)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), ReputationError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Look up an address.
    pub fn get(&self, address: &str) -> Option<&AddressReputation> {
        let addr = address.to_lowercase();
        self.records
            .iter()
            .find(|r| r.address.to_lowercase() == addr)
    }

    /// Look up mutable.
    pub fn get_mut(&mut self, address: &str) -> Option<&mut AddressReputation> {
        let addr = address.to_lowercase();
        self.records
            .iter_mut()
            .find(|r| r.address.to_lowercase() == addr)
    }

    /// Add or update an address record.
    pub fn upsert(&mut self, rep: AddressReputation) {
        if let Some(existing) = self.get_mut(&rep.address) {
            *existing = rep;
        } else {
            self.records.push(rep);
        }
    }

    /// Flag an address with a risk flag.
    pub fn flag(
        &mut self,
        address: &str,
        flag: RiskFlag,
        note: Option<&str>,
    ) -> Result<(), ReputationError> {
        let addr = address.to_lowercase();
        if let Some(record) = self.get_mut(&addr) {
            if !record.flags.contains(&flag) {
                record.flags.push(flag);
            }
            if let Some(n) = note {
                record.notes.push(n.to_string());
            }
            record.risk_score = Self::compute_score(&record.flags);
            record.trust_level = Self::score_to_trust(record.risk_score);
            record.updated_at = chrono::Utc::now().to_rfc3339();
            Ok(())
        } else {
            // Create new record with flag
            let flags = vec![flag];
            let score = Self::compute_score(&flags);
            let trust = Self::score_to_trust(score);
            let mut notes = Vec::new();
            if let Some(n) = note {
                notes.push(n.to_string());
            }
            let record = AddressReputation {
                address: addr,
                trust_level: trust,
                flags,
                risk_score: score,
                address_type: AddressType::Unknown,
                label: None,
                first_seen: None,
                tx_count: 0,
                notes,
                updated_at: chrono::Utc::now().to_rfc3339(),
            };
            self.records.push(record);
            Ok(())
        }
    }

    /// Remove a flag from an address.
    pub fn unflag(&mut self, address: &str, flag: &RiskFlag) -> Result<(), ReputationError> {
        let record = self
            .get_mut(address)
            .ok_or_else(|| ReputationError::NotFound(address.to_string()))?;
        record.flags.retain(|f| f != flag);
        record.risk_score = Self::compute_score(&record.flags);
        record.trust_level = Self::score_to_trust(record.risk_score);
        record.updated_at = chrono::Utc::now().to_rfc3339();
        Ok(())
    }

    /// Verify (whitelist) an address.
    pub fn verify(&mut self, address: &str, label: Option<&str>) {
        let addr = address.to_lowercase();
        if let Some(record) = self.get_mut(&addr) {
            record.trust_level = TrustLevel::Verified;
            record.risk_score = 0;
            record.flags.clear();
            if let Some(l) = label {
                record.label = Some(l.to_string());
            }
            record.updated_at = chrono::Utc::now().to_rfc3339();
        } else {
            let record = AddressReputation {
                address: addr,
                trust_level: TrustLevel::Verified,
                flags: vec![],
                risk_score: 0,
                address_type: AddressType::Unknown,
                label: label.map(|s| s.to_string()),
                first_seen: None,
                tx_count: 0,
                notes: vec![],
                updated_at: chrono::Utc::now().to_rfc3339(),
            };
            self.records.push(record);
        }
    }

    /// Assess risk for an address (creates a RiskAssessment).
    pub fn assess(&self, address: &str) -> RiskAssessment {
        match self.get(address) {
            Some(record) => {
                let mut warnings = Vec::new();
                for flag in &record.flags {
                    warnings.push(format!("{} (severity: {})", flag.label(), flag.severity()));
                }
                if record.trust_level == TrustLevel::Dangerous {
                    warnings.insert(0, "ADDRESS IS FLAGGED AS DANGEROUS".to_string());
                }
                RiskAssessment {
                    address: address.to_string(),
                    trust_level: record.trust_level,
                    risk_score: record.risk_score,
                    should_block: record.trust_level == TrustLevel::Dangerous
                        || record.risk_score >= self.block_threshold,
                    should_warn: record.trust_level.should_warn(),
                    warnings,
                }
            }
            None => {
                // Unknown address
                RiskAssessment {
                    address: address.to_string(),
                    trust_level: TrustLevel::Unknown,
                    risk_score: 30, // Mild baseline risk for unknown
                    should_block: false,
                    should_warn: true,
                    warnings: vec!["Address has no reputation history".to_string()],
                }
            }
        }
    }

    /// List all dangerous addresses.
    pub fn dangerous(&self) -> Vec<&AddressReputation> {
        self.records
            .iter()
            .filter(|r| r.trust_level == TrustLevel::Dangerous)
            .collect()
    }

    /// List all flagged addresses.
    pub fn flagged(&self) -> Vec<&AddressReputation> {
        self.records
            .iter()
            .filter(|r| !r.flags.is_empty())
            .collect()
    }

    /// List verified addresses.
    pub fn verified(&self) -> Vec<&AddressReputation> {
        self.records
            .iter()
            .filter(|r| r.trust_level == TrustLevel::Verified)
            .collect()
    }

    /// Search records by address or label.
    pub fn search(&self, query: &str) -> Vec<&AddressReputation> {
        let q = query.to_lowercase();
        self.records
            .iter()
            .filter(|r| {
                r.address.to_lowercase().contains(&q)
                    || r.label
                        .as_ref()
                        .is_some_and(|l| l.to_lowercase().contains(&q))
            })
            .collect()
    }

    /// Total records.
    pub fn count(&self) -> usize {
        self.records.len()
    }

    /// Set blocking threshold.
    pub fn set_block_threshold(&mut self, threshold: u8) {
        self.block_threshold = threshold;
    }

    /// Set warning threshold.
    pub fn set_warn_threshold(&mut self, threshold: u8) {
        self.warn_threshold = threshold;
    }

    // ── Internal ──

    fn compute_score(flags: &[RiskFlag]) -> u8 {
        let total: u16 = flags.iter().map(|f| f.severity() as u16).sum();
        (total.min(100)) as u8
    }

    fn score_to_trust(score: u8) -> TrustLevel {
        if score >= 15 {
            TrustLevel::Dangerous
        } else if score >= 8 {
            TrustLevel::Suspicious
        } else if score >= 4 {
            TrustLevel::Neutral
        } else {
            TrustLevel::Unknown
        }
    }
}

impl Default for ReputationStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Default path.
pub fn default_reputation_path() -> std::path::PathBuf {
    crate::config::default_data_dir().join("reputation.json")
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store() -> ReputationStore {
        let mut store = ReputationStore::new();
        store
            .flag("0xscammer", RiskFlag::Scam, Some("reported in Discord"))
            .unwrap();
        store.flag("0xscammer", RiskFlag::Phishing, None).unwrap();
        store.verify("0xexchange", Some("EvapSwap DEX"));
        store.flag("0xfresh", RiskFlag::FreshWallet, None).unwrap();
        store
    }

    #[test]
    fn test_flag_address() {
        let store = make_store();
        let scammer = store.get("0xscammer").unwrap();
        assert_eq!(scammer.flags.len(), 2);
        assert!(scammer.risk_score >= 15); // Scam(10) + Phishing(9)
        assert_eq!(scammer.trust_level, TrustLevel::Dangerous);
    }

    #[test]
    fn test_verify_address() {
        let store = make_store();
        let exchange = store.get("0xexchange").unwrap();
        assert_eq!(exchange.trust_level, TrustLevel::Verified);
        assert_eq!(exchange.risk_score, 0);
        assert_eq!(exchange.label.as_deref(), Some("EvapSwap DEX"));
    }

    #[test]
    fn test_fresh_wallet_mild_risk() {
        let store = make_store();
        let fresh = store.get("0xfresh").unwrap();
        assert_eq!(fresh.flags.len(), 1);
        assert_eq!(fresh.risk_score, 3); // FreshWallet severity = 3
                                         // Score 3 → Unknown trust
        assert_eq!(fresh.trust_level, TrustLevel::Unknown);
    }

    #[test]
    fn test_assess_scammer() {
        let store = make_store();
        let assessment = store.assess("0xscammer");
        assert_eq!(assessment.trust_level, TrustLevel::Dangerous);
        assert!(assessment.should_block);
        assert!(assessment.should_warn);
        assert!(!assessment.warnings.is_empty());
    }

    #[test]
    fn test_assess_verified() {
        let store = make_store();
        let assessment = store.assess("0xexchange");
        assert_eq!(assessment.trust_level, TrustLevel::Verified);
        assert!(!assessment.should_block);
        assert!(!assessment.should_warn);
    }

    #[test]
    fn test_assess_unknown() {
        let store = make_store();
        let assessment = store.assess("0xnobody");
        assert_eq!(assessment.trust_level, TrustLevel::Unknown);
        assert!(!assessment.should_block);
        assert!(assessment.should_warn);
        assert_eq!(assessment.risk_score, 30);
    }

    #[test]
    fn test_unflag() {
        let mut store = make_store();
        store.unflag("0xscammer", &RiskFlag::Phishing).unwrap();
        let scammer = store.get("0xscammer").unwrap();
        assert_eq!(scammer.flags.len(), 1);
        assert_eq!(scammer.risk_score, 10); // Only Scam remains
    }

    #[test]
    fn test_unflag_nonexistent() {
        let mut store = make_store();
        assert!(store.unflag("0xnobody", &RiskFlag::Scam).is_err());
    }

    #[test]
    fn test_dangerous_list() {
        let store = make_store();
        let dangerous = store.dangerous();
        assert_eq!(dangerous.len(), 1);
        assert_eq!(dangerous[0].address, "0xscammer");
    }

    #[test]
    fn test_flagged_list() {
        let store = make_store();
        let flagged = store.flagged();
        assert_eq!(flagged.len(), 2); // scammer + fresh
    }

    #[test]
    fn test_verified_list() {
        let store = make_store();
        let verified = store.verified();
        assert_eq!(verified.len(), 1);
        assert_eq!(verified[0].label.as_deref(), Some("EvapSwap DEX"));
    }

    #[test]
    fn test_search() {
        let store = make_store();
        let results = store.search("evapswap");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_search_by_address() {
        let store = make_store();
        let results = store.search("0xscam");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_case_insensitive() {
        let mut store = ReputationStore::new();
        store.flag("0xABC", RiskFlag::Scam, None).unwrap();
        assert!(store.get("0xabc").is_some());
        assert!(store.get("0xABC").is_some());
    }

    #[test]
    fn test_upsert() {
        let mut store = make_store();
        let count_before = store.count();
        let rep = AddressReputation {
            address: "0xscammer".to_string(),
            trust_level: TrustLevel::Neutral,
            flags: vec![],
            risk_score: 0,
            address_type: AddressType::Eoa,
            label: Some("reformed".into()),
            first_seen: None,
            tx_count: 100,
            notes: vec![],
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        store.upsert(rep);
        assert_eq!(store.count(), count_before); // Updated, not added
        assert_eq!(
            store.get("0xscammer").unwrap().label.as_deref(),
            Some("reformed")
        );
    }

    #[test]
    fn test_trust_level_ordering() {
        assert!(TrustLevel::Dangerous < TrustLevel::Suspicious);
        assert!(TrustLevel::Suspicious < TrustLevel::Unknown);
        assert!(TrustLevel::Unknown < TrustLevel::Neutral);
        assert!(TrustLevel::Neutral < TrustLevel::Trusted);
        assert!(TrustLevel::Trusted < TrustLevel::Verified);
    }

    #[test]
    fn test_trust_level_should_warn() {
        assert!(TrustLevel::Dangerous.should_warn());
        assert!(TrustLevel::Suspicious.should_warn());
        assert!(TrustLevel::Unknown.should_warn());
        assert!(!TrustLevel::Neutral.should_warn());
        assert!(!TrustLevel::Trusted.should_warn());
        assert!(!TrustLevel::Verified.should_warn());
    }

    #[test]
    fn test_risk_flag_severity() {
        assert!(RiskFlag::Scam.severity() > RiskFlag::FreshWallet.severity());
        assert!(RiskFlag::Phishing.severity() > RiskFlag::CommunityReport.severity());
    }

    #[test]
    fn test_set_thresholds() {
        let mut store = ReputationStore::new();
        store.set_block_threshold(90);
        store.set_warn_threshold(50);
        assert_eq!(store.block_threshold, 90);
        assert_eq!(store.warn_threshold, 50);
    }

    #[test]
    fn test_notes_preserved() {
        let store = make_store();
        let scammer = store.get("0xscammer").unwrap();
        assert_eq!(scammer.notes.len(), 1);
        assert_eq!(scammer.notes[0], "reported in Discord");
    }

    #[test]
    fn test_duplicate_flag_not_added() {
        let mut store = make_store();
        store.flag("0xscammer", RiskFlag::Scam, None).unwrap();
        let scammer = store.get("0xscammer").unwrap();
        // Should still have only 2 flags, not 3
        assert_eq!(scammer.flags.len(), 2);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let dir = std::env::temp_dir().join("evap_rep_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("reputation.json");

        let store = make_store();
        store.save(&path).unwrap();

        let loaded = ReputationStore::load(&path).unwrap();
        assert_eq!(loaded.count(), 3);
        assert!(loaded.get("0xscammer").is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_score_cap_at_100() {
        let mut store = ReputationStore::new();
        // Add many severe flags — score should cap at 100
        store.flag("0xbad", RiskFlag::Scam, None).unwrap();
        store.flag("0xbad", RiskFlag::Phishing, None).unwrap();
        store.flag("0xbad", RiskFlag::Mixer, None).unwrap();
        store.flag("0xbad", RiskFlag::TaintedFunds, None).unwrap();
        store.flag("0xbad", RiskFlag::DustAttack, None).unwrap();
        store
            .flag("0xbad", RiskFlag::UnverifiedContract, None)
            .unwrap();
        store
            .flag("0xbad", RiskFlag::CommunityReport, None)
            .unwrap();
        let record = store.get("0xbad").unwrap();
        assert!(record.risk_score <= 100);
    }
}
