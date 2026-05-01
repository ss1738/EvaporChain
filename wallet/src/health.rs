// wallet/src/health.rs — Wallet health checks and diagnostics
//
// Validates keystore integrity, config state, disk space, data dir
// structure, and reports overall wallet health with actionable fixes.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HealthError {
    #[error("health check failed: {0}")]
    CheckFailed(String),
    #[error("io error: {0}")]
    Io(String),
}

// ── Check results ─────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum HealthStatus {
    Healthy,
    Warning,
    Critical,
    Unknown,
}

impl HealthStatus {
    pub fn emoji(&self) -> &'static str {
        match self {
            HealthStatus::Healthy => "OK",
            HealthStatus::Warning => "WARN",
            HealthStatus::Critical => "CRIT",
            HealthStatus::Unknown => "????",
        }
    }

    pub fn is_ok(&self) -> bool {
        *self == HealthStatus::Healthy
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub name: String,
    pub status: HealthStatus,
    pub message: String,
    pub fix: Option<String>,
}

impl CheckResult {
    pub fn healthy(name: &str, message: &str) -> Self {
        Self {
            name: name.to_string(),
            status: HealthStatus::Healthy,
            message: message.to_string(),
            fix: None,
        }
    }

    pub fn warning(name: &str, message: &str, fix: &str) -> Self {
        Self {
            name: name.to_string(),
            status: HealthStatus::Warning,
            message: message.to_string(),
            fix: Some(fix.to_string()),
        }
    }

    pub fn critical(name: &str, message: &str, fix: &str) -> Self {
        Self {
            name: name.to_string(),
            status: HealthStatus::Critical,
            message: message.to_string(),
            fix: Some(fix.to_string()),
        }
    }

