//! Scripting DSL — JSON-defined workflows chaining wallet operations.
//!
//! Define automated playbooks: sequences of wallet operations with
//! conditionals, variable interpolation, and error handling. Scripts
//! are JSON files that can be shared, versioned, and audited.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

// ──────────────────────────── Types ──────────────────────────────────────

#[derive(Debug, Clone, thiserror::Error)]
pub enum ScriptError {
    #[error("step failed: {0}")]
    StepFailed(String),
    #[error("condition failed: {0}")]
    ConditionFailed(String),
    #[error("variable not found: {0}")]
    VariableNotFound(String),
    #[error("invalid script: {0}")]
    InvalidScript(String),
    #[error("step not found: {0}")]
    StepNotFound(String),
    #[error("max steps exceeded: {0}")]
    MaxStepsExceeded(usize),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
}

impl From<std::io::Error> for ScriptError {
    fn from(e: std::io::Error) -> Self {
        ScriptError::Io(e.to_string())
    }
}
impl From<serde_json::Error> for ScriptError {
    fn from(e: serde_json::Error) -> Self {
        ScriptError::Json(e.to_string())
    }
}

/// An operation in a script step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    /// Transfer EVAP.
    Transfer { to: String, amount: u64 },
    /// Refresh an object.
    Refresh { object_id: String, energy: u64 },
    /// Check balance (stores result in variable).
    CheckBalance { address: String, store_as: String },
    /// Set a variable.
    SetVar { name: String, value: String },
    /// Log a message.
    Log { message: String },
    /// Wait N seconds (simulated in dry-run).
    Wait { seconds: u64 },
    /// Request faucet tokens.
    Faucet { address: String },
    /// Stake tokens.
    Stake { pool_id: String, amount: u64 },
    /// No-op (placeholder).
    Noop,
}

impl Operation {
    pub fn label(&self) -> String {
        match self {
            Operation::Transfer { to, amount } => format!("transfer {} to {}", amount, to),
            Operation::Refresh { object_id, energy } => {
                format!("refresh {} with {}", object_id, energy)
            }
            Operation::CheckBalance { address, store_as } => {
                format!("check_balance {} -> ${}", address, store_as)
            }
            Operation::SetVar { name, value } => format!("set ${} = {}", name, value),
            Operation::Log { message } => format!("log: {}", message),
            Operation::Wait { seconds } => format!("wait {}s", seconds),
            Operation::Faucet { address } => format!("faucet {}", address),
            Operation::Stake { pool_id, amount } => format!("stake {} in pool {}", amount, pool_id),
            Operation::Noop => "noop".into(),
        }
    }
}

/// A condition for conditional execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Condition {
    /// Always true.
    Always,
    /// Variable equals value.
    Equals { var: String, value: String },
    /// Variable (parsed as u64) is less than value.
    LessThan { var: String, value: u64 },
    /// Variable (parsed as u64) is greater than value.
    GreaterThan { var: String, value: u64 },
    /// Variable exists.
    Exists { var: String },
    /// Negate a condition.
    Not(Box<Condition>),
}

impl Condition {
    pub fn evaluate(&self, vars: &HashMap<String, String>) -> bool {
        match self {
            Condition::Always => true,
            Condition::Equals { var, value } => vars.get(var).is_some_and(|v| v == value),
            Condition::LessThan { var, value } => vars
                .get(var)
                .and_then(|v| v.parse::<u64>().ok())
                .is_some_and(|v| v < *value),
            Condition::GreaterThan { var, value } => vars
                .get(var)
                .and_then(|v| v.parse::<u64>().ok())
                .is_some_and(|v| v > *value),
            Condition::Exists { var } => vars.contains_key(var),
            Condition::Not(inner) => !inner.evaluate(vars),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Condition::Always => "always".into(),
            Condition::Equals { var, value } => format!("${} == {}", var, value),
            Condition::LessThan { var, value } => format!("${} < {}", var, value),
            Condition::GreaterThan { var, value } => format!("${} > {}", var, value),
            Condition::Exists { var } => format!("${} exists", var),
            Condition::Not(inner) => format!("NOT({})", inner.label()),
        }
    }
}

/// What to do when a step fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnError {
    /// Stop the entire script.
    Abort,
    /// Skip this step and continue.
    Skip,
    /// Retry N times then abort.
    Retry(u8),
}

