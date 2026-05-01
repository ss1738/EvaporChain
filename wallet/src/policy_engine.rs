//! Composable Policy Engine — chain rules into policies that gate transactions.
//!
//! Combine spending limits, reputation checks, time locks, amount thresholds,
//! and address conditions into named policies. Evaluate any pending transaction
//! against all active policies before execution.

use std::path::Path;

use serde::{Deserialize, Serialize};

// ──────────────────────────── Types ──────────────────────────────────────

#[derive(Debug, Clone, thiserror::Error)]
pub enum PolicyError {
    #[error("policy not found: {0}")]
    NotFound(String),
    #[error("policy already exists: {0}")]
    AlreadyExists(String),
    #[error("policy violation: {0}")]
    Violation(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
}

impl From<std::io::Error> for PolicyError {
    fn from(e: std::io::Error) -> Self {
        PolicyError::Io(e.to_string())
    }
}
impl From<serde_json::Error> for PolicyError {
    fn from(e: serde_json::Error) -> Self {
        PolicyError::Json(e.to_string())
    }
}

/// A single rule condition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    /// Block if amount exceeds threshold.
    MaxAmount(u64),
    /// Block if amount is below minimum (anti-dust).
    MinAmount(u64),
    /// Block if recipient is not in the allowed list.
    AllowedRecipients(Vec<String>),
    /// Block if recipient IS in the blocked list.
    BlockedRecipients(Vec<String>),
    /// Block during certain hours (24h format, e.g. 22..6 means 10pm to 6am).
    TimeRestriction { deny_after: u8, deny_before: u8 },
    /// Block if recipient has risk score above threshold.
    MaxRiskScore(u8),
    /// Block if recipient trust level is below this (as string).
    MinTrustLevel(String),
    /// Require N confirmations / approvals.
    RequireApprovals(usize),
    /// Block specific transaction types.
    BlockTxTypes(Vec<String>),
    /// Rate limit: max N transactions in the window (seconds).
    RateLimit { max_count: u64, window_secs: u64 },
    /// Custom rule with a description.
    Custom(String),
}

impl Rule {
    pub fn label(&self) -> String {
        match self {
            Rule::MaxAmount(n) => format!("max_amount <= {}", n),
            Rule::MinAmount(n) => format!("min_amount >= {}", n),
            Rule::AllowedRecipients(list) => format!("allowed_recipients({})", list.len()),
            Rule::BlockedRecipients(list) => format!("blocked_recipients({})", list.len()),
            Rule::TimeRestriction {
                deny_after,
                deny_before,
            } => format!("time_lock({}h-{}h blocked)", deny_after, deny_before),
            Rule::MaxRiskScore(s) => format!("max_risk_score <= {}", s),
            Rule::MinTrustLevel(l) => format!("min_trust >= {}", l),
            Rule::RequireApprovals(n) => format!("require_approvals({})", n),
            Rule::BlockTxTypes(types) => format!("block_types({:?})", types),
            Rule::RateLimit {
                max_count,
                window_secs,
            } => format!("rate_limit({}/{}s)", max_count, window_secs),
            Rule::Custom(desc) => format!("custom({})", desc),
        }
    }
}

/// How rules in a policy are combined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CombineMode {
    /// ALL rules must pass.
    All,
    /// ANY rule failure blocks.
    Any,
}

/// What to do when a policy is violated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Enforcement {
    /// Hard block — transaction cannot proceed.
    Block,
    /// Warn but allow.
    Warn,
    /// Log only.
    Log,
}

/// A named policy containing rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    /// Policy name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Rules to evaluate.
    pub rules: Vec<Rule>,
    /// How rules are combined.
    pub combine: CombineMode,
    /// What to do on violation.
    pub enforcement: Enforcement,
    /// Whether active.
    pub enabled: bool,
    /// Priority (lower = evaluated first).
    pub priority: u32,
    /// Created at.
    pub created_at: String,
}

