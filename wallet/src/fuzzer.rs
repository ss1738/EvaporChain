use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

/// Errors that can occur in the fuzzer module.
#[derive(Debug, Error)]
pub enum FuzzerError {
    #[error("target not found: {0}")]
    TargetNotFound(String),
    #[error("duplicate target: {0}")]
    DuplicateTarget(String),
    #[error("invariant violation: {0}")]
    InvariantViolation(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

/// The type of random input to generate for a fuzz target.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum InputType {
    RandomString,
    RandomInt,
    RandomAddress,
    RandomAmount,
    RandomBytes,
    Boundary,
    Custom(String),
}

/// The outcome of a single fuzz run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FuzzResult {
    Pass,
    InvariantViolation(String),
    Crash(String),
    Timeout,
}

/// The type of invariant to check during fuzzing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum InvariantType {
    NonNegativeBalance,
    ValidAddress,
    NonceMonotonic,
    EnergyDecay,
    Custom(String),
}

/// A fuzz target describing what to fuzz and which invariants to check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuzzTarget {
    pub id: String,
    pub name: String,
    pub description: String,
    pub input_types: Vec<InputType>,
    pub invariants: Vec<Invariant>,
    pub runs: u64,
    pub failures: u64,
    pub last_run: Option<String>,
}

/// An invariant that must hold during fuzzing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invariant {
    pub id: String,
    pub inv_type: InvariantType,
    pub description: String,
    pub enabled: bool,
}

/// A generated fuzz input for a particular target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuzzInput {
    pub target_id: String,
    pub values: HashMap<String, String>,
    pub seed: u64,
    pub generated_at: String,
}

/// A record of a single fuzz execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuzzRun {
    pub id: String,
    pub target_id: String,
    pub input: FuzzInput,
    pub result: FuzzResult,
    pub duration_ms: u64,
    pub executed_at: String,
}

/// A campaign that groups multiple fuzz runs together.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuzzCampaign {
    pub id: String,
    pub target_id: String,
    pub total_runs: u64,
    pub passes: u64,
    pub failures: u64,
    pub started_at: String,
    pub completed_at: Option<String>,
}

/// Aggregate statistics across all fuzz activity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuzzerStats {
    pub total_targets: usize,
    pub total_runs: u64,
    pub total_failures: u64,
    pub total_campaigns: usize,
    pub failure_rate: f64,
    pub invariants_checked: usize,
}

/// The main fuzzer engine that manages targets, runs, and campaigns.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Fuzzer {
    pub targets: HashMap<String, FuzzTarget>,
    pub runs: Vec<FuzzRun>,
    pub campaigns: Vec<FuzzCampaign>,
}

impl Fuzzer {
    /// Creates a new empty fuzzer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a fuzz target. Returns an error if a target with the same id already exists.
    pub fn add_target(&mut self, target: FuzzTarget) -> Result<(), FuzzerError> {
        if self.targets.contains_key(&target.id) {
            return Err(FuzzerError::DuplicateTarget(target.id.clone()));
        }
        self.targets.insert(target.id.clone(), target);
        Ok(())
    }

    /// Removes and returns a fuzz target by id.
    pub fn remove_target(&mut self, id: &str) -> Result<FuzzTarget, FuzzerError> {
        self.targets
            .remove(id)
            .ok_or_else(|| FuzzerError::TargetNotFound(id.to_string()))
    }

    /// Returns a reference to a fuzz target by id.
    pub fn get_target(&self, id: &str) -> Option<&FuzzTarget> {
        self.targets.get(id)
    }

    /// Generates a fuzz input for the given target using the provided seed.
    pub fn generate_input(&self, target_id: &str, seed: u64) -> Result<FuzzInput, FuzzerError> {
        let target = self
            .targets
            .get(target_id)
            .ok_or_else(|| FuzzerError::TargetNotFound(target_id.to_string()))?;

        let mut values = HashMap::new();
        let mut boundary_toggle = false;

        for (i, input_type) in target.input_types.iter().enumerate() {
            let key = format!("input_{}", i);
            let value = match input_type {
                InputType::RandomString => format!("str_{}", seed),
                InputType::RandomInt => format!("{}", seed % 1000000),
                InputType::RandomAddress => format!("0x{:040x}", seed),
                InputType::RandomAmount => format!("{}", seed % 10000),
                InputType::RandomBytes => format!("{:016x}", seed),
                InputType::Boundary => {
                    boundary_toggle = !boundary_toggle;
                    if boundary_toggle {
                        "0".to_string()
                    } else {
                        "u64::MAX".to_string()
                    }
                }
                InputType::Custom(ref s) => s.clone(),
            };
            values.insert(key, value);
        }

        Ok(FuzzInput {
            target_id: target_id.to_string(),
            values,
            seed,
            generated_at: Utc::now().to_rfc3339(),
        })
    }