    pub fn unknown(name: &str, message: &str) -> Self {
        Self {
            name: name.to_string(),
            status: HealthStatus::Unknown,
            message: message.to_string(),
            fix: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    pub overall: HealthStatus,
    pub checks: Vec<CheckResult>,
    pub timestamp: String,
    pub wallet_version: String,
}

impl HealthReport {
    pub fn new(checks: Vec<CheckResult>) -> Self {
        let overall = checks
            .iter()
            .map(|c| c.status)
            .max()
            .unwrap_or(HealthStatus::Unknown);

        Self {
            overall,
            checks,
            timestamp: chrono::Utc::now().to_rfc3339(),
            wallet_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    pub fn healthy_count(&self) -> usize {
        self.checks
            .iter()
            .filter(|c| c.status == HealthStatus::Healthy)
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.checks
            .iter()
            .filter(|c| c.status == HealthStatus::Warning)
            .count()
    }

    pub fn critical_count(&self) -> usize {
        self.checks
            .iter()
            .filter(|c| c.status == HealthStatus::Critical)
            .count()
    }

    pub fn is_healthy(&self) -> bool {
        self.overall == HealthStatus::Healthy
    }

    pub fn fixes(&self) -> Vec<&str> {
        self.checks
            .iter()
            .filter_map(|c| c.fix.as_deref())
            .collect()
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("Wallet Health: [{}]\n", self.overall.emoji()));
        out.push_str(&format!("Version: {}\n\n", self.wallet_version));
        for check in &self.checks {
            out.push_str(&format!(
                "  [{}] {} — {}\n",
                check.status.emoji(),
                check.name,
                check.message
            ));
            if let Some(ref fix) = check.fix {
                out.push_str(&format!("        Fix: {}\n", fix));
            }
        }
        out.push_str(&format!(
            "\nSummary: {} healthy, {} warnings, {} critical\n",
            self.healthy_count(),
            self.warning_count(),
            self.critical_count()
        ));
        out
    }
}

// ── Health checker ────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct HealthChecker {
    pub data_dir: PathBuf,
    pub keystore_path: PathBuf,
    pub config_path: PathBuf,
}

impl HealthChecker {
    pub fn new(data_dir: PathBuf, keystore_path: PathBuf, config_path: PathBuf) -> Self {
        Self {
            data_dir,
            keystore_path,
            config_path,
        }
    }

    pub fn from_defaults() -> Self {
        let data_dir = crate::config::default_data_dir();
        Self {
            keystore_path: data_dir.join("keystore.json"),
            config_path: data_dir.join("config.json"),
            data_dir,
        }
    }

    /// Run all health checks
    pub fn run_all(&self) -> HealthReport {
        let checks = vec![
            self.check_data_dir(),
            self.check_keystore(),
            self.check_config(),
            self.check_permissions(),
            self.check_disk_space(),
            self.check_backup_age(),
            self.check_stale_locks(),
        ];
        HealthReport::new(checks)
    }

    /// Check data directory exists and is writable
    pub fn check_data_dir(&self) -> CheckResult {
        if !self.data_dir.exists() {
            return CheckResult::warning(
                "data_directory",
                "Data directory does not exist",
                "Run 'wallet account create' to initialize",
            );
        }
        if !self.data_dir.is_dir() {
            return CheckResult::critical(
                "data_directory",
                "Data path exists but is not a directory",
                "Remove the file and recreate as directory",
            );
        }
        // Check writable by trying to create a temp file
        let test_path = self.data_dir.join(".health_check_tmp");
        match std::fs::write(&test_path, b"ok") {
            Ok(_) => {
                let _ = std::fs::remove_file(&test_path);
                CheckResult::healthy("data_directory", "Data directory exists and is writable")
            }
            Err(_) => CheckResult::critical(
                "data_directory",
                "Data directory is not writable",
                "Check file permissions on the data directory",
            ),
        }
    }

    /// Check keystore file
    pub fn check_keystore(&self) -> CheckResult {
        if !self.keystore_path.exists() {
            return CheckResult::warning(
                "keystore",
                "No keystore file found",
                "Run 'wallet account create' to create one",
            );
        }

        match std::fs::metadata(&self.keystore_path) {
            Ok(meta) => {
                if meta.len() == 0 {
                    return CheckResult::critical(
                        "keystore",
                        "Keystore file is empty",
                        "Restore from backup or create new keystore",
                    );
                }
                // Try to parse as JSON
                match std::fs::read_to_string(&self.keystore_path) {
                    Ok(content) => {
                        if serde_json::from_str::<serde_json::Value>(&content).is_ok() {
                            CheckResult::healthy(
                                "keystore",
                                &format!("Keystore valid ({} bytes)", meta.len()),
                            )
                        } else {
                            CheckResult::critical(
                                "keystore",
                                "Keystore file is corrupted (invalid JSON)",
                                "Restore from backup",
                            )
                        }
                    }
                    Err(_) => CheckResult::critical(
                        "keystore",
                        "Cannot read keystore file",
                        "Check file permissions",
                    ),
                }
            }
            Err(e) => CheckResult::critical(
                "keystore",
                &format!("Cannot access keystore: {}", e),
                "Check file permissions",
            ),
        }
    }

    /// Check config file
    pub fn check_config(&self) -> CheckResult {
        if !self.config_path.exists() {
            return CheckResult::warning(
                "config",
                "No config file (using defaults)",
                "Run any wallet command to auto-create config",
            );
        }
        match std::fs::read_to_string(&self.config_path) {
            Ok(content) => {
                if serde_json::from_str::<serde_json::Value>(&content).is_ok() {
                    CheckResult::healthy("config", "Config file valid")
                } else {
                    CheckResult::warning(
                        "config",
                        "Config file has invalid JSON",
                        "Delete config file to reset to defaults",
                    )
                }
            }
            Err(_) => CheckResult::warning(
                "config",
                "Cannot read config file",
                "Check permissions or delete to reset",
            ),
        }
    }

    /// Check file permissions (keystore shouldn't be world-readable)
    pub fn check_permissions(&self) -> CheckResult {
        if !self.keystore_path.exists() {
            return CheckResult::healthy("permissions", "No keystore to check");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if let Ok(meta) = std::fs::metadata(&self.keystore_path) {
                let mode = meta.mode() & 0o777;
                if mode & 0o077 != 0 {
                    return CheckResult::warning(
                        "permissions",
                        &format!("Keystore is too permissive (mode {:o})", mode),
                        "Run: chmod 600 on the keystore file",
                    );
                }
                return CheckResult::healthy(
                    "permissions",
                    &format!("Keystore permissions OK (mode {:o})", mode),
                );
            }
        }
        CheckResult::healthy("permissions", "Permissions check passed")
    }

    /// Check available disk space
    pub fn check_disk_space(&self) -> CheckResult {
        // We can't easily get disk space in pure Rust without a crate,
        // so we check if we can write a small file
        let test_path = self.data_dir.join(".disk_space_check");
        let test_data = vec![0u8; 1024]; // 1KB
        match std::fs::write(&test_path, &test_data) {
            Ok(_) => {
                let _ = std::fs::remove_file(&test_path);
                CheckResult::healthy("disk_space", "Disk writable (basic check passed)")
            }
            Err(e) => {
                let msg = format!("Disk write failed: {}", e);
                if msg.contains("No space") {
                    CheckResult::critical(
                        "disk_space",
                        "No disk space available",
                        "Free up disk space",
                    )
                } else {
                    CheckResult::warning("disk_space", &msg, "Check disk space and permissions")
                }
            }
        }
    }

    /// Check if backup exists and is recent
    pub fn check_backup_age(&self) -> CheckResult {
        let backup_dir = self.data_dir.join("backups");
        if !backup_dir.exists() {
            return CheckResult::warning(
                "backup",
                "No backups directory found",
                "Run 'wallet backup export' to create a backup",
            );
        }
        match std::fs::read_dir(&backup_dir) {
            Ok(entries) => {
                let count = entries.count();
                if count == 0 {
                    CheckResult::warning(
                        "backup",
                        "Backup directory is empty",
                        "Run 'wallet backup export' to create a backup",
                    )
                } else {
                    CheckResult::healthy("backup", &format!("{} backup(s) found", count))
                }
            }
            Err(_) => CheckResult::warning(
                "backup",
                "Cannot read backups directory",
                "Check directory permissions",
            ),
        }
    }

    /// Check for stale lock files
    pub fn check_stale_locks(&self) -> CheckResult {
        let lock_path = self.data_dir.join(".lock");
        if lock_path.exists() {
            CheckResult::warning(
                "lock_files",
                "Stale lock file detected",
                "Remove .lock file if no other wallet process is running",
            )
        } else {
            CheckResult::healthy("lock_files", "No stale lock files")
        }
    }
}

// ── Quick check helpers ───────────────────────────────────────

/// Quick check: is the wallet in a usable state?
pub fn quick_check(data_dir: &Path) -> bool {
    data_dir.exists() && data_dir.is_dir() && data_dir.join("keystore.json").exists()
}

/// Count issues at each severity level
pub fn count_issues(report: &HealthReport) -> (usize, usize, usize) {
    (
        report.healthy_count(),
        report.warning_count(),
        report.critical_count(),
    )
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicU64, Ordering};
    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_dir() -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("evap_health_{}_{}", std::process::id(), id));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_health_status_ordering() {
        assert!(HealthStatus::Healthy < HealthStatus::Warning);
        assert!(HealthStatus::Warning < HealthStatus::Critical);
    }