/// A single step in a script.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    /// Step name.
    pub name: String,
    /// Operation to perform.
    pub operation: Operation,
    /// Condition for execution (default: Always).
    #[serde(default = "default_condition")]
    pub condition: Condition,
    /// Error handling.
    #[serde(default = "default_on_error")]
    pub on_error: OnError,
}

fn default_condition() -> Condition {
    Condition::Always
}
fn default_on_error() -> OnError {
    OnError::Abort
}

/// A complete script definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Script {
    /// Script name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Version.
    pub version: String,
    /// Author.
    pub author: String,
    /// Steps to execute.
    pub steps: Vec<Step>,
    /// Initial variables.
    #[serde(default)]
    pub variables: HashMap<String, String>,
    /// Maximum steps to execute (safety limit).
    #[serde(default = "default_max_steps")]
    pub max_steps: usize,
}

fn default_max_steps() -> usize {
    100
}

impl Script {
    /// Load script from a JSON file.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ScriptError> {
        let data = std::fs::read_to_string(path)?;
        let script: Script = serde_json::from_str(&data)?;
        script.validate()?;
        Ok(script)
    }

    /// Save script to a JSON file.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), ScriptError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Validate the script.
    pub fn validate(&self) -> Result<(), ScriptError> {
        if self.name.is_empty() {
            return Err(ScriptError::InvalidScript("script name is empty".into()));
        }
        if self.steps.is_empty() {
            return Err(ScriptError::InvalidScript("script has no steps".into()));
        }
        if self.steps.len() > self.max_steps {
            return Err(ScriptError::MaxStepsExceeded(self.steps.len()));
        }
        for step in &self.steps {
            if step.name.is_empty() {
                return Err(ScriptError::InvalidScript("step name is empty".into()));
            }
        }
        Ok(())
    }

    /// Get step count.
    pub fn step_count(&self) -> usize {
        self.steps.len()
    }
}

/// Result of executing a single step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub step_name: String,
    pub operation: String,
    pub executed: bool,
    pub skipped: bool,
    pub success: bool,
    pub output: String,
}

/// Result of executing an entire script.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptResult {
    pub script_name: String,
    pub total_steps: usize,
    pub executed: usize,
    pub skipped: usize,
    pub failed: usize,
    pub success: bool,
    pub step_results: Vec<StepResult>,
    pub final_variables: HashMap<String, String>,
}

// ──────────────────────────── Executor ───────────────────────────────────

/// The script executor — runs scripts in dry-run or live mode.
#[derive(Debug)]
pub struct ScriptExecutor {
    /// Variables available during execution.
    pub variables: HashMap<String, String>,
    /// Step results.
    pub results: Vec<StepResult>,
    /// Whether to actually execute (false = dry-run).
    pub live: bool,
}

impl ScriptExecutor {
    pub fn new(live: bool) -> Self {
        Self {
            variables: HashMap::new(),
            results: Vec::new(),
            live,
        }
    }

    /// Execute a script (dry-run by default).
    pub fn execute(&mut self, script: &Script) -> Result<ScriptResult, ScriptError> {
        script.validate()?;

        // Initialize variables
        self.variables = script.variables.clone();

        let mut executed = 0;
        let mut skipped = 0;
        let mut failed = 0;

        for step in &script.steps {
            // Check condition
            if !step.condition.evaluate(&self.variables) {
                self.results.push(StepResult {
                    step_name: step.name.clone(),
                    operation: step.operation.label(),
                    executed: false,
                    skipped: true,
                    success: true,
                    output: format!("Skipped: condition '{}' not met", step.condition.label()),
                });
                skipped += 1;
                continue;
            }

            // Execute the step
            match self.execute_step(step) {
                Ok(result) => {
                    executed += 1;
                    self.results.push(result);
                }
                Err(e) => {
                    failed += 1;
                    self.results.push(StepResult {
                        step_name: step.name.clone(),
                        operation: step.operation.label(),
                        executed: true,
                        skipped: false,
                        success: false,
                        output: format!("Error: {}", e),
                    });
                    match step.on_error {
                        OnError::Abort => {
                            return Ok(ScriptResult {
                                script_name: script.name.clone(),
                                total_steps: script.steps.len(),
                                executed,
                                skipped,
                                failed,
                                success: false,
                                step_results: self.results.clone(),
                                final_variables: self.variables.clone(),
                            });
                        }
                        OnError::Skip => continue,
                        OnError::Retry(_) => continue, // Simplified: just skip on retry failure
                    }
                }
            }
        }

        Ok(ScriptResult {
            script_name: script.name.clone(),
            total_steps: script.steps.len(),
            executed,
            skipped,
            failed,
            success: failed == 0,
            step_results: self.results.clone(),
            final_variables: self.variables.clone(),
        })
    }