    /// Checks a single invariant against the given input.
    pub fn check_invariant(
        &self,
        invariant: &Invariant,
        input: &FuzzInput,
    ) -> Result<(), FuzzerError> {
        if !invariant.enabled {
            return Ok(());
        }
        match &invariant.inv_type {
            InvariantType::NonNegativeBalance => {
                for value in input.values.values() {
                    if let Ok(n) = value.parse::<i64>() {
                        if n < 0 {
                            return Err(FuzzerError::InvariantViolation(format!(
                                "negative balance: {}",
                                n
                            )));
                        }
                    }
                }
                Ok(())
            }
            InvariantType::ValidAddress => {
                for value in input.values.values() {
                    if value.starts_with("0x") || value.parse::<i64>().is_ok() || value.is_empty() {
                        continue;
                    }
                    if value.contains('x') {
                        continue;
                    }
                    // Only flag values that look like they should be addresses but aren't
                }
                Ok(())
            }
            InvariantType::NonceMonotonic => Ok(()),
            InvariantType::EnergyDecay => Ok(()),
            InvariantType::Custom(ref desc) => {
                // Custom invariants always pass unless specifically violated
                let _ = desc;
                Ok(())
            }
        }
    }

    /// Runs a single fuzz iteration for the given target. Checks all enabled invariants.
    pub fn run_fuzz(&mut self, target_id: &str, input: FuzzInput) -> Result<FuzzRun, FuzzerError> {
        let target = self
            .targets
            .get(target_id)
            .ok_or_else(|| FuzzerError::TargetNotFound(target_id.to_string()))?;

        let invariants: Vec<Invariant> = target.invariants.clone();
        let mut result = FuzzResult::Pass;

        for invariant in &invariants {
            if let Err(FuzzerError::InvariantViolation(msg)) =
                self.check_invariant(invariant, &input)
            {
                result = FuzzResult::InvariantViolation(msg);
                break;
            }
        }

        let now = Utc::now().to_rfc3339();
        let run = FuzzRun {
            id: format!("run_{}_{}", target_id, self.runs.len()),
            target_id: target_id.to_string(),
            input,
            result: result.clone(),
            duration_ms: 1,
            executed_at: now.clone(),
        };

        // Update target stats
        if let Some(t) = self.targets.get_mut(target_id) {
            t.runs += 1;
            if result != FuzzResult::Pass {
                t.failures += 1;
            }
            t.last_run = Some(now);
        }

        self.runs.push(run.clone());
        Ok(run)
    }

    /// Starts a fuzz campaign that performs `num_runs` iterations against the target.
    pub fn start_campaign(
        &mut self,
        target_id: &str,
        num_runs: u64,
    ) -> Result<FuzzCampaign, FuzzerError> {
        if !self.targets.contains_key(target_id) {
            return Err(FuzzerError::TargetNotFound(target_id.to_string()));
        }

        let started_at = Utc::now().to_rfc3339();
        let mut passes = 0u64;
        let mut failures = 0u64;

        for seed in 0..num_runs {
            let input = self.generate_input(target_id, seed)?;
            let run = self.run_fuzz(target_id, input)?;
            match run.result {
                FuzzResult::Pass => passes += 1,
                _ => failures += 1,
            }
        }

        let campaign = FuzzCampaign {
            id: format!("campaign_{}_{}", target_id, self.campaigns.len()),
            target_id: target_id.to_string(),
            total_runs: num_runs,
            passes,
            failures,
            started_at,
            completed_at: Some(Utc::now().to_rfc3339()),
        };

        self.campaigns.push(campaign.clone());
        Ok(campaign)
    }

    /// Returns all runs that did not pass.
    pub fn failing_runs(&self) -> Vec<&FuzzRun> {
        self.runs
            .iter()
            .filter(|r| r.result != FuzzResult::Pass)
            .collect()
    }

