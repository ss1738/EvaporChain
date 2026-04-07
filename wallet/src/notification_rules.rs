// wallet/src/notification_rules.rs — Configurable notification rule engine
//
// Define rules with conditions, actions, and channels. Evaluate incoming
// events against all active rules and emit triggered notifications.
// Supports cooldowns, max triggers, priority filtering, and tag-based lookup.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum NotificationRuleError {
    #[error("rule already exists: {0}")]
    AlreadyExists(String),
    #[error("rule not found: {0}")]
    NotFound(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ──────────────────────────── Enums ────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RuleCondition {
    BalanceAbove(u64),
    BalanceBelow(u64),
    TxAmountAbove(u64),
    EnergyBelow(u32),
    PriceAbove(f64),
    PriceBelow(f64),
    NewIncoming,
    GasAbove(u64),
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuleAction {
    Log(String),
    Webhook(String),
    Email(String),
    Push(String),
    Sound(String),
    AutoRefresh,
    Pause,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Channel {
    Console,
    File,
    Webhook,
    Email,
    Push,
    Sms,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RulePriority {
    Critical,
    High,
    Normal,
    Low,
}

// ──────────────────────────── NotificationRule ──────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationRule {
    pub id: String,
    pub name: String,
    pub description: String,
    pub conditions: Vec<RuleCondition>,
    pub actions: Vec<RuleAction>,
    pub channels: Vec<Channel>,
    pub priority: RulePriority,
    pub enabled: bool,
    pub cooldown_secs: u64,
    pub last_triggered: Option<String>,
    pub trigger_count: u64,
    pub created_at: String,
    pub max_triggers: Option<u64>,
    pub tags: Vec<String>,
}

impl NotificationRule {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            conditions: Vec::new(),
            actions: Vec::new(),
            channels: Vec::new(),
            priority: RulePriority::Normal,
            enabled: true,
            cooldown_secs: 300,
            last_triggered: None,
            trigger_count: 0,
            created_at: chrono::Utc::now().to_rfc3339(),
            max_triggers: None,
            tags: Vec::new(),
        }
    }

    pub fn with_condition(mut self, c: RuleCondition) -> Self {
        self.conditions.push(c);
        self
    }

    pub fn with_action(mut self, a: RuleAction) -> Self {
        self.actions.push(a);
        self
    }

    pub fn with_channel(mut self, ch: Channel) -> Self {
        self.channels.push(ch);
        self
    }

    pub fn with_priority(mut self, p: RulePriority) -> Self {
        self.priority = p;
        self
    }

    pub fn with_cooldown(mut self, secs: u64) -> Self {
        self.cooldown_secs = secs;
        self
    }

    pub fn with_max_triggers(mut self, max: u64) -> Self {
        self.max_triggers = Some(max);
        self
    }

    pub fn with_tag(mut self, tag: &str) -> Self {
        self.tags.push(tag.to_string());
        self
    }

    /// Returns true if the rule is enabled and has not exceeded max triggers.
    pub fn is_active(&self) -> bool {
        self.enabled
            && match self.max_triggers {
                Some(max) => self.trigger_count < max,
                None => true,
            }
    }

    /// Returns true if the rule is still within its cooldown window.
    pub fn is_in_cooldown(&self) -> bool {
        match &self.last_triggered {
            Some(ts) => {
                if let Ok(last) = chrono::DateTime::parse_from_rfc3339(ts) {
                    let elapsed = chrono::Utc::now()
                        .signed_duration_since(last)
                        .num_seconds();
                    elapsed < self.cooldown_secs as i64
                } else {
                    false
                }
            }
            None => false,
        }
    }

    /// Returns true if the rule can fire right now.
    pub fn can_trigger(&self) -> bool {
        self.is_active() && !self.is_in_cooldown()
    }

    /// Evaluate all conditions against the given context (AND logic).
    pub fn evaluate(&self, context: &RuleContext) -> bool {
        self.conditions
            .iter()
            .all(|c| context.matches_condition(c))
    }

    /// Record a trigger event.
    pub fn trigger(&mut self) {
        self.trigger_count += 1;
        self.last_triggered = Some(chrono::Utc::now().to_rfc3339());
    }

    /// Reset trigger state.
    pub fn reset(&mut self) {
        self.trigger_count = 0;
        self.last_triggered = None;
    }

    pub fn disable(&mut self) {
        self.enabled = false;
    }

    pub fn enable(&mut self) {
        self.enabled = true;
    }
}

// ──────────────────────────── RuleContext ───────────────────────────────

#[derive(Debug, Clone)]
pub struct RuleContext {
    pub balance: Option<u64>,
    pub tx_amount: Option<u64>,
    pub energy: Option<u32>,
    pub price: Option<f64>,
    pub gas: Option<u64>,
    pub has_incoming: bool,
    pub custom: HashMap<String, String>,
}

impl RuleContext {
    pub fn new() -> Self {
        Self {
            balance: None,
            tx_amount: None,
            energy: None,
            price: None,
            gas: None,
            has_incoming: false,
            custom: HashMap::new(),
        }
    }

    pub fn with_balance(mut self, b: u64) -> Self {
        self.balance = Some(b);
        self
    }

    pub fn with_tx_amount(mut self, a: u64) -> Self {
        self.tx_amount = Some(a);
        self
    }

    pub fn with_energy(mut self, e: u32) -> Self {
        self.energy = Some(e);
        self
    }

    pub fn with_price(mut self, p: f64) -> Self {
        self.price = Some(p);
        self
    }

    pub fn with_gas(mut self, g: u64) -> Self {
        self.gas = Some(g);
        self
    }

    pub fn with_incoming(mut self) -> Self {
        self.has_incoming = true;
        self
    }

    pub fn with_custom(mut self, key: &str, value: &str) -> Self {
        self.custom.insert(key.to_string(), value.to_string());
        self
    }

    /// Evaluate a single condition against this context.
    pub fn matches_condition(&self, condition: &RuleCondition) -> bool {
        match condition {
            RuleCondition::BalanceAbove(threshold) => {
                self.balance.map_or(false, |b| b > *threshold)
            }
            RuleCondition::BalanceBelow(threshold) => {
                self.balance.map_or(false, |b| b < *threshold)
            }
            RuleCondition::TxAmountAbove(threshold) => {
                self.tx_amount.map_or(false, |a| a > *threshold)
            }
            RuleCondition::EnergyBelow(threshold) => {
                self.energy.map_or(false, |e| e < *threshold)
            }
            RuleCondition::PriceAbove(threshold) => {
                self.price.map_or(false, |p| p > *threshold)
            }
            RuleCondition::PriceBelow(threshold) => {
                self.price.map_or(false, |p| p < *threshold)
            }
            RuleCondition::NewIncoming => self.has_incoming,
            RuleCondition::GasAbove(threshold) => {
                self.gas.map_or(false, |g| g > *threshold)
            }
            RuleCondition::Custom(key) => self.custom.contains_key(key),
        }
    }
}

impl Default for RuleContext {
    fn default() -> Self {
        Self::new()
    }
}

// ──────────────────────────── TriggeredNotification ─────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggeredNotification {
    pub rule_id: String,
    pub rule_name: String,
    pub priority: RulePriority,
    pub actions: Vec<RuleAction>,
    pub channels: Vec<Channel>,
    pub triggered_at: String,
    pub message: String,
}