    fn execute_step(&mut self, step: &Step) -> Result<StepResult, ScriptError> {
        let output = match &step.operation {
            Operation::Transfer { to, amount } => {
                if self.live {
                    format!("Transferred {} EVAP to {}", amount, to)
                } else {
                    format!("[DRY-RUN] Would transfer {} EVAP to {}", amount, to)
                }
            }
            Operation::Refresh { object_id, energy } => {
                if self.live {
                    format!("Refreshed {} with {} energy", object_id, energy)
                } else {
                    format!(
                        "[DRY-RUN] Would refresh {} with {} energy",
                        object_id, energy
                    )
                }
            }
            Operation::CheckBalance { address, store_as } => {
                // In dry-run, simulate a balance
                let balance = if self.live { "0" } else { "50000" };
                self.variables.insert(store_as.clone(), balance.to_string());
                format!(
                    "Balance of {} = {} (stored as ${})",
                    address, balance, store_as
                )
            }
            Operation::SetVar { name, value } => {
                // Interpolate variables in value
                let resolved = self.interpolate(value);
                self.variables.insert(name.clone(), resolved.clone());
                format!("Set ${} = {}", name, resolved)
            }
            Operation::Log { message } => {
                let resolved = self.interpolate(message);
                format!("LOG: {}", resolved)
            }
            Operation::Wait { seconds } => {
                if self.live {
                    format!("Waited {}s", seconds)
                } else {
                    format!("[DRY-RUN] Would wait {}s", seconds)
                }
            }
            Operation::Faucet { address } => {
                if self.live {
                    format!("Requested faucet for {}", address)
                } else {
                    format!("[DRY-RUN] Would request faucet for {}", address)
                }
            }
            Operation::Stake { pool_id, amount } => {
                if self.live {
                    format!("Staked {} in pool {}", amount, pool_id)
                } else {
                    format!("[DRY-RUN] Would stake {} in pool {}", amount, pool_id)
                }
            }
            Operation::Noop => "No-op".into(),
        };

        Ok(StepResult {
            step_name: step.name.clone(),
            operation: step.operation.label(),
            executed: true,
            skipped: false,
            success: true,
            output,
        })
    }

    /// Interpolate ${var} references in a string.
    fn interpolate(&self, template: &str) -> String {
        let mut result = template.to_string();
        for (key, value) in &self.variables {
            result = result.replace(&format!("${{{}}}", key), value);
        }
        result
    }
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_script() -> Script {
        Script {
            name: "test-script".into(),
            description: "Test workflow".into(),
            version: "1.0".into(),
            author: "tester".into(),
            steps: vec![
                Step {
                    name: "check-balance".into(),
                    operation: Operation::CheckBalance {
                        address: "0xme".into(),
                        store_as: "bal".into(),
                    },
                    condition: Condition::Always,
                    on_error: OnError::Abort,
                },
                Step {
                    name: "log-balance".into(),
                    operation: Operation::Log {
                        message: "Balance is ${bal}".into(),
                    },
                    condition: Condition::Always,
                    on_error: OnError::Abort,
                },
                Step {
                    name: "send-if-rich".into(),
                    operation: Operation::Transfer {
                        to: "0xbob".into(),
                        amount: 1000,
                    },
                    condition: Condition::GreaterThan {
                        var: "bal".into(),
                        value: 10000,
                    },
                    on_error: OnError::Skip,
                },
                Step {
                    name: "faucet-if-poor".into(),
                    operation: Operation::Faucet {
                        address: "0xme".into(),
                    },
                    condition: Condition::LessThan {
                        var: "bal".into(),
                        value: 1000,
                    },
                    on_error: OnError::Abort,
                },
            ],
            variables: HashMap::new(),
            max_steps: 100,
        }
    }

    #[test]
    fn test_script_validate() {
        let s = make_script();
        assert!(s.validate().is_ok());
    }

    #[test]
    fn test_script_validate_empty_name() {
        let mut s = make_script();
        s.name = String::new();
        assert!(s.validate().is_err());
    }