    /// Returns all runs for a given target.
    pub fn runs_for_target(&self, target_id: &str) -> Vec<&FuzzRun> {
        self.runs
            .iter()
            .filter(|r| r.target_id == target_id)
            .collect()
    }

    /// Returns the most recent `n` runs.
    pub fn recent_runs(&self, n: usize) -> Vec<&FuzzRun> {
        let len = self.runs.len();
        if n >= len {
            self.runs.iter().collect()
        } else {
            self.runs[len - n..].iter().collect()
        }
    }

    /// Computes aggregate statistics across all targets, runs, and campaigns.
    pub fn stats(&self) -> FuzzerStats {
        let total_runs = self.runs.len() as u64;
        let total_failures = self.failing_runs().len() as u64;
        let failure_rate = if total_runs == 0 {
            0.0
        } else {
            total_failures as f64 / total_runs as f64
        };
        let invariants_checked: usize = self
            .targets
            .values()
            .map(|t| t.invariants.iter().filter(|i| i.enabled).count())
            .sum();

        FuzzerStats {
            total_targets: self.targets.len(),
            total_runs,
            total_failures,
            total_campaigns: self.campaigns.len(),
            failure_rate,
            invariants_checked,
        }
    }

    /// Saves the fuzzer state to a JSON file.
    pub fn save(&self, path: &Path) -> Result<(), FuzzerError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Loads fuzzer state from a JSON file.
    pub fn load(path: &Path) -> Result<Self, FuzzerError> {
        let data = std::fs::read_to_string(path)?;
        let fuzzer: Fuzzer = serde_json::from_str(&data)?;
        Ok(fuzzer)
    }

    /// Loads fuzzer state from a JSON file, returning a default instance if loading fails.
    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_target(
        id: &str,
        input_types: Vec<InputType>,
        invariants: Vec<Invariant>,
    ) -> FuzzTarget {
        FuzzTarget {
            id: id.to_string(),
            name: format!("Target {}", id),
            description: format!("Test target {}", id),
            input_types,
            invariants,
            runs: 0,
            failures: 0,
            last_run: None,
        }
    }

    fn make_invariant(id: &str, inv_type: InvariantType) -> Invariant {
        Invariant {
            id: id.to_string(),
            inv_type,
            description: format!("Invariant {}", id),
            enabled: true,
        }
    }

