use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum SmartAlertError {
    #[error("Alert already exists: {0}")]
    AlreadyExists(String),
    #[error("Alert not found: {0}")]
    NotFound(String),
    #[error("Alert is in cooldown")]
    InCooldown,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
    Emergency,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AlertStatus {
    Active,
    Triggered,
    Acknowledged,
    Dismissed,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AlertCondition {
    PriceAbove { token: String, threshold: f64 },
    PriceBelow { token: String, threshold: f64 },
    VolumeSpike { token: String, multiplier: f64 },
    WhaleMovement { min_amount: u64 },
    GasAbove { threshold: u64 },
    GasBelow { threshold: u64 },
    BalanceBelow { token: String, threshold: u64 },
    BalanceAbove { token: String, threshold: u64 },
    ContractEvent { contract: String, event_name: String },
    Custom { name: String, expression: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AlertAction {
    Notify { message: String },
    ExecuteTx { description: String },
    PauseDca { plan_id: String },
    CancelOrder { order_id: String },
    Webhook { url: String },
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartAlert {
    pub id: String,
    pub name: String,
    pub conditions: Vec<AlertCondition>,
    pub actions: Vec<AlertAction>,
    pub severity: AlertSeverity,
    pub status: AlertStatus,
    pub created_at: String,
    pub triggered_at: Option<String>,
    pub trigger_count: u32,
    pub cooldown_secs: u64,
    pub last_triggered: Option<String>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertEvent {
    pub alert_id: String,
    pub timestamp: String,
    pub conditions_met: Vec<String>,
    pub actions_taken: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartAlertStats {
    pub total_alerts: usize,
    pub active: usize,
    pub triggered: usize,
    pub dismissed: usize,
    pub total_events: usize,
    pub total_trigger_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertContext {
    pub prices: HashMap<String, f64>,
    pub volumes: HashMap<String, f64>,
    pub balances: HashMap<String, u64>,
    pub gas_price: u64,
    pub recent_transfers: Vec<(String, u64)>,
    pub contract_events: Vec<(String, String)>,
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SmartAlertEngine {
    pub alerts: HashMap<String, SmartAlert>,
    pub events: Vec<AlertEvent>,
}

impl SmartAlertEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_alert(&mut self, alert: SmartAlert) -> Result<(), SmartAlertError> {
        if self.alerts.contains_key(&alert.id) {
            return Err(SmartAlertError::AlreadyExists(alert.id.clone()));
        }
        self.alerts.insert(alert.id.clone(), alert);
        Ok(())
    }

    pub fn remove_alert(&mut self, id: &str) -> Result<SmartAlert, SmartAlertError> {
        self.alerts
            .remove(id)
            .ok_or_else(|| SmartAlertError::NotFound(id.to_string()))
    }

    pub fn get_alert(&self, id: &str) -> Option<&SmartAlert> {
        self.alerts.get(id)
    }

    pub fn acknowledge(&mut self, id: &str) -> Result<(), SmartAlertError> {
        let alert = self
            .alerts
            .get_mut(id)
            .ok_or_else(|| SmartAlertError::NotFound(id.to_string()))?;
        alert.status = AlertStatus::Acknowledged;
        Ok(())
    }

    pub fn dismiss(&mut self, id: &str) -> Result<(), SmartAlertError> {
        let alert = self
            .alerts
            .get_mut(id)
            .ok_or_else(|| SmartAlertError::NotFound(id.to_string()))?;
        alert.status = AlertStatus::Dismissed;
        Ok(())
    }

    pub fn evaluate_conditions(
        &self,
        alert_id: &str,
        context: &AlertContext,
    ) -> Result<bool, SmartAlertError> {
        let alert = self
            .alerts
            .get(alert_id)
            .ok_or_else(|| SmartAlertError::NotFound(alert_id.to_string()))?;

        for condition in &alert.conditions {
            if !Self::evaluate_single(condition, context) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn evaluate_single(condition: &AlertCondition, ctx: &AlertContext) -> bool {
        match condition {
            AlertCondition::PriceAbove { token, threshold } => {
                ctx.prices.get(token).map_or(false, |p| p > threshold)
            }
            AlertCondition::PriceBelow { token, threshold } => {
                ctx.prices.get(token).map_or(false, |p| p < threshold)
            }
            AlertCondition::VolumeSpike { token, multiplier } => {
                ctx.volumes.get(token).map_or(false, |v| v >= multiplier)
            }
            AlertCondition::WhaleMovement { min_amount } => ctx
                .recent_transfers
                .iter()
                .any(|(_, amt)| amt >= min_amount),
            AlertCondition::GasAbove { threshold } => ctx.gas_price > *threshold,
            AlertCondition::GasBelow { threshold } => ctx.gas_price < *threshold,
            AlertCondition::BalanceBelow { token, threshold } => {
                ctx.balances.get(token).map_or(false, |b| b < threshold)
            }
            AlertCondition::BalanceAbove { token, threshold } => {
                ctx.balances.get(token).map_or(false, |b| b > threshold)
            }
            AlertCondition::ContractEvent {
                contract,
                event_name,
            } => ctx
                .contract_events
                .iter()
                .any(|(c, e)| c == contract && e == event_name),
            AlertCondition::Custom { .. } => {
                // Custom expressions are not evaluated at runtime; always false.
                false
            }
        }
    }

    pub fn trigger_alert(&mut self, id: &str) -> Result<Vec<String>, SmartAlertError> {
        let now = chrono::Utc::now().to_rfc3339();

        let alert = self
            .alerts
            .get(id)
            .ok_or_else(|| SmartAlertError::NotFound(id.to_string()))?;

        // Cooldown check
        if let Some(ref last) = alert.last_triggered {
            if let Ok(last_time) = chrono::DateTime::parse_from_rfc3339(last) {
                let elapsed = chrono::Utc::now()
                    .signed_duration_since(last_time)
                    .num_seconds();
                if elapsed >= 0 && (elapsed as u64) < alert.cooldown_secs {
                    return Err(SmartAlertError::InCooldown);
                }
            }
        }

        let action_descriptions: Vec<String> = alert
            .actions
            .iter()
            .map(|a| match a {
                AlertAction::Notify { message } => format!("Notify: {}", message),
                AlertAction::ExecuteTx { description } => format!("ExecuteTx: {}", description),
                AlertAction::PauseDca { plan_id } => format!("PauseDca: {}", plan_id),
                AlertAction::CancelOrder { order_id } => format!("CancelOrder: {}", order_id),
                AlertAction::Webhook { url } => format!("Webhook: {}", url),
            })
            .collect();

        let conditions_met: Vec<String> = alert
            .conditions
            .iter()
            .map(|c| format!("{:?}", c))
            .collect();

        let event = AlertEvent {
            alert_id: id.to_string(),
            timestamp: now.clone(),
            conditions_met,
            actions_taken: action_descriptions.clone(),
        };

        // Now mutate
        let alert = self.alerts.get_mut(id).unwrap();
        alert.status = AlertStatus::Triggered;
        alert.triggered_at = Some(now.clone());
        alert.last_triggered = Some(now);
        alert.trigger_count += 1;

        self.events.push(event);

        Ok(action_descriptions)
    }

    pub fn check_all(&mut self, context: &AlertContext) -> Vec<String> {
        let active_ids: Vec<String> = self
            .alerts
            .iter()
            .filter(|(_, a)| a.status == AlertStatus::Active)
            .map(|(id, _)| id.clone())
            .collect();

        let mut triggered = Vec::new();
        for id in active_ids {
            let matched = self.evaluate_conditions(&id, context).unwrap_or(false);
            if matched {
                if self.trigger_alert(&id).is_ok() {
                    triggered.push(id);
                }
            }
        }
        triggered
    }

    pub fn active_alerts(&self) -> Vec<&SmartAlert> {
        self.alerts
            .values()
            .filter(|a| a.status == AlertStatus::Active)
            .collect()
    }

    pub fn alerts_by_severity(&self, severity: &AlertSeverity) -> Vec<&SmartAlert> {
        self.alerts
            .values()
            .filter(|a| &a.severity == severity)
            .collect()
    }

    pub fn recent_events(&self, n: usize) -> Vec<&AlertEvent> {
        self.events.iter().rev().take(n).collect()
    }

    pub fn expire_alerts(&mut self) -> Vec<String> {
        let now = chrono::Utc::now().to_rfc3339();
        let mut expired = Vec::new();

        for (id, alert) in self.alerts.iter_mut() {
            if alert.status == AlertStatus::Active || alert.status == AlertStatus::Triggered {
                if let Some(ref exp) = alert.expires_at {
                    if *exp <= now {
                        alert.status = AlertStatus::Expired;
                        expired.push(id.clone());
                    }
                }
            }
        }
        expired
    }

    pub fn stats(&self) -> SmartAlertStats {
        let total_alerts = self.alerts.len();
        let active = self
            .alerts
            .values()
            .filter(|a| a.status == AlertStatus::Active)
            .count();
        let triggered = self
            .alerts
            .values()
            .filter(|a| a.status == AlertStatus::Triggered)
            .count();
        let dismissed = self
            .alerts
            .values()
            .filter(|a| a.status == AlertStatus::Dismissed)
            .count();
        let total_events = self.events.len();
        let total_trigger_count: u32 = self.alerts.values().map(|a| a.trigger_count).sum();

        SmartAlertStats {
            total_alerts,
            active,
            triggered,
            dismissed,
            total_events,
            total_trigger_count,
        }
    }

    pub fn load(path: &Path) -> Result<Self, SmartAlertError> {
        let data = std::fs::read_to_string(path)?;
        let engine: Self = serde_json::from_str(&data)?;
        Ok(engine)
    }

    pub fn save(&self, path: &Path) -> Result<(), SmartAlertError> {
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Helpers for tests
// ---------------------------------------------------------------------------

fn _make_alert(id: &str, name: &str) -> SmartAlert {
    SmartAlert {
        id: id.to_string(),
        name: name.to_string(),
        conditions: Vec::new(),
        actions: Vec::new(),
        severity: AlertSeverity::Info,
        status: AlertStatus::Active,
        created_at: chrono::Utc::now().to_rfc3339(),
        triggered_at: None,
        trigger_count: 0,
        cooldown_secs: 0,
        last_triggered: None,
        expires_at: None,
    }
}

fn _make_context() -> AlertContext {
    AlertContext {
        prices: HashMap::new(),
        volumes: HashMap::new(),
        balances: HashMap::new(),
        gas_price: 0,
        recent_transfers: Vec::new(),
        contract_events: Vec::new(),
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

    fn tmp_path(suffix: &str) -> std::path::PathBuf {
        temp_dir().join(format!("smart_alerts_test_{}_{}", id(), suffix))
    }

    #[test]
    fn test_create_alert() {
        let mut engine = SmartAlertEngine::new();
        let alert = make_alert("a1", "Test Alert");
        assert!(engine.create_alert(alert).is_ok());
        assert!(engine.get_alert("a1").is_some());
    }

    #[test]
    fn test_create_duplicate_alert() {
        let mut engine = SmartAlertEngine::new();
        engine.create_alert(make_alert("a1", "First")).unwrap();
        let result = engine.create_alert(make_alert("a1", "Duplicate"));
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_alert() {
        let mut engine = SmartAlertEngine::new();
        engine.create_alert(make_alert("a1", "Test")).unwrap();
        let removed = engine.remove_alert("a1").unwrap();
        assert_eq!(removed.id, "a1");
        assert!(engine.get_alert("a1").is_none());
    }

    #[test]
    fn test_remove_missing_alert() {
        let mut engine = SmartAlertEngine::new();
        assert!(engine.remove_alert("nope").is_err());
    }

    #[test]
    fn test_acknowledge() {
        let mut engine = SmartAlertEngine::new();
        engine.create_alert(make_alert("a1", "Test")).unwrap();
        engine.acknowledge("a1").unwrap();
        assert_eq!(engine.get_alert("a1").unwrap().status, AlertStatus::Acknowledged);
    }

    #[test]
    fn test_dismiss() {
        let mut engine = SmartAlertEngine::new();
        engine.create_alert(make_alert("a1", "Test")).unwrap();
        engine.dismiss("a1").unwrap();
        assert_eq!(engine.get_alert("a1").unwrap().status, AlertStatus::Dismissed);
    }

    #[test]
    fn test_acknowledge_missing() {
        let mut engine = SmartAlertEngine::new();
        assert!(engine.acknowledge("nope").is_err());
    }

    #[test]
    fn test_evaluate_price_above() {
        let mut engine = SmartAlertEngine::new();
        let mut alert = make_alert("a1", "Price");
        alert.conditions.push(AlertCondition::PriceAbove {
            token: "ETH".to_string(),
            threshold: 2000.0,
        });
        engine.create_alert(alert).unwrap();

        let mut ctx = make_context();
        ctx.prices.insert("ETH".to_string(), 2500.0);
        assert!(engine.evaluate_conditions("a1", &ctx).unwrap());

        ctx.prices.insert("ETH".to_string(), 1500.0);
        assert!(!engine.evaluate_conditions("a1", &ctx).unwrap());
    }

    #[test]
    fn test_evaluate_price_below() {
        let mut engine = SmartAlertEngine::new();
        let mut alert = make_alert("a1", "Price Low");
        alert.conditions.push(AlertCondition::PriceBelow {
            token: "BTC".to_string(),
            threshold: 30000.0,
        });
        engine.create_alert(alert).unwrap();

        let mut ctx = make_context();
        ctx.prices.insert("BTC".to_string(), 25000.0);
        assert!(engine.evaluate_conditions("a1", &ctx).unwrap());
    }

    #[test]
    fn test_evaluate_gas_above() {
        let mut engine = SmartAlertEngine::new();
        let mut alert = make_alert("a1", "Gas");
        alert.conditions.push(AlertCondition::GasAbove { threshold: 100 });
        engine.create_alert(alert).unwrap();

        let mut ctx = make_context();
        ctx.gas_price = 150;
        assert!(engine.evaluate_conditions("a1", &ctx).unwrap());

        ctx.gas_price = 50;
        assert!(!engine.evaluate_conditions("a1", &ctx).unwrap());
    }

    #[test]
    fn test_evaluate_gas_below() {
        let mut engine = SmartAlertEngine::new();
        let mut alert = make_alert("a1", "Gas Low");
        alert.conditions.push(AlertCondition::GasBelow { threshold: 100 });
        engine.create_alert(alert).unwrap();

        let mut ctx = make_context();
        ctx.gas_price = 50;
        assert!(engine.evaluate_conditions("a1", &ctx).unwrap());
    }

    #[test]
    fn test_evaluate_whale_movement() {
        let mut engine = SmartAlertEngine::new();
        let mut alert = make_alert("a1", "Whale");
        alert
            .conditions
            .push(AlertCondition::WhaleMovement { min_amount: 1_000_000 });
        engine.create_alert(alert).unwrap();

        let mut ctx = make_context();
        ctx.recent_transfers
            .push(("0xabc".to_string(), 2_000_000));
        assert!(engine.evaluate_conditions("a1", &ctx).unwrap());
    }

    #[test]
    fn test_evaluate_balance_below() {
        let mut engine = SmartAlertEngine::new();
        let mut alert = make_alert("a1", "Low Balance");
        alert.conditions.push(AlertCondition::BalanceBelow {
            token: "USDC".to_string(),
            threshold: 1000,
        });
        engine.create_alert(alert).unwrap();

        let mut ctx = make_context();
        ctx.balances.insert("USDC".to_string(), 500);
        assert!(engine.evaluate_conditions("a1", &ctx).unwrap());
    }

    #[test]
    fn test_evaluate_contract_event() {
        let mut engine = SmartAlertEngine::new();
        let mut alert = make_alert("a1", "Event");
        alert.conditions.push(AlertCondition::ContractEvent {
            contract: "0xdead".to_string(),
            event_name: "Transfer".to_string(),
        });
        engine.create_alert(alert).unwrap();

        let mut ctx = make_context();
        ctx.contract_events
            .push(("0xdead".to_string(), "Transfer".to_string()));
        assert!(engine.evaluate_conditions("a1", &ctx).unwrap());
    }

    #[test]
    fn test_evaluate_and_logic() {
        let mut engine = SmartAlertEngine::new();
        let mut alert = make_alert("a1", "Multi");
        alert.conditions.push(AlertCondition::GasAbove { threshold: 50 });
        alert.conditions.push(AlertCondition::PriceAbove {
            token: "ETH".to_string(),
            threshold: 1000.0,
        });
        engine.create_alert(alert).unwrap();

        let mut ctx = make_context();
        ctx.gas_price = 100;
        ctx.prices.insert("ETH".to_string(), 500.0);
        // Gas matches but price does not
        assert!(!engine.evaluate_conditions("a1", &ctx).unwrap());

        ctx.prices.insert("ETH".to_string(), 2000.0);
        assert!(engine.evaluate_conditions("a1", &ctx).unwrap());
    }

    #[test]
    fn test_trigger_alert() {
        let mut engine = SmartAlertEngine::new();
        let mut alert = make_alert("a1", "Trigger");
        alert.actions.push(AlertAction::Notify {
            message: "hello".to_string(),
        });
        engine.create_alert(alert).unwrap();

        let actions = engine.trigger_alert("a1").unwrap();
        assert_eq!(actions.len(), 1);
        assert!(actions[0].contains("Notify"));

        let a = engine.get_alert("a1").unwrap();
        assert_eq!(a.status, AlertStatus::Triggered);
        assert_eq!(a.trigger_count, 1);
        assert!(a.triggered_at.is_some());
        assert_eq!(engine.events.len(), 1);
    }

    #[test]
    fn test_trigger_cooldown() {
        let mut engine = SmartAlertEngine::new();
        let mut alert = make_alert("a1", "Cooldown");
        alert.cooldown_secs = 3600;
        alert.actions.push(AlertAction::Notify {
            message: "test".to_string(),
        });
        engine.create_alert(alert).unwrap();

        engine.trigger_alert("a1").unwrap();
        let result = engine.trigger_alert("a1");
        assert!(result.is_err());
    }

    #[test]
    fn test_check_all() {
        let mut engine = SmartAlertEngine::new();
        let mut a1 = make_alert("a1", "Match");
        a1.conditions
            .push(AlertCondition::GasAbove { threshold: 10 });
        a1.actions.push(AlertAction::Notify {
            message: "gas".to_string(),
        });
        engine.create_alert(a1).unwrap();

        let mut a2 = make_alert("a2", "No Match");
        a2.conditions
            .push(AlertCondition::GasAbove { threshold: 1000 });
        engine.create_alert(a2).unwrap();

        let mut ctx = make_context();
        ctx.gas_price = 50;
        let triggered = engine.check_all(&ctx);
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered[0], "a1");
    }

    #[test]
    fn test_active_alerts() {
        let mut engine = SmartAlertEngine::new();
        engine.create_alert(make_alert("a1", "Active")).unwrap();
        let mut dismissed = make_alert("a2", "Dismissed");
        dismissed.status = AlertStatus::Dismissed;
        engine.create_alert(dismissed).unwrap();

        assert_eq!(engine.active_alerts().len(), 1);
    }

    #[test]
    fn test_alerts_by_severity() {
        let mut engine = SmartAlertEngine::new();
        let mut crit = make_alert("a1", "Crit");
        crit.severity = AlertSeverity::Critical;
        engine.create_alert(crit).unwrap();
        engine.create_alert(make_alert("a2", "Info")).unwrap();

        assert_eq!(engine.alerts_by_severity(&AlertSeverity::Critical).len(), 1);
        assert_eq!(engine.alerts_by_severity(&AlertSeverity::Info).len(), 1);
        assert_eq!(engine.alerts_by_severity(&AlertSeverity::Emergency).len(), 0);
    }

    #[test]
    fn test_recent_events() {
        let mut engine = SmartAlertEngine::new();
        let mut alert = make_alert("a1", "Test");
        alert.actions.push(AlertAction::Notify {
            message: "x".to_string(),
        });
        engine.create_alert(alert).unwrap();
        engine.trigger_alert("a1").unwrap();

        let events = engine.recent_events(5);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].alert_id, "a1");
    }

    #[test]
    fn test_expire_alerts() {
        let mut engine = SmartAlertEngine::new();
        let mut alert = make_alert("a1", "Expiring");
        alert.expires_at = Some("2000-01-01T00:00:00+00:00".to_string());
        engine.create_alert(alert).unwrap();

        let expired = engine.expire_alerts();
        assert_eq!(expired.len(), 1);
        assert_eq!(
            engine.get_alert("a1").unwrap().status,
            AlertStatus::Expired
        );
    }

    #[test]
    fn test_stats() {
        let mut engine = SmartAlertEngine::new();
        engine.create_alert(make_alert("a1", "One")).unwrap();
        let mut d = make_alert("a2", "Two");
        d.status = AlertStatus::Dismissed;
        engine.create_alert(d).unwrap();

        let s = engine.stats();
        assert_eq!(s.total_alerts, 2);
        assert_eq!(s.active, 1);
        assert_eq!(s.dismissed, 1);
    }

    #[test]
    fn test_save_and_load() {
        let path = tmp_path("save_load.json");
        let mut engine = SmartAlertEngine::new();
        engine.create_alert(make_alert("a1", "Persist")).unwrap();
        engine.save(&path).unwrap();

        let loaded = SmartAlertEngine::load(&path).unwrap();
        assert!(loaded.get_alert("a1").is_some());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = tmp_path("nonexistent.json");
        let engine = SmartAlertEngine::load_or_default(&path);
        assert!(engine.alerts.is_empty());
    }
}