// ──────────────────────────── EngineStats ───────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct EngineStats {
    pub total_rules: usize,
    pub active_rules: usize,
    pub disabled_rules: usize,
    pub total_notifications: usize,
    pub rules_in_cooldown: usize,
}

// ──────────────────────────── RuleEngine ────────────────────────────────

const MAX_HISTORY: usize = 500;

#[derive(Debug, Serialize, Deserialize)]
pub struct RuleEngine {
    pub rules: HashMap<String, NotificationRule>,
    pub history: Vec<TriggeredNotification>,
}

impl Default for RuleEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl RuleEngine {
    pub fn new() -> Self {
        Self {
            rules: HashMap::new(),
            history: Vec::new(),
        }
    }

    pub fn add_rule(&mut self, rule: NotificationRule) -> Result<(), NotificationRuleError> {
        if self.rules.contains_key(&rule.id) {
            return Err(NotificationRuleError::AlreadyExists(rule.id));
        }
        self.rules.insert(rule.id.clone(), rule);
        Ok(())
    }

    pub fn remove_rule(&mut self, id: &str) -> Result<NotificationRule, NotificationRuleError> {
        self.rules
            .remove(id)
            .ok_or_else(|| NotificationRuleError::NotFound(id.to_string()))
    }

    pub fn get_rule(&self, id: &str) -> Option<&NotificationRule> {
        self.rules.get(id)
    }