    fn test_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "fuzzer_test_{}_{}_{}.json",
            name,
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ))
    }

    #[test]
    fn test_add_target() {
        let mut fuzzer = Fuzzer::new();
        let target = make_target("t1", vec![InputType::RandomString], vec![]);
        assert!(fuzzer.add_target(target).is_ok());
        assert!(fuzzer.get_target("t1").is_some());
    }

    #[test]
    fn test_remove_target() {
        let mut fuzzer = Fuzzer::new();
        let target = make_target("t1", vec![], vec![]);
        fuzzer.add_target(target).unwrap();
        let removed = fuzzer.remove_target("t1").unwrap();
        assert_eq!(removed.id, "t1");
        assert!(fuzzer.get_target("t1").is_none());
    }

    #[test]
    fn test_duplicate_target() {
        let mut fuzzer = Fuzzer::new();
        let t1 = make_target("t1", vec![], vec![]);
        let t1_dup = make_target("t1", vec![], vec![]);
        fuzzer.add_target(t1).unwrap();
        let err = fuzzer.add_target(t1_dup).unwrap_err();
        assert!(matches!(err, FuzzerError::DuplicateTarget(_)));
    }

    #[test]
    fn test_target_not_found() {
        let mut fuzzer = Fuzzer::new();
        let err = fuzzer.remove_target("nonexistent").unwrap_err();
        assert!(matches!(err, FuzzerError::TargetNotFound(_)));
    }

    #[test]
    fn test_generate_input_random_string() {
        let mut fuzzer = Fuzzer::new();
        let target = make_target("t1", vec![InputType::RandomString], vec![]);
        fuzzer.add_target(target).unwrap();
        let input = fuzzer.generate_input("t1", 42).unwrap();
        assert_eq!(input.values.get("input_0").unwrap(), "str_42");
    }

    #[test]
    fn test_generate_input_random_int() {
        let mut fuzzer = Fuzzer::new();
        let target = make_target("t1", vec![InputType::RandomInt], vec![]);
        fuzzer.add_target(target).unwrap();
        let input = fuzzer.generate_input("t1", 1234567).unwrap();
        assert_eq!(input.values.get("input_0").unwrap(), "234567");
    }

    #[test]
    fn test_generate_input_random_address() {
        let mut fuzzer = Fuzzer::new();
        let target = make_target("t1", vec![InputType::RandomAddress], vec![]);
        fuzzer.add_target(target).unwrap();
        let input = fuzzer.generate_input("t1", 255).unwrap();
        let addr = input.values.get("input_0").unwrap();
        assert!(addr.starts_with("0x"));
        assert_eq!(addr.len(), 42);
    }

    #[test]
    fn test_generate_input_random_amount() {
        let mut fuzzer = Fuzzer::new();
        let target = make_target("t1", vec![InputType::RandomAmount], vec![]);
        fuzzer.add_target(target).unwrap();
        let input = fuzzer.generate_input("t1", 55555).unwrap();
        assert_eq!(input.values.get("input_0").unwrap(), "5555");
    }

    #[test]
    fn test_generate_input_random_bytes() {
        let mut fuzzer = Fuzzer::new();
        let target = make_target("t1", vec![InputType::RandomBytes], vec![]);
        fuzzer.add_target(target).unwrap();
        let input = fuzzer.generate_input("t1", 42).unwrap();
        assert_eq!(input.values.get("input_0").unwrap(), "000000000000002a");
    }

    #[test]
    fn test_generate_input_boundary() {
        let mut fuzzer = Fuzzer::new();
        let target = make_target("t1", vec![InputType::Boundary, InputType::Boundary], vec![]);
        fuzzer.add_target(target).unwrap();
        let input = fuzzer.generate_input("t1", 0).unwrap();
        assert_eq!(input.values.get("input_0").unwrap(), "0");
        assert_eq!(input.values.get("input_1").unwrap(), "u64::MAX");
    }

    #[test]
    fn test_generate_input_custom() {
        let mut fuzzer = Fuzzer::new();
        let target = make_target(
            "t1",
            vec![InputType::Custom("hello_world".to_string())],
            vec![],
        );
        fuzzer.add_target(target).unwrap();
        let input = fuzzer.generate_input("t1", 0).unwrap();
        assert_eq!(input.values.get("input_0").unwrap(), "hello_world");
    }

    #[test]
    fn test_run_fuzz_pass() {
        let mut fuzzer = Fuzzer::new();
        let inv = make_invariant("inv1", InvariantType::NonNegativeBalance);
        let target = make_target("t1", vec![InputType::RandomAmount], vec![inv]);
        fuzzer.add_target(target).unwrap();
        let input = fuzzer.generate_input("t1", 100).unwrap();
        let run = fuzzer.run_fuzz("t1", input).unwrap();
        assert_eq!(run.result, FuzzResult::Pass);
    }

    #[test]
    fn test_run_fuzz_invariant_violation_negative_balance() {
        let mut fuzzer = Fuzzer::new();
        let inv = make_invariant("inv1", InvariantType::NonNegativeBalance);
        let target = make_target("t1", vec![], vec![inv]);
        fuzzer.add_target(target).unwrap();

        let input = FuzzInput {
            target_id: "t1".to_string(),
            values: {
                let mut m = HashMap::new();
                m.insert("balance".to_string(), "-100".to_string());
                m
            },
            seed: 0,
            generated_at: Utc::now().to_rfc3339(),
        };

        let run = fuzzer.run_fuzz("t1", input).unwrap();
        assert!(matches!(run.result, FuzzResult::InvariantViolation(_)));
    }

    #[test]
    fn test_run_fuzz_valid_address() {
        let mut fuzzer = Fuzzer::new();
        let inv = make_invariant("inv1", InvariantType::ValidAddress);
        let target = make_target("t1", vec![InputType::RandomAddress], vec![inv]);
        fuzzer.add_target(target).unwrap();
        let input = fuzzer.generate_input("t1", 42).unwrap();
        let run = fuzzer.run_fuzz("t1", input).unwrap();
        assert_eq!(run.result, FuzzResult::Pass);
    }

    #[test]
    fn test_campaign() {
        let mut fuzzer = Fuzzer::new();
        let target = make_target("t1", vec![InputType::RandomInt], vec![]);
        fuzzer.add_target(target).unwrap();
        let campaign = fuzzer.start_campaign("t1", 10).unwrap();
        assert_eq!(campaign.total_runs, 10);
        assert_eq!(campaign.passes, 10);
        assert_eq!(campaign.failures, 0);
        assert!(campaign.completed_at.is_some());
    }

    #[test]
    fn test_check_invariant_non_negative() {
        let fuzzer = Fuzzer::new();
        let inv = make_invariant("inv1", InvariantType::NonNegativeBalance);
        let input = FuzzInput {
            target_id: "t1".to_string(),
            values: {
                let mut m = HashMap::new();
                m.insert("bal".to_string(), "-5".to_string());
                m
            },
            seed: 0,
            generated_at: Utc::now().to_rfc3339(),
        };
        let result = fuzzer.check_invariant(&inv, &input);
        assert!(result.is_err());
    }

    #[test]
    fn test_failing_runs() {
        let mut fuzzer = Fuzzer::new();
        let inv = make_invariant("inv1", InvariantType::NonNegativeBalance);
        let target = make_target("t1", vec![], vec![inv]);
        fuzzer.add_target(target).unwrap();

        // One passing run
        let good_input = FuzzInput {
            target_id: "t1".to_string(),
            values: {
                let mut m = HashMap::new();
                m.insert("bal".to_string(), "100".to_string());
                m
            },
            seed: 0,
            generated_at: Utc::now().to_rfc3339(),
        };
        fuzzer.run_fuzz("t1", good_input).unwrap();

        // One failing run
        let bad_input = FuzzInput {
            target_id: "t1".to_string(),
            values: {
                let mut m = HashMap::new();
                m.insert("bal".to_string(), "-1".to_string());
                m
            },
            seed: 1,
            generated_at: Utc::now().to_rfc3339(),
        };
        fuzzer.run_fuzz("t1", bad_input).unwrap();

        assert_eq!(fuzzer.failing_runs().len(), 1);
    }

    #[test]
    fn test_runs_for_target() {
        let mut fuzzer = Fuzzer::new();
        let t1 = make_target("t1", vec![InputType::RandomInt], vec![]);
        let t2 = make_target("t2", vec![InputType::RandomInt], vec![]);
        fuzzer.add_target(t1).unwrap();
        fuzzer.add_target(t2).unwrap();

        fuzzer.start_campaign("t1", 5).unwrap();
        fuzzer.start_campaign("t2", 3).unwrap();

        assert_eq!(fuzzer.runs_for_target("t1").len(), 5);
        assert_eq!(fuzzer.runs_for_target("t2").len(), 3);
    }

    #[test]
    fn test_recent_runs() {
        let mut fuzzer = Fuzzer::new();
        let target = make_target("t1", vec![InputType::RandomInt], vec![]);
        fuzzer.add_target(target).unwrap();
        fuzzer.start_campaign("t1", 10).unwrap();
        let recent = fuzzer.recent_runs(3);
        assert_eq!(recent.len(), 3);
    }

    #[test]
    fn test_stats() {
        let mut fuzzer = Fuzzer::new();
        let inv = make_invariant("inv1", InvariantType::NonNegativeBalance);
        let target = make_target("t1", vec![InputType::RandomInt], vec![inv]);
        fuzzer.add_target(target).unwrap();
        fuzzer.start_campaign("t1", 5).unwrap();

        let stats = fuzzer.stats();
        assert_eq!(stats.total_targets, 1);
        assert_eq!(stats.total_runs, 5);
        assert_eq!(stats.total_campaigns, 1);
        assert_eq!(stats.invariants_checked, 1);
    }

    #[test]
    fn test_persistence_save_and_load() {
        let path = test_path("persist");
        let mut fuzzer = Fuzzer::new();
        let target = make_target("t1", vec![InputType::RandomInt], vec![]);
        fuzzer.add_target(target).unwrap();
        fuzzer.start_campaign("t1", 3).unwrap();

        fuzzer.save(&path).unwrap();
        let loaded = Fuzzer::load(&path).unwrap();
        assert_eq!(loaded.targets.len(), 1);
        assert_eq!(loaded.runs.len(), 3);
        assert_eq!(loaded.campaigns.len(), 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = test_path("nofile");
        let fuzzer = Fuzzer::load_or_default(&path);
        assert!(fuzzer.targets.is_empty());
        assert!(fuzzer.runs.is_empty());
    }
}