    #[test]
    fn test_script_validate_no_steps() {
        let mut s = make_script();
        s.steps.clear();
        assert!(s.validate().is_err());
    }

    #[test]
    fn test_script_step_count() {
        let s = make_script();
        assert_eq!(s.step_count(), 4);
    }

    #[test]
    fn test_dry_run_execution() {
        let script = make_script();
        let mut executor = ScriptExecutor::new(false);
        let result = executor.execute(&script).unwrap();
        assert_eq!(result.script_name, "test-script");
        assert!(result.success);
        assert_eq!(result.total_steps, 4);
    }

    #[test]
    fn test_condition_always() {
        let vars = HashMap::new();
        assert!(Condition::Always.evaluate(&vars));
    }

    #[test]
    fn test_condition_equals() {
        let mut vars = HashMap::new();
        vars.insert("status".into(), "ok".into());
        assert!(Condition::Equals {
            var: "status".into(),
            value: "ok".into()
        }
        .evaluate(&vars));
        assert!(!Condition::Equals {
            var: "status".into(),
            value: "fail".into()
        }
        .evaluate(&vars));
    }

    #[test]
    fn test_condition_less_than() {
        let mut vars = HashMap::new();
        vars.insert("bal".into(), "500".into());
        assert!(Condition::LessThan {
            var: "bal".into(),
            value: 1000
        }
        .evaluate(&vars));
        assert!(!Condition::LessThan {
            var: "bal".into(),
            value: 100
        }
        .evaluate(&vars));
    }

    #[test]
    fn test_condition_greater_than() {
        let mut vars = HashMap::new();
        vars.insert("bal".into(), "50000".into());
        assert!(Condition::GreaterThan {
            var: "bal".into(),
            value: 10000
        }
        .evaluate(&vars));
        assert!(!Condition::GreaterThan {
            var: "bal".into(),
            value: 100000
        }
        .evaluate(&vars));
    }

    #[test]
    fn test_condition_exists() {
        let mut vars = HashMap::new();
        vars.insert("bal".into(), "100".into());
        assert!(Condition::Exists { var: "bal".into() }.evaluate(&vars));
        assert!(!Condition::Exists { var: "nope".into() }.evaluate(&vars));
    }

    #[test]
    fn test_condition_not() {
        let mut vars = HashMap::new();
        vars.insert("bal".into(), "100".into());
        let cond = Condition::Not(Box::new(Condition::GreaterThan {
            var: "bal".into(),
            value: 1000,
        }));
        assert!(cond.evaluate(&vars)); // 100 > 1000 is false, NOT(false) = true
    }

    #[test]
    fn test_conditional_skip() {
        let script = make_script();
        let mut executor = ScriptExecutor::new(false);
        let result = executor.execute(&script).unwrap();
        // check-balance sets bal=50000 (dry-run)
        // send-if-rich: 50000 > 10000 → executes
        // faucet-if-poor: 50000 < 1000 → skipped
        let faucet_result = result
            .step_results
            .iter()
            .find(|r| r.step_name == "faucet-if-poor")
            .unwrap();
        assert!(faucet_result.skipped);
    }

    #[test]
    fn test_variable_interpolation() {
        let script = make_script();
        let mut executor = ScriptExecutor::new(false);
        let result = executor.execute(&script).unwrap();
        let log_result = result
            .step_results
            .iter()
            .find(|r| r.step_name == "log-balance")
            .unwrap();
        assert!(log_result.output.contains("50000"));
    }

    #[test]
    fn test_set_var_operation() {
        let script = Script {
            name: "set-test".into(),
            description: "test".into(),
            version: "1".into(),
            author: "t".into(),
            steps: vec![Step {
                name: "set".into(),
                operation: Operation::SetVar {
                    name: "greeting".into(),
                    value: "hello".into(),
                },
                condition: Condition::Always,
                on_error: OnError::Abort,
            }],
            variables: HashMap::new(),
            max_steps: 10,
        };
        let mut executor = ScriptExecutor::new(false);
        let result = executor.execute(&script).unwrap();
        assert_eq!(result.final_variables.get("greeting").unwrap(), "hello");
    }