    pub fn get_rule_mut(&mut self, id: &str) -> Option<&mut NotificationRule> {
        self.rules.get_mut(id)
    }

    pub fn list_rules(&self) -> Vec<&NotificationRule> {
        self.rules.values().collect()
    }

    pub fn active_rules(&self) -> Vec<&NotificationRule> {
        self.rules.values().filter(|r| r.is_active()).collect()
    }

    /// Evaluate all active rules against the context. Trigger matching rules
    /// and return their notifications.
    pub fn evaluate_all(&mut self, context: &RuleContext) -> Vec<TriggeredNotification> {
        let matching_ids: Vec<String> = self
            .rules
            .values()
            .filter(|r| r.can_trigger() && r.evaluate(context))
            .map(|r| r.id.clone())
            .collect();

        let mut notifications = Vec::new();
        for id in &matching_ids {
            if let Some(rule) = self.rules.get_mut(id) {
                rule.trigger();
                let notif = TriggeredNotification {
                    rule_id: rule.id.clone(),
                    rule_name: rule.name.clone(),
                    priority: rule.priority,
                    actions: rule.actions.clone(),
                    channels: rule.channels.clone(),
                    triggered_at: chrono::Utc::now().to_rfc3339(),
                    message: format!("Rule '{}' triggered", rule.name),
                };
                notifications.push(notif);
            }
        }

        for notif in &notifications {
            self.history.push(notif.clone());
        }
        // Prune oldest if history exceeds limit.
        while self.history.len() > MAX_HISTORY {
            self.history.remove(0);
        }

        notifications
    }

    pub fn by_priority(&self, priority: &RulePriority) -> Vec<&NotificationRule> {
        self.rules
            .values()
            .filter(|r| r.priority == *priority)
            .collect()
    }

    pub fn by_tag(&self, tag: &str) -> Vec<&NotificationRule> {
        self.rules
            .values()
            .filter(|r| r.tags.iter().any(|t| t == tag))
            .collect()
    }

    pub fn recent_notifications(&self, n: usize) -> Vec<&TriggeredNotification> {
        self.history.iter().rev().take(n).collect()
    }

    pub fn notification_count(&self) -> usize {
        self.history.len()
    }

    pub fn clear_history(&mut self) -> usize {
        let count = self.history.len();
        self.history.clear();
        count
    }

    pub fn stats(&self) -> EngineStats {
        let total_rules = self.rules.len();
        let active_rules = self.rules.values().filter(|r| r.is_active()).count();
        let disabled_rules = self.rules.values().filter(|r| !r.enabled).count();
        let rules_in_cooldown = self.rules.values().filter(|r| r.is_in_cooldown()).count();
        EngineStats {
            total_rules,
            active_rules,
            disabled_rules,
            total_notifications: self.history.len(),
            rules_in_cooldown,
        }
    }

    // ── Persistence ──────────────────────────────────────────────

    /// Save the engine to a JSON file.
    pub fn save(&self, path: &Path) -> Result<(), NotificationRuleError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load the engine from a JSON file.
    pub fn load(path: &Path) -> Result<Self, NotificationRuleError> {
        let data = std::fs::read_to_string(path)?;
        let engine: Self = serde_json::from_str(&data)?;
        Ok(engine)
    }