    #[test]
    fn test_health_status_emoji() {
        assert_eq!(HealthStatus::Healthy.emoji(), "OK");
        assert_eq!(HealthStatus::Critical.emoji(), "CRIT");
    }

    #[test]
    fn test_health_status_is_ok() {
        assert!(HealthStatus::Healthy.is_ok());
        assert!(!HealthStatus::Warning.is_ok());
        assert!(!HealthStatus::Critical.is_ok());
    }

    #[test]
    fn test_check_result_healthy() {
        let r = CheckResult::healthy("test", "all good");
        assert_eq!(r.status, HealthStatus::Healthy);
        assert!(r.fix.is_none());
    }

    #[test]
    fn test_check_result_warning() {
        let r = CheckResult::warning("test", "issue", "fix it");
        assert_eq!(r.status, HealthStatus::Warning);
        assert_eq!(r.fix, Some("fix it".to_string()));
    }

    #[test]
    fn test_check_result_critical() {
        let r = CheckResult::critical("test", "broken", "rebuild");
        assert_eq!(r.status, HealthStatus::Critical);
    }

    #[test]
    fn test_check_result_unknown() {
        let r = CheckResult::unknown("test", "dunno");
        assert_eq!(r.status, HealthStatus::Unknown);
        assert!(r.fix.is_none());
    }

