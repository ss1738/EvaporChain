use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum CoverageReportError {
    #[error("Module not found: {0}")]
    ModuleNotFound(String),
    #[error("Duplicate module: {0}")]
    DuplicateModule(String),
    #[error("Report not found: {0}")]
    ReportNotFound(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Parse(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CoverageLevel2 {
    None,
    Low,
    Medium,
    High,
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PathStatus {
    Covered,
    Uncovered,
    Partial,
}

// ---------------------------------------------------------------------------
// Supporting structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UncoveredPath {
    pub path: String,
    pub line_start: u32,
    pub line_end: u32,
    pub status: PathStatus,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleCoverage {
    pub id: String,
    pub module_name: String,
    pub total_lines: u32,
    pub covered_lines: u32,
    pub total_functions: u32,
    pub covered_functions: u32,
    pub total_branches: u32,
    pub covered_branches: u32,
    pub uncovered_paths: Vec<UncoveredPath>,
    pub updated_at: String,
}

impl ModuleCoverage {
    pub fn line_coverage_pct(&self) -> f64 {
        if self.total_lines == 0 {
            return 0.0;
        }
        (self.covered_lines as f64 / self.total_lines as f64) * 100.0
    }

    pub fn function_coverage_pct(&self) -> f64 {
        if self.total_functions == 0 {
            return 0.0;
        }
        (self.covered_functions as f64 / self.total_functions as f64) * 100.0
    }

    pub fn branch_coverage_pct(&self) -> f64 {
        if self.total_branches == 0 {
            return 0.0;
        }
        (self.covered_branches as f64 / self.total_branches as f64) * 100.0
    }

    pub fn coverage_level(&self) -> CoverageLevel2 {
        let pct = self.line_coverage_pct();
        if pct == 0.0 {
            CoverageLevel2::None
        } else if pct < 50.0 {
            CoverageLevel2::Low
        } else if pct < 75.0 {
            CoverageLevel2::Medium
        } else if pct < 90.0 {
            CoverageLevel2::High
        } else {
            CoverageLevel2::Full
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageReport2 {
    pub id: String,
    pub name: String,
    pub modules: Vec<String>,
    pub total_coverage: f64,
    pub generated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageTrend {
    pub module_id: String,
    pub coverage_pct: f64,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageStats2 {
    pub total_modules: usize,
    pub avg_line_coverage: f64,
    pub avg_function_coverage: f64,
    pub avg_branch_coverage: f64,
    pub fully_covered: usize,
    pub uncovered: usize,
    pub total_uncovered_paths: usize,
    pub reports_generated: usize,
}

// ---------------------------------------------------------------------------
// Main tracker
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CoverageTracker {
    pub modules: HashMap<String, ModuleCoverage>,
    pub reports: Vec<CoverageReport2>,
    pub trends: Vec<CoverageTrend>,
}

impl CoverageTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_module(&mut self, module: ModuleCoverage) -> Result<(), CoverageReportError> {
        if self.modules.contains_key(&module.id) {
            return Err(CoverageReportError::DuplicateModule(module.id.clone()));
        }
        self.modules.insert(module.id.clone(), module);
        Ok(())
    }

    pub fn remove_module(&mut self, id: &str) -> Result<ModuleCoverage, CoverageReportError> {
        self.modules
            .remove(id)
            .ok_or_else(|| CoverageReportError::ModuleNotFound(id.to_string()))
    }

    pub fn update_module(
        &mut self,
        id: &str,
        covered_lines: u32,
        covered_functions: u32,
        covered_branches: u32,
    ) -> Result<(), CoverageReportError> {
        let module = self
            .modules
            .get_mut(id)
            .ok_or_else(|| CoverageReportError::ModuleNotFound(id.to_string()))?;
        module.covered_lines = covered_lines;
        module.covered_functions = covered_functions;
        module.covered_branches = covered_branches;
        module.updated_at = Utc::now().to_rfc3339();

        let pct = module.line_coverage_pct();
        self.trends.push(CoverageTrend {
            module_id: id.to_string(),
            coverage_pct: pct,
            recorded_at: Utc::now().to_rfc3339(),
        });

        Ok(())
    }

    pub fn get_module(&self, id: &str) -> Option<&ModuleCoverage> {
        self.modules.get(id)
    }

    pub fn add_uncovered_path(
        &mut self,
        module_id: &str,
        path: UncoveredPath,
    ) -> Result<(), CoverageReportError> {
        let module = self
            .modules
            .get_mut(module_id)
            .ok_or_else(|| CoverageReportError::ModuleNotFound(module_id.to_string()))?;
        module.uncovered_paths.push(path);
        Ok(())
    }

    pub fn modules_by_coverage(&self, min_pct: f64) -> Vec<&ModuleCoverage> {
        let mut result: Vec<&ModuleCoverage> = self
            .modules
            .values()
            .filter(|m| m.line_coverage_pct() >= min_pct)
            .collect();
        result.sort_by(|a, b| {
            b.line_coverage_pct()
                .partial_cmp(&a.line_coverage_pct())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        result
    }

    pub fn modules_below_threshold(&self, threshold: f64) -> Vec<&ModuleCoverage> {
        let mut result: Vec<&ModuleCoverage> = self
            .modules
            .values()
            .filter(|m| m.line_coverage_pct() < threshold)
            .collect();
        result.sort_by(|a, b| {
            a.line_coverage_pct()
                .partial_cmp(&b.line_coverage_pct())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        result
    }

    pub fn generate_report(&mut self, name: &str) -> CoverageReport2 {
        let module_ids: Vec<String> = self.modules.keys().cloned().collect();
        let total_coverage = self.overall_coverage();
        let report = CoverageReport2 {
            id: format!("report-{}", self.reports.len() + 1),
            name: name.to_string(),
            modules: module_ids,
            total_coverage,
            generated_at: Utc::now().to_rfc3339(),
        };
        self.reports.push(report.clone());
        report
    }

    pub fn trends_for_module(&self, module_id: &str) -> Vec<&CoverageTrend> {
        self.trends
            .iter()
            .filter(|t| t.module_id == module_id)
            .collect()
    }

    pub fn overall_coverage(&self) -> f64 {
        let total_lines: u32 = self.modules.values().map(|m| m.total_lines).sum();
        if total_lines == 0 {
            return 0.0;
        }
        let covered_lines: u32 = self.modules.values().map(|m| m.covered_lines).sum();
        (covered_lines as f64 / total_lines as f64) * 100.0
    }

    pub fn uncovered_paths_all(&self) -> Vec<(&str, &UncoveredPath)> {
        let mut result = Vec::new();
        for module in self.modules.values() {
            for path in &module.uncovered_paths {
                result.push((module.module_name.as_str(), path));
            }
        }
        result
    }

    pub fn stats(&self) -> CoverageStats2 {
        let total_modules = self.modules.len();
        let (avg_line, avg_func, avg_branch) = if total_modules == 0 {
            (0.0, 0.0, 0.0)
        } else {
            let sum_line: f64 = self.modules.values().map(|m| m.line_coverage_pct()).sum();
            let sum_func: f64 = self.modules.values().map(|m| m.function_coverage_pct()).sum();
            let sum_branch: f64 = self.modules.values().map(|m| m.branch_coverage_pct()).sum();
            (
                sum_line / total_modules as f64,
                sum_func / total_modules as f64,
                sum_branch / total_modules as f64,
            )
        };
        let fully_covered = self
            .modules
            .values()
            .filter(|m| m.coverage_level() == CoverageLevel2::Full)
            .count();
        let uncovered = self
            .modules
            .values()
            .filter(|m| m.coverage_level() == CoverageLevel2::None)
            .count();
        let total_uncovered_paths: usize = self
            .modules
            .values()
            .map(|m| m.uncovered_paths.len())
            .sum();

        CoverageStats2 {
            total_modules,
            avg_line_coverage: avg_line,
            avg_function_coverage: avg_func,
            avg_branch_coverage: avg_branch,
            fully_covered,
            uncovered,
            total_uncovered_paths,
            reports_generated: self.reports.len(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), CoverageReportError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, CoverageReportError> {
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
    use std::process::id;

    fn test_path(name: &str) -> std::path::PathBuf {
        temp_dir().join(format!("coverage_report_test_{}_{}.json", id(), name))
    }

    fn make_module(mid: &str, name: &str, total: u32, covered: u32) -> ModuleCoverage {
        ModuleCoverage {
            id: mid.to_string(),
            module_name: name.to_string(),
            total_lines: total,
            covered_lines: covered,
            total_functions: 10,
            covered_functions: 8,
            total_branches: 20,
            covered_branches: 15,
            uncovered_paths: vec![],
            updated_at: Utc::now().to_rfc3339(),
        }
    }

    #[test]
    fn test_add_module() {
        let mut tracker = CoverageTracker::new();
        let m = make_module("m1", "wallet", 100, 80);
        assert!(tracker.add_module(m).is_ok());
        assert!(tracker.get_module("m1").is_some());
    }

    #[test]
    fn test_duplicate_module() {
        let mut tracker = CoverageTracker::new();
        tracker.add_module(make_module("m1", "wallet", 100, 80)).unwrap();
        let result = tracker.add_module(make_module("m1", "wallet", 100, 80));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CoverageReportError::DuplicateModule(_)));
    }

    #[test]
    fn test_remove_module() {
        let mut tracker = CoverageTracker::new();
        tracker.add_module(make_module("m1", "wallet", 100, 80)).unwrap();
        let removed = tracker.remove_module("m1").unwrap();
        assert_eq!(removed.id, "m1");
        assert!(tracker.get_module("m1").is_none());
    }

    #[test]
    fn test_remove_module_not_found() {
        let mut tracker = CoverageTracker::new();
        let result = tracker.remove_module("nonexistent");
        assert!(matches!(result.unwrap_err(), CoverageReportError::ModuleNotFound(_)));
    }

    #[test]
    fn test_update_module_records_trend() {
        let mut tracker = CoverageTracker::new();
        tracker.add_module(make_module("m1", "wallet", 100, 50)).unwrap();
        tracker.update_module("m1", 80, 9, 18).unwrap();
        let m = tracker.get_module("m1").unwrap();
        assert_eq!(m.covered_lines, 80);
        assert_eq!(m.covered_functions, 9);
        assert_eq!(m.covered_branches, 18);
        let trends = tracker.trends_for_module("m1");
        assert_eq!(trends.len(), 1);
        assert!((trends[0].coverage_pct - 80.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_update_module_not_found() {
        let mut tracker = CoverageTracker::new();
        let result = tracker.update_module("nope", 1, 1, 1);
        assert!(matches!(result.unwrap_err(), CoverageReportError::ModuleNotFound(_)));
    }

    #[test]
    fn test_line_coverage_pct() {
        let m = make_module("m1", "wallet", 200, 150);
        assert!((m.line_coverage_pct() - 75.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_function_coverage_pct() {
        let m = make_module("m1", "wallet", 100, 80);
        assert!((m.function_coverage_pct() - 80.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_branch_coverage_pct() {
        let m = make_module("m1", "wallet", 100, 80);
        assert!((m.branch_coverage_pct() - 75.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_coverage_level_none() {
        let m = make_module("m1", "wallet", 100, 0);
        assert_eq!(m.coverage_level(), CoverageLevel2::None);
    }

    #[test]
    fn test_coverage_level_low() {
        let m = make_module("m1", "wallet", 100, 30);
        assert_eq!(m.coverage_level(), CoverageLevel2::Low);
    }

    #[test]
    fn test_coverage_level_medium() {
        let m = make_module("m1", "wallet", 100, 60);
        assert_eq!(m.coverage_level(), CoverageLevel2::Medium);
    }

    #[test]
    fn test_coverage_level_high() {
        let m = make_module("m1", "wallet", 100, 80);
        assert_eq!(m.coverage_level(), CoverageLevel2::High);
    }

    #[test]
    fn test_coverage_level_full() {
        let m = make_module("m1", "wallet", 100, 95);
        assert_eq!(m.coverage_level(), CoverageLevel2::Full);
    }

    #[test]
    fn test_add_uncovered_path() {
        let mut tracker = CoverageTracker::new();
        tracker.add_module(make_module("m1", "wallet", 100, 80)).unwrap();
        let p = UncoveredPath {
            path: "src/lib.rs".to_string(),
            line_start: 10,
            line_end: 20,
            status: PathStatus::Uncovered,
            description: "missing tests".to_string(),
        };
        tracker.add_uncovered_path("m1", p).unwrap();
        assert_eq!(tracker.get_module("m1").unwrap().uncovered_paths.len(), 1);
    }

    #[test]
    fn test_modules_by_coverage() {
        let mut tracker = CoverageTracker::new();
        tracker.add_module(make_module("m1", "low", 100, 30)).unwrap();
        tracker.add_module(make_module("m2", "high", 100, 90)).unwrap();
        let result = tracker.modules_by_coverage(50.0);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "m2");
    }

    #[test]
    fn test_modules_below_threshold() {
        let mut tracker = CoverageTracker::new();
        tracker.add_module(make_module("m1", "low", 100, 30)).unwrap();
        tracker.add_module(make_module("m2", "high", 100, 90)).unwrap();
        let result = tracker.modules_below_threshold(50.0);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "m1");
    }

    #[test]
    fn test_generate_report() {
        let mut tracker = CoverageTracker::new();
        tracker.add_module(make_module("m1", "wallet", 100, 80)).unwrap();
        let report = tracker.generate_report("weekly");
        assert_eq!(report.name, "weekly");
        assert!((report.total_coverage - 80.0).abs() < f64::EPSILON);
        assert_eq!(tracker.reports.len(), 1);
    }

    #[test]
    fn test_overall_coverage_weighted() {
        let mut tracker = CoverageTracker::new();
        // 200 lines, 100 covered => 50%
        tracker.add_module(make_module("m1", "big", 200, 100)).unwrap();
        // 100 lines, 100 covered => 100%
        tracker.add_module(make_module("m2", "small", 100, 100)).unwrap();
        // weighted: (100+100) / (200+100) = 200/300 = 66.666...%
        let cov = tracker.overall_coverage();
        assert!((cov - 66.66666666666667).abs() < 0.001);
    }

    #[test]
    fn test_uncovered_paths_all() {
        let mut tracker = CoverageTracker::new();
        tracker.add_module(make_module("m1", "wallet", 100, 80)).unwrap();
        let p = UncoveredPath {
            path: "src/lib.rs".to_string(),
            line_start: 1,
            line_end: 5,
            status: PathStatus::Partial,
            description: "partial".to_string(),
        };
        tracker.add_uncovered_path("m1", p).unwrap();
        let all = tracker.uncovered_paths_all();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, "wallet");
    }

    #[test]
    fn test_stats() {
        let mut tracker = CoverageTracker::new();
        tracker.add_module(make_module("m1", "wallet", 100, 95)).unwrap();
        tracker.add_module(make_module("m2", "staking", 100, 0)).unwrap();
        tracker.generate_report("r1");
        let stats = tracker.stats();
        assert_eq!(stats.total_modules, 2);
        assert_eq!(stats.fully_covered, 1);
        assert_eq!(stats.uncovered, 1);
        assert_eq!(stats.reports_generated, 1);
    }

    #[test]
    fn test_save_and_load() {
        let path = test_path("save_load");
        let mut tracker = CoverageTracker::new();
        tracker.add_module(make_module("m1", "wallet", 100, 80)).unwrap();
        tracker.save(&path).unwrap();
        let loaded = CoverageTracker::load(&path).unwrap();
        assert!(loaded.get_module("m1").is_some());
        assert_eq!(loaded.get_module("m1").unwrap().covered_lines, 80);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = test_path("nonexistent_load_or_default");
        let _ = std::fs::remove_file(&path);
        let tracker = CoverageTracker::load_or_default(&path);
        assert!(tracker.modules.is_empty());
    }

    #[test]
    fn test_empty_module_zero_lines() {
        let m = make_module("m0", "empty", 0, 0);
        assert!((m.line_coverage_pct() - 0.0).abs() < f64::EPSILON);
        assert_eq!(m.coverage_level(), CoverageLevel2::None);
    }
}
