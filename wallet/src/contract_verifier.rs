use std::collections::HashMap;
use std::path::Path;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ContractVerifierError {
    #[error("contract not found: {0}")]
    ContractNotFound(String),
    #[error("contract already registered: {0}")]
    AlreadyRegistered(String),
    #[error("verification failed: {0}")]
    VerificationFailed(String),
    #[error("source mismatch: {0}")]
    SourceMismatch(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Parse(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum VerificationStatus {
    Unverified,
    Verified,
    Failed,
    Partial,
    Outdated,
}

impl Default for VerificationStatus {
    fn default() -> Self {
        Self::Unverified
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CompilerVersion {
    V1,
    V2,
    V3,
    Custom(String),
}

impl Default for CompilerVersion {
    fn default() -> Self {
        Self::V1
    }
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractSource {
    pub contract_address: String,
    pub source_code: String,
    pub source_hash: String,
    pub compiler_version: CompilerVersion,
    pub registered_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationRecord {
    pub contract_address: String,
    pub deployed_hash: String,
    pub source_hash: String,
    pub matches: bool,
    pub verified_at: String,
    pub diff_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BytecodeDiff {
    pub offset: usize,
    pub expected: String,
    pub actual: String,
    pub length: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub contract_address: String,
    pub status: VerificationStatus,
    pub source_hash: String,
    pub deployed_hash: String,
    pub diffs: Vec<BytecodeDiff>,
    pub verified_at: String,
    pub compiler_version: CompilerVersion,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VerifierStats {
    pub total_contracts: usize,
    pub verified: usize,
    pub failed: usize,
    pub unverified: usize,
    pub partial: usize,
    pub total_verifications: usize,
    pub last_verification: Option<String>,
}

// ---------------------------------------------------------------------------
// ContractVerifier
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContractVerifier {
    pub sources: HashMap<String, ContractSource>,
    pub records: Vec<VerificationRecord>,
    pub reports: Vec<VerificationReport>,
}

fn compute_hash(data: &str) -> String {
    blake3::hash(data.as_bytes()).to_hex().to_string()
}

impl ContractVerifier {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_source(
        &mut self,
        address: &str,
        source_code: &str,
        compiler: CompilerVersion,
    ) -> Result<(), ContractVerifierError> {
        if self.sources.contains_key(address) {
            return Err(ContractVerifierError::AlreadyRegistered(
                address.to_string(),
            ));
        }
        let source_hash = compute_hash(source_code);
        self.sources.insert(
            address.to_string(),
            ContractSource {
                contract_address: address.to_string(),
                source_code: source_code.to_string(),
                source_hash,
                compiler_version: compiler,
                registered_at: Utc::now().to_rfc3339(),
            },
        );
        Ok(())
    }

    pub fn unregister_source(
        &mut self,
        address: &str,
    ) -> Result<ContractSource, ContractVerifierError> {
        self.sources
            .remove(address)
            .ok_or_else(|| ContractVerifierError::ContractNotFound(address.to_string()))
    }

    pub fn update_source(
        &mut self,
        address: &str,
        new_source: &str,
    ) -> Result<(), ContractVerifierError> {
        let source = self
            .sources
            .get_mut(address)
            .ok_or_else(|| ContractVerifierError::ContractNotFound(address.to_string()))?;
        source.source_code = new_source.to_string();
        source.source_hash = compute_hash(new_source);

        // Mark any existing reports for this address as Outdated.
        for report in &mut self.reports {
            if report.contract_address == address {
                report.status = VerificationStatus::Outdated;
            }
        }
        Ok(())
    }

    pub fn verify_contract(
        &mut self,
        address: &str,
        deployed_bytecode: &str,
    ) -> Result<VerificationReport, ContractVerifierError> {
        let source = self
            .sources
            .get(address)
            .ok_or_else(|| ContractVerifierError::ContractNotFound(address.to_string()))?;

        let deployed_hash = compute_hash(deployed_bytecode);
        let source_hash = source.source_hash.clone();
        let compiler_version = source.compiler_version.clone();
        let now = Utc::now().to_rfc3339();

        let matches = deployed_hash == source_hash;

        let (status, diffs, diff_summary) = if matches {
            (VerificationStatus::Verified, vec![], None)
        } else {
            let diff = BytecodeDiff {
                offset: 0,
                expected: source_hash.clone(),
                actual: deployed_hash.clone(),
                length: deployed_bytecode.len(),
            };
            (
                VerificationStatus::Failed,
                vec![diff],
                Some("hashes differ".to_string()),
            )
        };

        let record = VerificationRecord {
            contract_address: address.to_string(),
            deployed_hash: deployed_hash.clone(),
            source_hash: source_hash.clone(),
            matches,
            verified_at: now.clone(),
            diff_summary,
        };
        self.records.push(record);

        let report = VerificationReport {
            contract_address: address.to_string(),
            status,
            source_hash,
            deployed_hash,
            diffs,
            verified_at: now,
            compiler_version,
        };
        self.reports.push(report.clone());
        Ok(report)
    }

    pub fn get_source(&self, address: &str) -> Option<&ContractSource> {
        self.sources.get(address)
    }

    pub fn get_latest_report(&self, address: &str) -> Option<&VerificationReport> {
        self.reports
            .iter()
            .rev()
            .find(|r| r.contract_address == address)
    }

    pub fn verification_history(&self, address: &str) -> Vec<&VerificationRecord> {
        self.records
            .iter()
            .filter(|r| r.contract_address == address)
            .collect()
    }

    pub fn verified_contracts(&self) -> Vec<&ContractSource> {
        self.sources
            .values()
            .filter(|src| {
                self.reports.iter().rev().any(|r| {
                    r.contract_address == src.contract_address
                        && r.status == VerificationStatus::Verified
                })
            })
            .collect()
    }

    pub fn unverified_contracts(&self) -> Vec<&ContractSource> {
        self.sources
            .values()
            .filter(|src| {
                !self.reports.iter().any(|r| {
                    r.contract_address == src.contract_address
                        && r.status == VerificationStatus::Verified
                })
            })
            .collect()
    }

    pub fn failed_contracts(&self) -> Vec<&VerificationReport> {
        self.reports
            .iter()
            .filter(|r| r.status == VerificationStatus::Failed)
            .collect()
    }

    pub fn search_contracts(&self, query: &str) -> Vec<&ContractSource> {
        self.sources
            .values()
            .filter(|src| src.contract_address.contains(query))
            .collect()
    }

    pub fn stats(&self) -> VerifierStats {
        let total_contracts = self.sources.len();

        let mut verified_addrs = std::collections::HashSet::new();
        let mut failed_addrs = std::collections::HashSet::new();
        let mut partial_addrs = std::collections::HashSet::new();

        // Use the latest report per address to determine status.
        let mut latest: HashMap<&str, &VerificationReport> = HashMap::new();
        for report in &self.reports {
            latest.insert(&report.contract_address, report);
        }

        for (_, report) in &latest {
            match report.status {
                VerificationStatus::Verified => {
                    verified_addrs.insert(&report.contract_address);
                }
                VerificationStatus::Failed => {
                    failed_addrs.insert(&report.contract_address);
                }
                VerificationStatus::Partial => {
                    partial_addrs.insert(&report.contract_address);
                }
                _ => {}
            }
        }

        let verified = verified_addrs.len();
        let failed = failed_addrs.len();
        let partial = partial_addrs.len();
        let unverified = total_contracts.saturating_sub(verified + failed + partial);

        let last_verification = self.records.last().map(|r| r.verified_at.clone());

        VerifierStats {
            total_contracts,
            verified,
            failed,
            unverified,
            partial,
            total_verifications: self.records.len(),
            last_verification,
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), ContractVerifierError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, ContractVerifierError> {
        let data = std::fs::read_to_string(path)?;
        let verifier: Self = serde_json::from_str(&data)?;
        Ok(verifier)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "contract_verifier_test_{}.json",
            std::process::id()
        ))
    }

    #[test]
    fn test_register_source() {
        let mut v = ContractVerifier::new();
        v.register_source("0xABC", "fn main() {}", CompilerVersion::V1)
            .unwrap();
        assert!(v.get_source("0xABC").is_some());
    }

    #[test]
    fn test_register_duplicate() {
        let mut v = ContractVerifier::new();
        v.register_source("0xABC", "code", CompilerVersion::V1)
            .unwrap();
        let err = v
            .register_source("0xABC", "code", CompilerVersion::V1)
            .unwrap_err();
        assert!(matches!(err, ContractVerifierError::AlreadyRegistered(_)));
    }

    #[test]
    fn test_unregister_source() {
        let mut v = ContractVerifier::new();
        v.register_source("0xABC", "code", CompilerVersion::V2)
            .unwrap();
        let removed = v.unregister_source("0xABC").unwrap();
        assert_eq!(removed.contract_address, "0xABC");
        assert!(v.get_source("0xABC").is_none());
    }

    #[test]
    fn test_unregister_not_found() {
        let mut v = ContractVerifier::new();
        let err = v.unregister_source("0xNOPE").unwrap_err();
        assert!(matches!(err, ContractVerifierError::ContractNotFound(_)));
    }

    #[test]
    fn test_update_source() {
        let mut v = ContractVerifier::new();
        v.register_source("0xABC", "old_code", CompilerVersion::V1)
            .unwrap();
        let old_hash = v.get_source("0xABC").unwrap().source_hash.clone();
        v.update_source("0xABC", "new_code").unwrap();
        let new_hash = v.get_source("0xABC").unwrap().source_hash.clone();
        assert_ne!(old_hash, new_hash);
    }

    #[test]
    fn test_update_source_not_found() {
        let mut v = ContractVerifier::new();
        let err = v.update_source("0xNOPE", "code").unwrap_err();
        assert!(matches!(err, ContractVerifierError::ContractNotFound(_)));
    }

    #[test]
    fn test_update_marks_outdated() {
        let mut v = ContractVerifier::new();
        v.register_source("0xABC", "code", CompilerVersion::V1)
            .unwrap();
        v.verify_contract("0xABC", "code").unwrap();
        v.update_source("0xABC", "new_code").unwrap();
        let report = v.get_latest_report("0xABC").unwrap();
        assert_eq!(report.status, VerificationStatus::Outdated);
    }

    #[test]
    fn test_verify_matching_bytecode() {
        let mut v = ContractVerifier::new();
        let source = "fn main() {}";
        v.register_source("0xABC", source, CompilerVersion::V1)
            .unwrap();
        let report = v.verify_contract("0xABC", source).unwrap();
        assert_eq!(report.status, VerificationStatus::Verified);
        assert!(report.diffs.is_empty());
    }

    #[test]
    fn test_verify_mismatched_bytecode() {
        let mut v = ContractVerifier::new();
        v.register_source("0xABC", "source_code", CompilerVersion::V1)
            .unwrap();
        let report = v.verify_contract("0xABC", "different_bytecode").unwrap();
        assert_eq!(report.status, VerificationStatus::Failed);
        assert_eq!(report.diffs.len(), 1);
    }

    #[test]
    fn test_verify_not_found() {
        let mut v = ContractVerifier::new();
        let err = v.verify_contract("0xNOPE", "bytes").unwrap_err();
        assert!(matches!(err, ContractVerifierError::ContractNotFound(_)));
    }

    #[test]
    fn test_verification_record_created() {
        let mut v = ContractVerifier::new();
        v.register_source("0xABC", "code", CompilerVersion::V1)
            .unwrap();
        v.verify_contract("0xABC", "code").unwrap();
        let history = v.verification_history("0xABC");
        assert_eq!(history.len(), 1);
        assert!(history[0].matches);
    }

    #[test]
    fn test_verification_history_multiple() {
        let mut v = ContractVerifier::new();
        v.register_source("0xABC", "code", CompilerVersion::V1)
            .unwrap();
        v.verify_contract("0xABC", "code").unwrap();
        v.verify_contract("0xABC", "other").unwrap();
        let history = v.verification_history("0xABC");
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn test_verified_contracts() {
        let mut v = ContractVerifier::new();
        v.register_source("0xA", "code_a", CompilerVersion::V1)
            .unwrap();
        v.register_source("0xB", "code_b", CompilerVersion::V2)
            .unwrap();
        v.verify_contract("0xA", "code_a").unwrap();
        v.verify_contract("0xB", "wrong").unwrap();
        let verified = v.verified_contracts();
        assert_eq!(verified.len(), 1);
        assert_eq!(verified[0].contract_address, "0xA");
    }

    #[test]
    fn test_unverified_contracts() {
        let mut v = ContractVerifier::new();
        v.register_source("0xA", "code_a", CompilerVersion::V1)
            .unwrap();
        v.register_source("0xB", "code_b", CompilerVersion::V2)
            .unwrap();
        // Only verify 0xA
        v.verify_contract("0xA", "code_a").unwrap();
        let unverified = v.unverified_contracts();
        assert_eq!(unverified.len(), 1);
        assert_eq!(unverified[0].contract_address, "0xB");
    }

    #[test]
    fn test_failed_contracts() {
        let mut v = ContractVerifier::new();
        v.register_source("0xA", "code_a", CompilerVersion::V1)
            .unwrap();
        v.verify_contract("0xA", "wrong_bytes").unwrap();
        let failed = v.failed_contracts();
        assert_eq!(failed.len(), 1);
    }

    #[test]
    fn test_search_contracts() {
        let mut v = ContractVerifier::new();
        v.register_source("0xABCDEF", "c1", CompilerVersion::V1)
            .unwrap();
        v.register_source("0x123456", "c2", CompilerVersion::V1)
            .unwrap();
        v.register_source("0xAB9999", "c3", CompilerVersion::V1)
            .unwrap();
        let results = v.search_contracts("0xAB");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_search_no_results() {
        let mut v = ContractVerifier::new();
        v.register_source("0xABC", "code", CompilerVersion::V1)
            .unwrap();
        let results = v.search_contracts("0xZZZ");
        assert!(results.is_empty());
    }

    #[test]
    fn test_stats_empty() {
        let v = ContractVerifier::new();
        let stats = v.stats();
        assert_eq!(stats.total_contracts, 0);
        assert_eq!(stats.verified, 0);
        assert_eq!(stats.failed, 0);
        assert_eq!(stats.total_verifications, 0);
        assert!(stats.last_verification.is_none());
    }

    #[test]
    fn test_stats_populated() {
        let mut v = ContractVerifier::new();
        v.register_source("0xA", "code_a", CompilerVersion::V1)
            .unwrap();
        v.register_source("0xB", "code_b", CompilerVersion::V2)
            .unwrap();
        v.register_source("0xC", "code_c", CompilerVersion::V3)
            .unwrap();
        v.verify_contract("0xA", "code_a").unwrap();
        v.verify_contract("0xB", "wrong").unwrap();
        let stats = v.stats();
        assert_eq!(stats.total_contracts, 3);
        assert_eq!(stats.verified, 1);
        assert_eq!(stats.failed, 1);
        assert_eq!(stats.unverified, 1);
        assert_eq!(stats.total_verifications, 2);
        assert!(stats.last_verification.is_some());
    }

    #[test]
    fn test_save_and_load() {
        let path = test_path();
        let mut v = ContractVerifier::new();
        v.register_source("0xABC", "code", CompilerVersion::V1)
            .unwrap();
        v.verify_contract("0xABC", "code").unwrap();
        v.save(&path).unwrap();

        let loaded = ContractVerifier::load(&path).unwrap();
        assert!(loaded.get_source("0xABC").is_some());
        assert_eq!(loaded.records.len(), 1);
        assert_eq!(loaded.reports.len(), 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = std::env::temp_dir().join(format!(
            "contract_verifier_missing_{}.json",
            std::process::id()
        ));
        let v = ContractVerifier::load_or_default(&path);
        assert_eq!(v.sources.len(), 0);
    }

    #[test]
    fn test_get_latest_report() {
        let mut v = ContractVerifier::new();
        v.register_source("0xA", "code", CompilerVersion::V1)
            .unwrap();
        v.verify_contract("0xA", "wrong1").unwrap();
        v.verify_contract("0xA", "code").unwrap();
        let latest = v.get_latest_report("0xA").unwrap();
        assert_eq!(latest.status, VerificationStatus::Verified);
    }

    #[test]
    fn test_compiler_custom_version() {
        let mut v = ContractVerifier::new();
        v.register_source(
            "0xA",
            "code",
            CompilerVersion::Custom("nightly-2025".to_string()),
        )
        .unwrap();
        let src = v.get_source("0xA").unwrap();
        assert_eq!(
            src.compiler_version,
            CompilerVersion::Custom("nightly-2025".to_string())
        );
    }
}