/// Context for evaluating a transaction against policies.
#[derive(Debug, Clone)]
pub struct TxContext {
    pub tx_type: String,
    pub recipient: String,
    pub amount: u64,
    pub sender: String,
    pub risk_score: Option<u8>,
    pub trust_level: Option<String>,
    pub approvals: usize,
    pub hour: u8,
    pub recent_tx_count: u64,
}

impl TxContext {
    pub fn new(tx_type: &str, recipient: &str, amount: u64, sender: &str) -> Self {
        let hour = chrono::Utc::now()
            .format("%H")
            .to_string()
            .parse()
            .unwrap_or(12);
        Self {
            tx_type: tx_type.to_string(),
            recipient: recipient.to_string(),
            amount,
            sender: sender.to_string(),
            risk_score: None,
            trust_level: None,
            approvals: 0,
            hour,
            recent_tx_count: 0,
        }
    }
}

/// Result of policy evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyResult {
    pub policy_name: String,
    pub passed: bool,
    pub enforcement: Enforcement,
    pub violations: Vec<String>,
    pub warnings: Vec<String>,
}

// ──────────────────────────── Engine ─────────────────────────────────────

/// The policy engine — manages and evaluates policies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyEngine {
    pub policies: Vec<Policy>,
}

impl PolicyEngine {
    pub fn new() -> Self {
        Self {
            policies: Vec::new(),
        }
    }

    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, PolicyError> {
        let data = std::fs::read_to_string(path)?;
        let engine: PolicyEngine = serde_json::from_str(&data)?;
        Ok(engine)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), PolicyError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Add a policy.
    pub fn add_policy(&mut self, policy: Policy) -> Result<(), PolicyError> {
        if self.policies.iter().any(|p| p.name == policy.name) {
            return Err(PolicyError::AlreadyExists(policy.name));
        }
        self.policies.push(policy);
        self.policies.sort_by_key(|p| p.priority);
        Ok(())
    }

    /// Remove a policy.
    pub fn remove_policy(&mut self, name: &str) -> Result<(), PolicyError> {
        let idx = self
            .policies
            .iter()
            .position(|p| p.name == name)
            .ok_or_else(|| PolicyError::NotFound(name.to_string()))?;
        self.policies.remove(idx);
        Ok(())
    }

    /// Enable a policy.
    pub fn enable(&mut self, name: &str) -> Result<(), PolicyError> {
        let p = self
            .policies
            .iter_mut()
            .find(|p| p.name == name)
            .ok_or_else(|| PolicyError::NotFound(name.to_string()))?;
        p.enabled = true;
        Ok(())
    }

    /// Disable a policy.
    pub fn disable(&mut self, name: &str) -> Result<(), PolicyError> {
        let p = self
            .policies
            .iter_mut()
            .find(|p| p.name == name)
            .ok_or_else(|| PolicyError::NotFound(name.to_string()))?;
        p.enabled = false;
        Ok(())
    }

    /// Get a policy by name.
    pub fn get(&self, name: &str) -> Option<&Policy> {
        self.policies.iter().find(|p| p.name == name)
    }

    /// List all policies.
    pub fn list(&self) -> &[Policy] {
        &self.policies
    }

    /// Evaluate a transaction against all active policies.
    pub fn evaluate(&self, ctx: &TxContext) -> Vec<PolicyResult> {
        self.policies
            .iter()
            .filter(|p| p.enabled)
            .map(|p| self.evaluate_policy(p, ctx))
            .collect()
    }

    /// Check if a transaction is blocked by any policy.
    pub fn is_blocked(&self, ctx: &TxContext) -> Option<PolicyResult> {
        self.evaluate(ctx)
            .into_iter()
            .find(|r| !r.passed && r.enforcement == Enforcement::Block)
    }