    /// Load the engine from a JSON file, falling back to an empty default.
    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ──────────────────────────── Tests ────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "notif_rules_test_{}_{name}",
            std::process::id()
        ))
    }

    #[test]
    fn test_create_rule() {
        let rule = NotificationRule::new("r1", "Balance Alert");
        assert_eq!(rule.id, "r1");
        assert_eq!(rule.name, "Balance Alert");
        assert!(rule.enabled);
        assert_eq!(rule.priority, RulePriority::Normal);
        assert_eq!(rule.cooldown_secs, 300);
        assert_eq!(rule.trigger_count, 0);
        assert!(rule.conditions.is_empty());
        assert!(rule.actions.is_empty());
    }

    #[test]
    fn test_add_duplicate_rejected() {
        let mut engine = RuleEngine::new();
        let r1 = NotificationRule::new("dup", "First");
        let r2 = NotificationRule::new("dup", "Second");
        engine.add_rule(r1).unwrap();
        let err = engine.add_rule(r2).unwrap_err();
        assert!(matches!(err, NotificationRuleError::AlreadyExists(_)));
    }

    #[test]
    fn test_remove_rule() {
        let mut engine = RuleEngine::new();
        engine
            .add_rule(NotificationRule::new("rem", "Removable"))
            .unwrap();
        let removed = engine.remove_rule("rem").unwrap();
        assert_eq!(removed.id, "rem");
        assert!(engine.get_rule("rem").is_none());
        let err = engine.remove_rule("rem").unwrap_err();
        assert!(matches!(err, NotificationRuleError::NotFound(_)));
    }

    #[test]
    fn test_rule_builder_chain() {
        let rule = NotificationRule::new("chain", "Chained")
            .with_condition(RuleCondition::BalanceAbove(1000))
            .with_action(RuleAction::Log("test".into()))
            .with_channel(Channel::Console)
            .with_priority(RulePriority::High)
            .with_cooldown(60)
            .with_max_triggers(5)
            .with_tag("wallet");

        assert_eq!(rule.conditions.len(), 1);
        assert_eq!(rule.actions.len(), 1);
        assert_eq!(rule.channels.len(), 1);
        assert_eq!(rule.priority, RulePriority::High);
        assert_eq!(rule.cooldown_secs, 60);
        assert_eq!(rule.max_triggers, Some(5));
        assert_eq!(rule.tags, vec!["wallet".to_string()]);
    }

    #[test]
    fn test_evaluate_balance_above() {
        let ctx = RuleContext::new().with_balance(5000);
        assert!(ctx.matches_condition(&RuleCondition::BalanceAbove(1000)));
        assert!(!ctx.matches_condition(&RuleCondition::BalanceAbove(9000)));
    }

    #[test]
    fn test_evaluate_balance_below() {
        let ctx = RuleContext::new().with_balance(500);
        assert!(ctx.matches_condition(&RuleCondition::BalanceBelow(1000)));
        assert!(!ctx.matches_condition(&RuleCondition::BalanceBelow(100)));
    }

    #[test]
    fn test_evaluate_tx_amount() {
        let ctx = RuleContext::new().with_tx_amount(2000);
        assert!(ctx.matches_condition(&RuleCondition::TxAmountAbove(1000)));
        assert!(!ctx.matches_condition(&RuleCondition::TxAmountAbove(5000)));
    }

    #[test]
    fn test_evaluate_energy_below() {
        let ctx = RuleContext::new().with_energy(10);
        assert!(ctx.matches_condition(&RuleCondition::EnergyBelow(50)));
        assert!(!ctx.matches_condition(&RuleCondition::EnergyBelow(5)));
    }

    #[test]
    fn test_evaluate_price_above() {
        let ctx = RuleContext::new().with_price(150.0);
        assert!(ctx.matches_condition(&RuleCondition::PriceAbove(100.0)));
        assert!(!ctx.matches_condition(&RuleCondition::PriceAbove(200.0)));
    }

    #[test]
    fn test_evaluate_new_incoming() {
        let ctx = RuleContext::new().with_incoming();
        assert!(ctx.matches_condition(&RuleCondition::NewIncoming));
        let ctx2 = RuleContext::new();
        assert!(!ctx2.matches_condition(&RuleCondition::NewIncoming));
    }

    #[test]
    fn test_evaluate_custom_condition() {
        let ctx = RuleContext::new().with_custom("alert_type", "whale");
        assert!(ctx.matches_condition(&RuleCondition::Custom("alert_type".into())));
        assert!(!ctx.matches_condition(&RuleCondition::Custom("missing_key".into())));
    }

    #[test]
    fn test_evaluate_all_conditions_must_match() {
        let rule = NotificationRule::new("multi", "Multi-cond")
            .with_condition(RuleCondition::BalanceAbove(1000))
            .with_condition(RuleCondition::EnergyBelow(50));

        // Both match
        let ctx1 = RuleContext::new().with_balance(2000).with_energy(20);
        assert!(rule.evaluate(&ctx1));

        // Only one matches
        let ctx2 = RuleContext::new().with_balance(2000).with_energy(80);
        assert!(!rule.evaluate(&ctx2));

        // Neither matches
        let ctx3 = RuleContext::new().with_balance(500).with_energy(80);
        assert!(!rule.evaluate(&ctx3));
    }

    #[test]
    fn test_trigger_increments_count() {
        let mut rule = NotificationRule::new("trig", "Trigger");
        assert_eq!(rule.trigger_count, 0);
        assert!(rule.last_triggered.is_none());
        rule.trigger();
        assert_eq!(rule.trigger_count, 1);
        assert!(rule.last_triggered.is_some());
        rule.trigger();
        assert_eq!(rule.trigger_count, 2);
    }

    #[test]
    fn test_cooldown_prevents_trigger() {
        let mut rule = NotificationRule::new("cd", "Cooldown Test").with_cooldown(600);

        // No last_triggered — not in cooldown
        assert!(!rule.is_in_cooldown());
        assert!(rule.can_trigger());

        // Set last_triggered to now — should be in cooldown
        rule.last_triggered = Some(chrono::Utc::now().to_rfc3339());
        assert!(rule.is_in_cooldown());
        assert!(!rule.can_trigger());

        // Set last_triggered far in the past — cooldown expired
        rule.last_triggered = Some("2020-01-01T00:00:00+00:00".to_string());
        assert!(!rule.is_in_cooldown());
        assert!(rule.can_trigger());
    }

    #[test]
    fn test_max_triggers_disables() {
        let mut rule = NotificationRule::new("max", "Max Test").with_max_triggers(2);
        assert!(rule.is_active());

        rule.trigger();
        assert!(rule.is_active()); // count=1, max=2
        rule.trigger();
        assert!(!rule.is_active()); // count=2, max=2
    }

    #[test]
    fn test_evaluate_all_returns_notifications() {
        let mut engine = RuleEngine::new();

        let rule = NotificationRule::new("bal", "Bal Alert")
            .with_condition(RuleCondition::BalanceAbove(1000))
            .with_action(RuleAction::Log("high balance".into()))
            .with_channel(Channel::Console)
            .with_cooldown(0);

        engine.add_rule(rule).unwrap();

        let ctx = RuleContext::new().with_balance(5000);
        let notifs = engine.evaluate_all(&ctx);
        assert_eq!(notifs.len(), 1);
        assert_eq!(notifs[0].rule_id, "bal");
        assert_eq!(notifs[0].message, "Rule 'Bal Alert' triggered");
        assert_eq!(engine.notification_count(), 1);
    }

    #[test]
    fn test_by_priority() {
        let mut engine = RuleEngine::new();
        engine
            .add_rule(NotificationRule::new("c1", "Crit1").with_priority(RulePriority::Critical))
            .unwrap();
        engine
            .add_rule(NotificationRule::new("n1", "Norm1").with_priority(RulePriority::Normal))
            .unwrap();
        engine
            .add_rule(NotificationRule::new("c2", "Crit2").with_priority(RulePriority::Critical))
            .unwrap();

        let crits = engine.by_priority(&RulePriority::Critical);
        assert_eq!(crits.len(), 2);
        let norms = engine.by_priority(&RulePriority::Normal);
        assert_eq!(norms.len(), 1);
    }

    #[test]
    fn test_by_tag() {
        let mut engine = RuleEngine::new();
        engine
            .add_rule(NotificationRule::new("t1", "Tagged1").with_tag("defi"))
            .unwrap();
        engine
            .add_rule(NotificationRule::new("t2", "Tagged2").with_tag("defi").with_tag("staking"))
            .unwrap();
        engine
            .add_rule(NotificationRule::new("t3", "Tagged3").with_tag("nft"))
            .unwrap();

        assert_eq!(engine.by_tag("defi").len(), 2);
        assert_eq!(engine.by_tag("staking").len(), 1);
        assert_eq!(engine.by_tag("nft").len(), 1);
        assert_eq!(engine.by_tag("missing").len(), 0);
    }

    #[test]
    fn test_recent_notifications() {
        let mut engine = RuleEngine::new();
        engine
            .add_rule(
                NotificationRule::new("rn", "Recent")
                    .with_condition(RuleCondition::NewIncoming)
                    .with_cooldown(0),
            )
            .unwrap();

        let ctx = RuleContext::new().with_incoming();
        engine.evaluate_all(&ctx);
        engine.evaluate_all(&ctx);
        engine.evaluate_all(&ctx);

        let recent = engine.recent_notifications(2);
        assert_eq!(recent.len(), 2);
        // Most recent first
        assert_eq!(recent[0].rule_id, "rn");
    }

    #[test]
    fn test_disable_enable() {
        let mut rule = NotificationRule::new("de", "Toggle");
        assert!(rule.is_active());
        rule.disable();
        assert!(!rule.is_active());
        assert!(!rule.enabled);
        rule.enable();
        assert!(rule.is_active());
        assert!(rule.enabled);
    }

    #[test]
    fn test_reset_rule() {
        let mut rule = NotificationRule::new("rst", "Reset Test");
        rule.trigger();
        rule.trigger();
        assert_eq!(rule.trigger_count, 2);
        assert!(rule.last_triggered.is_some());
        rule.reset();
        assert_eq!(rule.trigger_count, 0);
        assert!(rule.last_triggered.is_none());
    }

    #[test]
    fn test_stats() {
        let mut engine = RuleEngine::new();
        engine
            .add_rule(
                NotificationRule::new("s1", "Active Rule")
                    .with_condition(RuleCondition::NewIncoming)
                    .with_cooldown(0),
            )
            .unwrap();

        let mut disabled_rule = NotificationRule::new("s2", "Disabled Rule");
        disabled_rule.disable();
        engine.add_rule(disabled_rule).unwrap();

        let ctx = RuleContext::new().with_incoming();
        engine.evaluate_all(&ctx);

        let stats = engine.stats();
        assert_eq!(stats.total_rules, 2);
        assert_eq!(stats.active_rules, 1);
        assert_eq!(stats.disabled_rules, 1);
        assert_eq!(stats.total_notifications, 1);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path("roundtrip.json");

        let mut engine = RuleEngine::new();
        engine
            .add_rule(
                NotificationRule::new("p1", "Persist Rule")
                    .with_condition(RuleCondition::BalanceAbove(500))
                    .with_action(RuleAction::Push("alert".into()))
                    .with_channel(Channel::Push)
                    .with_priority(RulePriority::High)
                    .with_tag("persist"),
            )
            .unwrap();

        engine.save(&path).unwrap();
        let loaded = RuleEngine::load(&path).unwrap();
        assert_eq!(loaded.rules.len(), 1);
        let rule = loaded.get_rule("p1").unwrap();
        assert_eq!(rule.name, "Persist Rule");
        assert_eq!(rule.priority, RulePriority::High);
        assert_eq!(rule.conditions.len(), 1);
        assert_eq!(rule.tags, vec!["persist".to_string()]);

        // Clean up
        let _ = std::fs::remove_file(&path);

        // load_or_default on missing file
        let default = RuleEngine::load_or_default(&test_path("nonexistent.json"));
        assert!(default.rules.is_empty());
    }
}