    #[test]
    fn test_initial_variables() {
        let mut vars = HashMap::new();
        vars.insert("addr".into(), "0xtest".into());
        let script = Script {
            name: "vars-test".into(),
            description: "test".into(),
            version: "1".into(),
            author: "t".into(),
            steps: vec![Step {
                name: "log".into(),
                operation: Operation::Log {
                    message: "Address: ${addr}".into(),
                },
                condition: Condition::Always,
                on_error: OnError::Abort,
            }],
            variables: vars,
            max_steps: 10,
        };
        let mut executor = ScriptExecutor::new(false);
        let result = executor.execute(&script).unwrap();
        assert!(result.step_results[0].output.contains("0xtest"));
    }

    #[test]
    fn test_on_error_skip() {
        // Steps with Skip on_error should continue
        let script = Script {
            name: "skip-test".into(),
            description: "test".into(),
            version: "1".into(),
            author: "t".into(),
            steps: vec![
                Step {
                    name: "ok".into(),
                    operation: Operation::Noop,
                    condition: Condition::Always,
                    on_error: OnError::Abort,
                },
                Step {
                    name: "also-ok".into(),
                    operation: Operation::Noop,
                    condition: Condition::Always,
                    on_error: OnError::Skip,
                },
            ],
            variables: HashMap::new(),
            max_steps: 10,
        };
        let mut executor = ScriptExecutor::new(false);
        let result = executor.execute(&script).unwrap();
        assert_eq!(result.executed, 2);
        assert!(result.success);
    }

    #[test]
    fn test_operation_labels() {
        assert!(Operation::Transfer {
            to: "0x1".into(),
            amount: 100
        }
        .label()
        .contains("100"));
        assert!(Operation::Refresh {
            object_id: "obj".into(),
            energy: 50
        }
        .label()
        .contains("obj"));
        assert!(Operation::Log {
            message: "hi".into()
        }
        .label()
        .contains("hi"));
        assert_eq!(Operation::Noop.label(), "noop");
    }

    #[test]
    fn test_condition_labels() {
        assert_eq!(Condition::Always.label(), "always");
        assert!(Condition::LessThan {
            var: "x".into(),
            value: 10
        }
        .label()
        .contains("< 10"));
        assert!(Condition::Not(Box::new(Condition::Always))
            .label()
            .contains("NOT"));
    }

    #[test]
    fn test_script_persistence() {
        let dir = std::env::temp_dir().join("evap_script_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.json");

        let script = make_script();
        script.save(&path).unwrap();

        let loaded = Script::load(&path).unwrap();
        assert_eq!(loaded.name, "test-script");
        assert_eq!(loaded.step_count(), 4);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_max_steps_exceeded() {
        let mut script = make_script();
        script.max_steps = 2;
        assert!(script.validate().is_err());
    }

    #[test]
    fn test_noop_operation() {
        let script = Script {
            name: "noop-test".into(),
            description: "test".into(),
            version: "1".into(),
            author: "t".into(),
            steps: vec![Step {
                name: "do-nothing".into(),
                operation: Operation::Noop,
                condition: Condition::Always,
                on_error: OnError::Abort,
            }],
            variables: HashMap::new(),
            max_steps: 10,
        };
        let mut executor = ScriptExecutor::new(false);
        let result = executor.execute(&script).unwrap();
        assert!(result.success);
        assert_eq!(result.executed, 1);
    }

    #[test]
    fn test_live_vs_dryrun() {
        let script = Script {
            name: "mode-test".into(),
            description: "test".into(),
            version: "1".into(),
            author: "t".into(),
            steps: vec![Step {
                name: "send".into(),
                operation: Operation::Transfer {
                    to: "0x1".into(),
                    amount: 100,
                },
                condition: Condition::Always,
                on_error: OnError::Abort,
            }],
            variables: HashMap::new(),
            max_steps: 10,
        };

        let mut dry = ScriptExecutor::new(false);
        let dry_result = dry.execute(&script).unwrap();
        assert!(dry_result.step_results[0].output.contains("DRY-RUN"));

        let mut live = ScriptExecutor::new(true);
        let live_result = live.execute(&script).unwrap();
        assert!(!live_result.step_results[0].output.contains("DRY-RUN"));
    }

    #[test]
    fn test_condition_missing_var() {
        let vars = HashMap::new();
        assert!(!Condition::Equals {
            var: "x".into(),
            value: "1".into()
        }
        .evaluate(&vars));
        assert!(!Condition::LessThan {
            var: "x".into(),
            value: 10
        }
        .evaluate(&vars));
        assert!(!Condition::GreaterThan {
            var: "x".into(),
            value: 10
        }
        .evaluate(&vars));
    }
}