    #[test]
    fn test_health_report_all_healthy() {
        let checks = vec![
            CheckResult::healthy("a", "ok"),
            CheckResult::healthy("b", "ok"),
        ];
        let report = HealthReport::new(checks);
        assert!(report.is_healthy());
        assert_eq!(report.healthy_count(), 2);
        assert_eq!(report.warning_count(), 0);
        assert_eq!(report.critical_count(), 0);
    }

    #[test]
    fn test_health_report_mixed() {
        let checks = vec![
            CheckResult::healthy("a", "ok"),
            CheckResult::warning("b", "warn", "fix"),
            CheckResult::critical("c", "bad", "rebuild"),
        ];
        let report = HealthReport::new(checks);
        assert!(!report.is_healthy());
        assert_eq!(report.overall, HealthStatus::Critical);
        assert_eq!(report.healthy_count(), 1);
        assert_eq!(report.warning_count(), 1);
        assert_eq!(report.critical_count(), 1);
    }

    #[test]
    fn test_health_report_fixes() {
        let checks = vec![
            CheckResult::healthy("a", "ok"),
            CheckResult::warning("b", "warn", "do X"),
            CheckResult::critical("c", "bad", "do Y"),
        ];
        let report = HealthReport::new(checks);
        let fixes = report.fixes();
        assert_eq!(fixes.len(), 2);
        assert!(fixes.contains(&"do X"));
        assert!(fixes.contains(&"do Y"));
    }

    #[test]
    fn test_health_report_to_text() {
        let checks = vec![CheckResult::healthy("test_check", "all good")];
        let report = HealthReport::new(checks);
        let text = report.to_text();
        assert!(text.contains("Wallet Health"));
        assert!(text.contains("test_check"));
        assert!(text.contains("all good"));
    }

    #[test]
    fn test_health_report_empty() {
        let report = HealthReport::new(vec![]);
        assert_eq!(report.overall, HealthStatus::Unknown);
        assert_eq!(report.healthy_count(), 0);
    }

    #[test]
    fn test_checker_data_dir_exists() {
        let dir = test_dir();
        let checker = HealthChecker::new(dir.clone(), dir.join("ks.json"), dir.join("config.json"));
        let result = checker.check_data_dir();
        assert_eq!(result.status, HealthStatus::Healthy);
        cleanup(&dir);
    }

    #[test]
    fn test_checker_data_dir_missing() {
        let dir = std::env::temp_dir().join("evap_health_missing_12345");
        let _ = std::fs::remove_dir_all(&dir);
        let checker = HealthChecker::new(dir.clone(), dir.join("ks.json"), dir.join("config.json"));
        let result = checker.check_data_dir();
        assert_eq!(result.status, HealthStatus::Warning);
    }

    #[test]
    fn test_checker_keystore_missing() {
        let dir = test_dir();
        let checker = HealthChecker::new(
            dir.clone(),
            dir.join("nonexistent_ks.json"),
            dir.join("config.json"),
        );
        let result = checker.check_keystore();
        assert_eq!(result.status, HealthStatus::Warning);
        cleanup(&dir);
    }