    /// Get all warnings (non-blocking violations).
    pub fn get_warnings(&self, ctx: &TxContext) -> Vec<PolicyResult> {
        self.evaluate(ctx)
            .into_iter()
            .filter(|r| !r.passed && r.enforcement == Enforcement::Warn)
            .collect()
    }

    /// Total number of policies.
    pub fn count(&self) -> usize {
        self.policies.len()
    }

    /// Active policy count.
    pub fn active_count(&self) -> usize {
        self.policies.iter().filter(|p| p.enabled).count()
    }

    fn evaluate_policy(&self, policy: &Policy, ctx: &TxContext) -> PolicyResult {
        let mut violations = Vec::new();

        for rule in &policy.rules {
            if let Some(violation) = self.check_rule(rule, ctx) {
                violations.push(violation);
            }
        }

        let passed = match policy.combine {
            CombineMode::All => violations.is_empty(),
            CombineMode::Any => violations.is_empty(),
        };

        let warnings = if !passed && policy.enforcement == Enforcement::Warn {
            violations.clone()
        } else {
            vec![]
        };

        PolicyResult {
            policy_name: policy.name.clone(),
            passed,
            enforcement: policy.enforcement,
            violations,
            warnings,
        }
    }

    fn check_rule(&self, rule: &Rule, ctx: &TxContext) -> Option<String> {
        match rule {
            Rule::MaxAmount(max) => {
                if ctx.amount > *max {
                    Some(format!("Amount {} exceeds max {}", ctx.amount, max))
                } else {
                    None
                }
            }
            Rule::MinAmount(min) => {
                if ctx.amount < *min {
                    Some(format!("Amount {} below min {}", ctx.amount, min))
                } else {
                    None
                }
            }
            Rule::AllowedRecipients(list) => {
                let r = ctx.recipient.to_lowercase();
                if !list.iter().any(|a| a.to_lowercase() == r) {
                    Some(format!("Recipient {} not in allowed list", ctx.recipient))
                } else {
                    None
                }
            }
            Rule::BlockedRecipients(list) => {
                let r = ctx.recipient.to_lowercase();
                if list.iter().any(|a| a.to_lowercase() == r) {
                    Some(format!("Recipient {} is blocked", ctx.recipient))
                } else {
                    None
                }
            }
            Rule::TimeRestriction {
                deny_after,
                deny_before,
            } => {
                let h = ctx.hour;
                let blocked = if deny_after < deny_before {
                    // e.g. deny 9..17 (9am to 5pm)
                    h >= *deny_after && h < *deny_before
                } else {
                    // e.g. deny 22..6 (10pm to 6am)
                    h >= *deny_after || h < *deny_before
                };
                if blocked {
                    Some(format!(
                        "Transaction blocked during hours {}h-{}h (current: {}h)",
                        deny_after, deny_before, h
                    ))
                } else {
                    None
                }
            }
            Rule::MaxRiskScore(max) => {
                if let Some(score) = ctx.risk_score {
                    if score > *max {
                        Some(format!(
                            "Recipient risk score {} exceeds max {}",
                            score, max
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            Rule::MinTrustLevel(min_level) => {
                if let Some(ref trust) = ctx.trust_level {
                    let min_lower = min_level.to_lowercase();
                    let trust_lower = trust.to_lowercase();
                    let trust_order = [
                        "dangerous",
                        "suspicious",
                        "unknown",
                        "neutral",
                        "trusted",
                        "verified",
                    ];
                    let min_idx = trust_order
                        .iter()
                        .position(|&l| l == min_lower)
                        .unwrap_or(0);
                    let actual_idx = trust_order
                        .iter()
                        .position(|&l| l == trust_lower)
                        .unwrap_or(0);
                    if actual_idx < min_idx {
                        Some(format!(
                            "Trust level '{}' below minimum '{}'",
                            trust, min_level
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            Rule::RequireApprovals(n) => {
                if ctx.approvals < *n {
                    Some(format!("Requires {} approvals, has {}", n, ctx.approvals))
                } else {
                    None
                }
            }
            Rule::BlockTxTypes(types) => {
                if types
                    .iter()
                    .any(|t| t.to_lowercase() == ctx.tx_type.to_lowercase())
                {
                    Some(format!("Transaction type '{}' is blocked", ctx.tx_type))
                } else {
                    None
                }
            }
            Rule::RateLimit { max_count, .. } => {
                if ctx.recent_tx_count >= *max_count {
                    Some(format!(
                        "Rate limit exceeded: {} transactions (max {})",
                        ctx.recent_tx_count, max_count
                    ))
                } else {
                    None
                }
            }
            Rule::Custom(_desc) => {
                // Custom rules always pass (they're informational)
                None
            }
        }
    }
}

impl Default for PolicyEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Default path.
pub fn default_policy_path() -> std::path::PathBuf {
    crate::config::default_data_dir().join("policies.json")
}

/// Helper to create a policy.
pub fn make_policy(
    name: &str,
    description: &str,
    rules: Vec<Rule>,
    enforcement: Enforcement,
    priority: u32,
) -> Policy {
    Policy {
        name: name.to_string(),
        description: description.to_string(),
        rules,
        combine: CombineMode::All,
        enforcement,
        enabled: true,
        priority,
        created_at: chrono::Utc::now().to_rfc3339(),
    }
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_engine() -> PolicyEngine {
        let mut engine = PolicyEngine::new();
        engine
            .add_policy(make_policy(
                "max-transfer",
                "Block transfers over 10000",
                vec![Rule::MaxAmount(10000)],
                Enforcement::Block,
                1,
            ))
            .unwrap();
        engine
            .add_policy(make_policy(
                "no-scammers",
                "Block transfers to scam addresses",
                vec![Rule::BlockedRecipients(vec!["0xscammer".into()])],
                Enforcement::Block,
                2,
            ))
            .unwrap();
        engine
            .add_policy(make_policy(
                "warn-large",
                "Warn on transfers over 5000",
                vec![Rule::MaxAmount(5000)],
                Enforcement::Warn,
                3,
            ))
            .unwrap();
        engine
    }

    fn make_ctx(amount: u64, recipient: &str) -> TxContext {
        TxContext::new("transfer", recipient, amount, "0xme")
    }

    #[test]
    fn test_add_policy() {
        let engine = make_engine();
        assert_eq!(engine.count(), 3);
    }

    #[test]
    fn test_duplicate_rejected() {
        let mut engine = make_engine();
        let err = engine.add_policy(make_policy(
            "max-transfer",
            "dup",
            vec![],
            Enforcement::Block,
            1,
        ));
        assert!(err.is_err());
    }

    #[test]
    fn test_remove_policy() {
        let mut engine = make_engine();
        engine.remove_policy("max-transfer").unwrap();
        assert_eq!(engine.count(), 2);
    }

    #[test]
    fn test_enable_disable() {
        let mut engine = make_engine();
        engine.disable("max-transfer").unwrap();
        assert!(!engine.get("max-transfer").unwrap().enabled);
        engine.enable("max-transfer").unwrap();
        assert!(engine.get("max-transfer").unwrap().enabled);
    }

    #[test]
    fn test_max_amount_blocks() {
        let engine = make_engine();
        let ctx = make_ctx(15000, "0xsafe");
        let blocked = engine.is_blocked(&ctx);
        assert!(blocked.is_some());
        assert_eq!(blocked.unwrap().policy_name, "max-transfer");
    }

    #[test]
    fn test_max_amount_passes() {
        let engine = make_engine();
        let ctx = make_ctx(5000, "0xsafe");
        let blocked = engine.is_blocked(&ctx);
        assert!(blocked.is_none());
    }

    #[test]
    fn test_blocked_recipient() {
        let engine = make_engine();
        let ctx = make_ctx(100, "0xscammer");
        let blocked = engine.is_blocked(&ctx);
        assert!(blocked.is_some());
        assert_eq!(blocked.unwrap().policy_name, "no-scammers");
    }

    #[test]
    fn test_warning_on_large() {
        let engine = make_engine();
        let ctx = make_ctx(7000, "0xsafe");
        let warnings = engine.get_warnings(&ctx);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].policy_name, "warn-large");
    }

    #[test]
    fn test_all_pass() {
        let engine = make_engine();
        let ctx = make_ctx(1000, "0xsafe");
        let results = engine.evaluate(&ctx);
        assert!(results.iter().all(|r| r.passed));
    }

    #[test]
    fn test_disabled_policy_skipped() {
        let mut engine = make_engine();
        engine.disable("max-transfer").unwrap();
        let ctx = make_ctx(15000, "0xsafe");
        let blocked = engine.is_blocked(&ctx);
        assert!(blocked.is_none()); // max-transfer is disabled
    }

    #[test]
    fn test_min_amount() {
        let mut engine = PolicyEngine::new();
        engine
            .add_policy(make_policy(
                "anti-dust",
                "Block tiny transfers",
                vec![Rule::MinAmount(10)],
                Enforcement::Block,
                1,
            ))
            .unwrap();
        let ctx = make_ctx(5, "0xfoo");
        assert!(engine.is_blocked(&ctx).is_some());
        let ctx2 = make_ctx(100, "0xfoo");
        assert!(engine.is_blocked(&ctx2).is_none());
    }

    #[test]
    fn test_allowed_recipients() {
        let mut engine = PolicyEngine::new();
        engine
            .add_policy(make_policy(
                "whitelist",
                "Only send to known addresses",
                vec![Rule::AllowedRecipients(vec![
                    "0xalice".into(),
                    "0xbob".into(),
                ])],
                Enforcement::Block,
                1,
            ))
            .unwrap();
        let ctx = make_ctx(100, "0xalice");
        assert!(engine.is_blocked(&ctx).is_none());
        let ctx2 = make_ctx(100, "0xcharlie");
        assert!(engine.is_blocked(&ctx2).is_some());
    }

    #[test]
    fn test_time_restriction() {
        let mut engine = PolicyEngine::new();
        // Block after 22h, before 6h
        engine
            .add_policy(make_policy(
                "nighttime",
                "Block nighttime transfers",
                vec![Rule::TimeRestriction {
                    deny_after: 22,
                    deny_before: 6,
                }],
                Enforcement::Block,
                1,
            ))
            .unwrap();
        let mut ctx = make_ctx(100, "0xfoo");
        ctx.hour = 3; // 3am — should be blocked
        assert!(engine.is_blocked(&ctx).is_some());
        ctx.hour = 14; // 2pm — should pass
        assert!(engine.is_blocked(&ctx).is_none());
    }

    #[test]
    fn test_risk_score_check() {
        let mut engine = PolicyEngine::new();
        engine
            .add_policy(make_policy(
                "no-risky",
                "Block risky recipients",
                vec![Rule::MaxRiskScore(50)],
                Enforcement::Block,
                1,
            ))
            .unwrap();
        let mut ctx = make_ctx(100, "0xfoo");
        ctx.risk_score = Some(80);
        assert!(engine.is_blocked(&ctx).is_some());
        ctx.risk_score = Some(30);
        assert!(engine.is_blocked(&ctx).is_none());
    }

    #[test]
    fn test_require_approvals() {
        let mut engine = PolicyEngine::new();
        engine
            .add_policy(make_policy(
                "multisig",
                "Require 2 approvals",
                vec![Rule::RequireApprovals(2)],
                Enforcement::Block,
                1,
            ))
            .unwrap();
        let mut ctx = make_ctx(100, "0xfoo");
        ctx.approvals = 1;
        assert!(engine.is_blocked(&ctx).is_some());
        ctx.approvals = 2;
        assert!(engine.is_blocked(&ctx).is_none());
    }

    #[test]
    fn test_block_tx_types() {
        let mut engine = PolicyEngine::new();
        engine
            .add_policy(make_policy(
                "no-stakes",
                "Block staking",
                vec![Rule::BlockTxTypes(vec!["stake".into(), "unstake".into()])],
                Enforcement::Block,
                1,
            ))
            .unwrap();
        let mut ctx = make_ctx(100, "0xfoo");
        ctx.tx_type = "stake".to_string();
        assert!(engine.is_blocked(&ctx).is_some());
        ctx.tx_type = "transfer".to_string();
        assert!(engine.is_blocked(&ctx).is_none());
    }

    #[test]
    fn test_rate_limit() {
        let mut engine = PolicyEngine::new();
        engine
            .add_policy(make_policy(
                "rate-limit",
                "Max 10 txns per window",
                vec![Rule::RateLimit {
                    max_count: 10,
                    window_secs: 3600,
                }],
                Enforcement::Block,
                1,
            ))
            .unwrap();
        let mut ctx = make_ctx(100, "0xfoo");
        ctx.recent_tx_count = 15;
        assert!(engine.is_blocked(&ctx).is_some());
        ctx.recent_tx_count = 5;
        assert!(engine.is_blocked(&ctx).is_none());
    }

    #[test]
    fn test_multiple_rules_in_policy() {
        let mut engine = PolicyEngine::new();
        engine
            .add_policy(make_policy(
                "combo",
                "Max 10k and no scammers",
                vec![
                    Rule::MaxAmount(10000),
                    Rule::BlockedRecipients(vec!["0xbad".into()]),
                ],
                Enforcement::Block,
                1,
            ))
            .unwrap();
        // Passes both
        let ctx = make_ctx(5000, "0xgood");
        assert!(engine.is_blocked(&ctx).is_none());
        // Fails amount
        let ctx2 = make_ctx(15000, "0xgood");
        assert!(engine.is_blocked(&ctx2).is_some());
        // Fails recipient
        let ctx3 = make_ctx(100, "0xbad");
        assert!(engine.is_blocked(&ctx3).is_some());
    }

    #[test]
    fn test_priority_ordering() {
        let engine = make_engine();
        let policies = engine.list();
        assert!(policies[0].priority <= policies[1].priority);
    }

    #[test]
    fn test_active_count() {
        let mut engine = make_engine();
        assert_eq!(engine.active_count(), 3);
        engine.disable("max-transfer").unwrap();
        assert_eq!(engine.active_count(), 2);
    }

    #[test]
    fn test_rule_labels() {
        assert!(Rule::MaxAmount(1000).label().contains("1000"));
        assert!(Rule::MinAmount(10).label().contains("10"));
        assert!(Rule::RequireApprovals(3).label().contains("3"));
    }

    #[test]
    fn test_persistence_roundtrip() {
        let dir = std::env::temp_dir().join("evap_policy_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("policies.json");

        let engine = make_engine();
        engine.save(&path).unwrap();

        let loaded = PolicyEngine::load(&path).unwrap();
        assert_eq!(loaded.count(), 3);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_default_trait() {
        let engine = PolicyEngine::default();
        assert_eq!(engine.count(), 0);
    }

    #[test]
    fn test_trust_level_check() {
        let mut engine = PolicyEngine::new();
        engine
            .add_policy(make_policy(
                "trust-check",
                "Min trust neutral",
                vec![Rule::MinTrustLevel("neutral".into())],
                Enforcement::Block,
                1,
            ))
            .unwrap();
        let mut ctx = make_ctx(100, "0xfoo");
        ctx.trust_level = Some("suspicious".into());
        assert!(engine.is_blocked(&ctx).is_some());
        ctx.trust_level = Some("trusted".into());
        assert!(engine.is_blocked(&ctx).is_none());
    }
}
