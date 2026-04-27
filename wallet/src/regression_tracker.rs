use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum RegressionTrackerError {
    #[error("issue not found: {0}")]
    IssueNotFound(String),
    #[error("duplicate issue: {0}")]
    DuplicateIssue(String),
    #[error("baseline not found: {0}")]
    BaselineNotFound(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Parse(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IssueStatus2 {
    Open,
    Fixed,
    Regressed,
    WontFix,
    InProgress,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IssueSeverity2 {
    Critical,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BaselineStatus {
    Current,
    Outdated,
    Archived,
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownIssue {
    pub id: String,
    pub title: String,
    pub description: String,
    pub status: IssueStatus2,
    pub severity: IssueSeverity2,
    pub module: String,
    pub first_seen: String,
    pub last_seen: String,
    pub occurrences: u32,
    pub fix_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestBaseline {
    pub id: String,
    pub name: String,
    pub module: String,
    pub expected_result: String,
    pub actual_result: Option<String>,
    pub status: BaselineStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionDiff {
    pub baseline_id: String,
    pub expected: String,
    pub actual: String,
    pub detected_at: String,
    pub module: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackerStats {
    pub total_issues: usize,
    pub open: usize,
    pub fixed: usize,
    pub regressed: usize,
    pub total_baselines: usize,
    pub current_baselines: usize,
    pub total_diffs: usize,
    pub by_severity: HashMap<String, usize>,
}

// ---------------------------------------------------------------------------
// Main struct
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RegressionTracker {
    pub issues: HashMap<String, KnownIssue>,
    pub baselines: HashMap<String, TestBaseline>,
    pub diffs: Vec<RegressionDiff>,
}

impl RegressionTracker {
    pub fn new() -> Self {
        Self::default()
    }

    // -- Issue management ---------------------------------------------------

    pub fn report_issue(&mut self, issue: KnownIssue) -> Result<(), RegressionTrackerError> {
        if self.issues.contains_key(&issue.id) {
            return Err(RegressionTrackerError::DuplicateIssue(issue.id));
        }
        self.issues.insert(issue.id.clone(), issue);
        Ok(())
    }

    pub fn close_issue(
        &mut self,
        id: &str,
        fix_version: Option<String>,
    ) -> Result<(), RegressionTrackerError> {
        let issue = self
            .issues
            .get_mut(id)
            .ok_or_else(|| RegressionTrackerError::IssueNotFound(id.to_string()))?;
        issue.status = IssueStatus2::Fixed;
        issue.fix_version = fix_version;
        issue.last_seen = Utc::now().to_rfc3339();
        Ok(())
    }

    pub fn reopen_issue(&mut self, id: &str) -> Result<(), RegressionTrackerError> {
        let issue = self
            .issues
            .get_mut(id)
            .ok_or_else(|| RegressionTrackerError::IssueNotFound(id.to_string()))?;
        issue.status = IssueStatus2::Regressed;
        issue.occurrences += 1;
        issue.last_seen = Utc::now().to_rfc3339();
        Ok(())
    }

    pub fn update_issue_status(
        &mut self,
        id: &str,
        status: IssueStatus2,
    ) -> Result<(), RegressionTrackerError> {
        let issue = self
            .issues
            .get_mut(id)
            .ok_or_else(|| RegressionTrackerError::IssueNotFound(id.to_string()))?;
        issue.status = status;
        issue.last_seen = Utc::now().to_rfc3339();
        Ok(())
    }

    pub fn get_issue(&self, id: &str) -> Option<&KnownIssue> {
        self.issues.get(id)
    }

    pub fn issues_by_status(&self, status: &IssueStatus2) -> Vec<&KnownIssue> {
        self.issues
            .values()
            .filter(|i| &i.status == status)
            .collect()
    }

    pub fn issues_by_severity(&self, severity: &IssueSeverity2) -> Vec<&KnownIssue> {
        self.issues
            .values()
            .filter(|i| &i.severity == severity)
            .collect()
    }

    pub fn issues_by_module(&self, module: &str) -> Vec<&KnownIssue> {
        self.issues
            .values()
            .filter(|i| i.module == module)
            .collect()
    }

    // -- Baseline management ------------------------------------------------

    pub fn add_baseline(
        &mut self,
        baseline: TestBaseline,
    ) -> Result<(), RegressionTrackerError> {
        if self.baselines.contains_key(&baseline.id) {
            return Err(RegressionTrackerError::DuplicateIssue(baseline.id));
        }
        self.baselines.insert(baseline.id.clone(), baseline);
        Ok(())
    }

    pub fn check_baseline(
        &mut self,
        id: &str,
        actual: &str,
    ) -> Result<bool, RegressionTrackerError> {
        let baseline = self
            .baselines
            .get_mut(id)
            .ok_or_else(|| RegressionTrackerError::BaselineNotFound(id.to_string()))?;

        baseline.actual_result = Some(actual.to_string());
        baseline.updated_at = Utc::now().to_rfc3339();

        if baseline.expected_result == actual {
            Ok(true)
        } else {
            let diff = RegressionDiff {
                baseline_id: id.to_string(),
                expected: baseline.expected_result.clone(),
                actual: actual.to_string(),
                detected_at: Utc::now().to_rfc3339(),
                module: baseline.module.clone(),
            };
            self.diffs.push(diff);
            Ok(false)
        }
    }

    pub fn archive_baseline(&mut self, id: &str) -> Result<(), RegressionTrackerError> {
        let baseline = self
            .baselines
            .get_mut(id)
            .ok_or_else(|| RegressionTrackerError::BaselineNotFound(id.to_string()))?;
        baseline.status = BaselineStatus::Archived;
        baseline.updated_at = Utc::now().to_rfc3339();
        Ok(())
    }

    pub fn current_baselines(&self) -> Vec<&TestBaseline> {
        self.baselines
            .values()
            .filter(|b| b.status == BaselineStatus::Current)
            .collect()
    }

    // -- Diffs --------------------------------------------------------------

    pub fn diffs_for_module(&self, module: &str) -> Vec<&RegressionDiff> {
        self.diffs.iter().filter(|d| d.module == module).collect()
    }

    pub fn recent_diffs(&self, n: usize) -> Vec<&RegressionDiff> {
        let len = self.diffs.len();
        let start = len.saturating_sub(n);
        self.diffs[start..].iter().collect()
    }

    // -- Search / stats -----------------------------------------------------

    pub fn search_issues(&self, query: &str) -> Vec<&KnownIssue> {
        let q = query.to_lowercase();
        self.issues
            .values()
            .filter(|i| {
                i.title.to_lowercase().contains(&q)
                    || i.description.to_lowercase().contains(&q)
                    || i.module.to_lowercase().contains(&q)
            })
            .collect()
    }

    pub fn stats(&self) -> TrackerStats {
        let mut by_severity: HashMap<String, usize> = HashMap::new();
        let mut open = 0usize;
        let mut fixed = 0usize;
        let mut regressed = 0usize;

        for issue in self.issues.values() {
            match issue.status {
                IssueStatus2::Open => open += 1,
                IssueStatus2::Fixed => fixed += 1,
                IssueStatus2::Regressed => regressed += 1,
                _ => {}
            }
            let key = format!("{:?}", issue.severity);
            *by_severity.entry(key).or_insert(0) += 1;
        }

        TrackerStats {
            total_issues: self.issues.len(),
            open,
            fixed,
            regressed,
            total_baselines: self.baselines.len(),
            current_baselines: self
                .baselines
                .values()
                .filter(|b| b.status == BaselineStatus::Current)
                .count(),
            total_diffs: self.diffs.len(),
            by_severity,
        }
    }

    // -- Persistence --------------------------------------------------------

    pub fn save(&self, path: &Path) -> Result<(), RegressionTrackerError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, RegressionTrackerError> {
        let data = std::fs::read_to_string(path)?;
        let tracker: Self = serde_json::from_str(&data)?;
        Ok(tracker)
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
    use std::env::temp_dir;
    use std::process;

    fn test_path(name: &str) -> std::path::PathBuf {
        temp_dir().join(format!("regression_tracker_{}_{}.json", process::id(), name))
    }

    fn make_issue(id: &str, severity: IssueSeverity2, module: &str) -> KnownIssue {
        KnownIssue {
            id: id.to_string(),
            title: format!("Issue {}", id),
            description: format!("Description for {}", id),
            status: IssueStatus2::Open,
            severity,
            module: module.to_string(),
            first_seen: Utc::now().to_rfc3339(),
            last_seen: Utc::now().to_rfc3339(),
            occurrences: 1,
            fix_version: None,
        }
    }

    fn make_baseline(id: &str, module: &str, expected: &str) -> TestBaseline {
        TestBaseline {
            id: id.to_string(),
            name: format!("Baseline {}", id),
            module: module.to_string(),
            expected_result: expected.to_string(),
            actual_result: None,
            status: BaselineStatus::Current,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        }
    }

    #[test]
    fn test_report_issue() {
        let mut tracker = RegressionTracker::new();
        let issue = make_issue("ISS-1", IssueSeverity2::High, "wallet");
        assert!(tracker.report_issue(issue).is_ok());
        assert!(tracker.get_issue("ISS-1").is_some());
    }

    #[test]
    fn test_duplicate_issue() {
        let mut tracker = RegressionTracker::new();
        let issue = make_issue("ISS-1", IssueSeverity2::High, "wallet");
        tracker.report_issue(issue).unwrap();
        let dup = make_issue("ISS-1", IssueSeverity2::Low, "wallet");
        assert!(tracker.report_issue(dup).is_err());
    }

    #[test]
    fn test_close_issue() {
        let mut tracker = RegressionTracker::new();
        tracker
            .report_issue(make_issue("ISS-2", IssueSeverity2::Medium, "tx"))
            .unwrap();
        tracker
            .close_issue("ISS-2", Some("v1.2.0".to_string()))
            .unwrap();
        let issue = tracker.get_issue("ISS-2").unwrap();
        assert_eq!(issue.status, IssueStatus2::Fixed);
        assert_eq!(issue.fix_version, Some("v1.2.0".to_string()));
    }

    #[test]
    fn test_close_issue_not_found() {
        let mut tracker = RegressionTracker::new();
        assert!(tracker.close_issue("NOPE", None).is_err());
    }

    #[test]
    fn test_reopen_issue() {
        let mut tracker = RegressionTracker::new();
        tracker
            .report_issue(make_issue("ISS-3", IssueSeverity2::High, "wallet"))
            .unwrap();
        tracker.close_issue("ISS-3", None).unwrap();
        tracker.reopen_issue("ISS-3").unwrap();
        let issue = tracker.get_issue("ISS-3").unwrap();
        assert_eq!(issue.status, IssueStatus2::Regressed);
        assert_eq!(issue.occurrences, 2);
    }

    #[test]
    fn test_reopen_not_found() {
        let mut tracker = RegressionTracker::new();
        assert!(tracker.reopen_issue("NOPE").is_err());
    }

    #[test]
    fn test_update_issue_status() {
        let mut tracker = RegressionTracker::new();
        tracker
            .report_issue(make_issue("ISS-4", IssueSeverity2::Low, "gas"))
            .unwrap();
        tracker
            .update_issue_status("ISS-4", IssueStatus2::InProgress)
            .unwrap();
        assert_eq!(
            tracker.get_issue("ISS-4").unwrap().status,
            IssueStatus2::InProgress
        );
    }

    #[test]
    fn test_update_status_not_found() {
        let mut tracker = RegressionTracker::new();
        assert!(tracker
            .update_issue_status("NOPE", IssueStatus2::WontFix)
            .is_err());
    }

    #[test]
    fn test_issues_by_status() {
        let mut tracker = RegressionTracker::new();
        tracker
            .report_issue(make_issue("A", IssueSeverity2::High, "m"))
            .unwrap();
        tracker
            .report_issue(make_issue("B", IssueSeverity2::Low, "m"))
            .unwrap();
        tracker.close_issue("B", None).unwrap();
        assert_eq!(tracker.issues_by_status(&IssueStatus2::Open).len(), 1);
        assert_eq!(tracker.issues_by_status(&IssueStatus2::Fixed).len(), 1);
    }

    #[test]
    fn test_issues_by_severity() {
        let mut tracker = RegressionTracker::new();
        tracker
            .report_issue(make_issue("X", IssueSeverity2::Critical, "net"))
            .unwrap();
        tracker
            .report_issue(make_issue("Y", IssueSeverity2::Critical, "rpc"))
            .unwrap();
        tracker
            .report_issue(make_issue("Z", IssueSeverity2::Low, "rpc"))
            .unwrap();
        assert_eq!(
            tracker.issues_by_severity(&IssueSeverity2::Critical).len(),
            2
        );
        assert_eq!(tracker.issues_by_severity(&IssueSeverity2::Low).len(), 1);
    }

    #[test]
    fn test_issues_by_module() {
        let mut tracker = RegressionTracker::new();
        tracker
            .report_issue(make_issue("M1", IssueSeverity2::High, "wallet"))
            .unwrap();
        tracker
            .report_issue(make_issue("M2", IssueSeverity2::Low, "wallet"))
            .unwrap();
        tracker
            .report_issue(make_issue("M3", IssueSeverity2::Low, "gas"))
            .unwrap();
        assert_eq!(tracker.issues_by_module("wallet").len(), 2);
        assert_eq!(tracker.issues_by_module("gas").len(), 1);
    }

    #[test]
    fn test_add_baseline() {
        let mut tracker = RegressionTracker::new();
        let bl = make_baseline("BL-1", "wallet", "pass");
        assert!(tracker.add_baseline(bl).is_ok());
        assert_eq!(tracker.baselines.len(), 1);
    }

    #[test]
    fn test_add_duplicate_baseline() {
        let mut tracker = RegressionTracker::new();
        tracker
            .add_baseline(make_baseline("BL-1", "wallet", "pass"))
            .unwrap();
        assert!(tracker
            .add_baseline(make_baseline("BL-1", "wallet", "pass"))
            .is_err());
    }

    #[test]
    fn test_check_baseline_match() {
        let mut tracker = RegressionTracker::new();
        tracker
            .add_baseline(make_baseline("BL-2", "tx", "ok"))
            .unwrap();
        let result = tracker.check_baseline("BL-2", "ok").unwrap();
        assert!(result);
        assert_eq!(tracker.diffs.len(), 0);
    }

    #[test]
    fn test_check_baseline_mismatch() {
        let mut tracker = RegressionTracker::new();
        tracker
            .add_baseline(make_baseline("BL-3", "tx", "ok"))
            .unwrap();
        let result = tracker.check_baseline("BL-3", "fail").unwrap();
        assert!(!result);
        assert_eq!(tracker.diffs.len(), 1);
        assert_eq!(tracker.diffs[0].actual, "fail");
    }

    #[test]
    fn test_check_baseline_not_found() {
        let mut tracker = RegressionTracker::new();
        assert!(tracker.check_baseline("NOPE", "x").is_err());
    }

    #[test]
    fn test_archive_baseline() {
        let mut tracker = RegressionTracker::new();
        tracker
            .add_baseline(make_baseline("BL-4", "net", "ok"))
            .unwrap();
        tracker.archive_baseline("BL-4").unwrap();
        assert_eq!(
            tracker.baselines.get("BL-4").unwrap().status,
            BaselineStatus::Archived
        );
    }

    #[test]
    fn test_current_baselines() {
        let mut tracker = RegressionTracker::new();
        tracker
            .add_baseline(make_baseline("C1", "a", "ok"))
            .unwrap();
        tracker
            .add_baseline(make_baseline("C2", "b", "ok"))
            .unwrap();
        tracker.archive_baseline("C2").unwrap();
        assert_eq!(tracker.current_baselines().len(), 1);
    }

    #[test]
    fn test_diffs_for_module() {
        let mut tracker = RegressionTracker::new();
        tracker
            .add_baseline(make_baseline("D1", "wallet", "a"))
            .unwrap();
        tracker
            .add_baseline(make_baseline("D2", "gas", "b"))
            .unwrap();
        tracker.check_baseline("D1", "x").unwrap();
        tracker.check_baseline("D2", "y").unwrap();
        assert_eq!(tracker.diffs_for_module("wallet").len(), 1);
        assert_eq!(tracker.diffs_for_module("gas").len(), 1);
    }

    #[test]
    fn test_recent_diffs() {
        let mut tracker = RegressionTracker::new();
        for i in 0..5 {
            let id = format!("R{}", i);
            tracker
                .add_baseline(make_baseline(&id, "m", "exp"))
                .unwrap();
            tracker.check_baseline(&id, "act").unwrap();
        }
        assert_eq!(tracker.recent_diffs(3).len(), 3);
        assert_eq!(tracker.recent_diffs(10).len(), 5);
    }

    #[test]
    fn test_search_issues() {
        let mut tracker = RegressionTracker::new();
        let mut issue = make_issue("S1", IssueSeverity2::High, "wallet");
        issue.title = "Transaction timeout bug".to_string();
        tracker.report_issue(issue).unwrap();

        let mut issue2 = make_issue("S2", IssueSeverity2::Low, "rpc");
        issue2.description = "Timeout on rpc calls".to_string();
        tracker.report_issue(issue2).unwrap();

        assert_eq!(tracker.search_issues("timeout").len(), 2);
        assert_eq!(tracker.search_issues("rpc").len(), 1);
    }

    #[test]
    fn test_stats() {
        let mut tracker = RegressionTracker::new();
        tracker
            .report_issue(make_issue("ST1", IssueSeverity2::High, "a"))
            .unwrap();
        tracker
            .report_issue(make_issue("ST2", IssueSeverity2::High, "b"))
            .unwrap();
        tracker
            .report_issue(make_issue("ST3", IssueSeverity2::Low, "c"))
            .unwrap();
        tracker.close_issue("ST2", None).unwrap();
        tracker
            .add_baseline(make_baseline("SB1", "a", "ok"))
            .unwrap();
        tracker.check_baseline("SB1", "bad").unwrap();

        let s = tracker.stats();
        assert_eq!(s.total_issues, 3);
        assert_eq!(s.open, 2);
        assert_eq!(s.fixed, 1);
        assert_eq!(s.total_baselines, 1);
        assert_eq!(s.current_baselines, 1);
        assert_eq!(s.total_diffs, 1);
        assert_eq!(s.by_severity.get("High"), Some(&2));
        assert_eq!(s.by_severity.get("Low"), Some(&1));
    }

    #[test]
    fn test_save_and_load() {
        let path = test_path("save_load");
        let mut tracker = RegressionTracker::new();
        tracker
            .report_issue(make_issue("P1", IssueSeverity2::Medium, "wallet"))
            .unwrap();
        tracker
            .add_baseline(make_baseline("PB1", "wallet", "ok"))
            .unwrap();
        tracker.check_baseline("PB1", "bad").unwrap();
        tracker.save(&path).unwrap();

        let loaded = RegressionTracker::load(&path).unwrap();
        assert_eq!(loaded.issues.len(), 1);
        assert_eq!(loaded.baselines.len(), 1);
        assert_eq!(loaded.diffs.len(), 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing() {
        let path = test_path("nonexistent_file");
        let _ = std::fs::remove_file(&path);
        let tracker = RegressionTracker::load_or_default(&path);
        assert_eq!(tracker.issues.len(), 0);
    }
}