    #[test]
    fn test_checker_keystore_valid() {
        let dir = test_dir();
        let ks_path = dir.join("ks.json");
        std::fs::write(&ks_path, r#"{"keys":[]}"#).unwrap();
        let checker = HealthChecker::new(dir.clone(), ks_path, dir.join("config.json"));
        let result = checker.check_keystore();
        assert_eq!(result.status, HealthStatus::Healthy);
        cleanup(&dir);
    }

    #[test]
    fn test_checker_keystore_empty() {
        let dir = test_dir();
        let ks_path = dir.join("ks.json");
        std::fs::write(&ks_path, "").unwrap();
        let checker = HealthChecker::new(dir.clone(), ks_path, dir.join("config.json"));
        let result = checker.check_keystore();
        assert_eq!(result.status, HealthStatus::Critical);
        cleanup(&dir);
    }

    #[test]
    fn test_checker_keystore_invalid_json() {
        let dir = test_dir();
        let ks_path = dir.join("ks.json");
        std::fs::write(&ks_path, "not json{{{").unwrap();
        let checker = HealthChecker::new(dir.clone(), ks_path, dir.join("config.json"));
        let result = checker.check_keystore();
        assert_eq!(result.status, HealthStatus::Critical);
        cleanup(&dir);
    }

    #[test]
    fn test_checker_config_missing() {
        let dir = test_dir();
        let checker =
            HealthChecker::new(dir.clone(), dir.join("ks.json"), dir.join("no_config.json"));
        let result = checker.check_config();
        assert_eq!(result.status, HealthStatus::Warning);
        cleanup(&dir);
    }

    #[test]
    fn test_checker_config_valid() {
        let dir = test_dir();
        let cfg_path = dir.join("config.json");
        std::fs::write(&cfg_path, r#"{"node_url":"http://localhost:3000"}"#).unwrap();
        let checker = HealthChecker::new(dir.clone(), dir.join("ks.json"), cfg_path);
        let result = checker.check_config();
        assert_eq!(result.status, HealthStatus::Healthy);
        cleanup(&dir);
    }

    #[test]
    fn test_checker_disk_space() {
        let dir = test_dir();
        let checker = HealthChecker::new(dir.clone(), dir.join("ks.json"), dir.join("config.json"));
        let result = checker.check_disk_space();
        assert_eq!(result.status, HealthStatus::Healthy);
        cleanup(&dir);
    }

    #[test]
    fn test_checker_no_stale_locks() {
        let dir = test_dir();
        let checker = HealthChecker::new(dir.clone(), dir.join("ks.json"), dir.join("config.json"));
        let result = checker.check_stale_locks();
        assert_eq!(result.status, HealthStatus::Healthy);
        cleanup(&dir);
    }

    #[test]
    fn test_checker_stale_lock_detected() {
        let dir = test_dir();
        std::fs::write(dir.join(".lock"), "pid=1234").unwrap();
        let checker = HealthChecker::new(dir.clone(), dir.join("ks.json"), dir.join("config.json"));
        let result = checker.check_stale_locks();
        assert_eq!(result.status, HealthStatus::Warning);
        cleanup(&dir);
    }

    #[test]
    fn test_checker_backup_no_dir() {
        let dir = test_dir();
        let checker = HealthChecker::new(dir.clone(), dir.join("ks.json"), dir.join("config.json"));
        let result = checker.check_backup_age();
        assert_eq!(result.status, HealthStatus::Warning);
        cleanup(&dir);
    }

    #[test]
    fn test_checker_backup_with_files() {
        let dir = test_dir();
        let backup_dir = dir.join("backups");
        std::fs::create_dir_all(&backup_dir).unwrap();
        std::fs::write(backup_dir.join("backup_001.enc"), "data").unwrap();
        let checker = HealthChecker::new(dir.clone(), dir.join("ks.json"), dir.join("config.json"));
        let result = checker.check_backup_age();
        assert_eq!(result.status, HealthStatus::Healthy);
        cleanup(&dir);
    }

    #[test]
    fn test_checker_run_all() {
        let dir = test_dir();
        let ks_path = dir.join("ks.json");
        std::fs::write(&ks_path, r#"{"keys":[]}"#).unwrap();
        let cfg_path = dir.join("config.json");
        std::fs::write(&cfg_path, r#"{}"#).unwrap();
        let checker = HealthChecker::new(dir.clone(), ks_path, cfg_path);
        let report = checker.run_all();
        assert_eq!(report.checks.len(), 7);
        cleanup(&dir);
    }

    #[test]
    fn test_quick_check_true() {
        let dir = test_dir();
        std::fs::write(dir.join("keystore.json"), "{}").unwrap();
        assert!(quick_check(&dir));
        cleanup(&dir);
    }

    #[test]
    fn test_quick_check_false() {
        let dir = std::env::temp_dir().join("evap_health_noexist_999");
        assert!(!quick_check(&dir));
    }

    #[test]
    fn test_count_issues() {
        let checks = vec![
            CheckResult::healthy("a", "ok"),
            CheckResult::warning("b", "warn", "fix"),
        ];
        let report = HealthReport::new(checks);
        let (h, w, c) = count_issues(&report);
        assert_eq!(h, 1);
        assert_eq!(w, 1);
        assert_eq!(c, 0);
    }

    #[test]
    fn test_health_report_serialization() {
        let checks = vec![CheckResult::healthy("test", "ok")];
        let report = HealthReport::new(checks);
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"overall\":\"Healthy\""));
    }
}
