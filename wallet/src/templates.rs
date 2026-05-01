//! Transaction Templates — save, replay, and schedule recurring transactions.
//!
//! Save common transactions as templates ("pay rent", "refresh my NFTs") and
//! replay them with a single command. Supports recurring schedules with
//! frequency-based execution tracking.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

// ──────────────────────────── Types ──────────────────────────────────────

/// Error type for template operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum TemplateError {
    #[error("template not found: {0}")]
    NotFound(String),
    #[error("template already exists: {0}")]
    AlreadyExists(String),
    #[error("invalid frequency: {0}")]
    InvalidFrequency(String),
    #[error("template is disabled: {0}")]
    Disabled(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
}

impl From<std::io::Error> for TemplateError {
    fn from(e: std::io::Error) -> Self {
        TemplateError::Io(e.to_string())
    }
}
impl From<serde_json::Error> for TemplateError {
    fn from(e: serde_json::Error) -> Self {
        TemplateError::Json(e.to_string())
    }
}

/// The type of transaction a template describes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateType {
    Transfer,
    Refresh,
    Stake,
    Unstake,
    NftMint,
    NftTransfer,
    TokenTransfer,
    ContractCall,
}

impl TemplateType {
    pub fn label(&self) -> &'static str {
        match self {
            TemplateType::Transfer => "transfer",
            TemplateType::Refresh => "refresh",
            TemplateType::Stake => "stake",
            TemplateType::Unstake => "unstake",
            TemplateType::NftMint => "nft_mint",
            TemplateType::NftTransfer => "nft_transfer",
            TemplateType::TokenTransfer => "token_transfer",
            TemplateType::ContractCall => "contract_call",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "transfer" | "send" => Some(TemplateType::Transfer),
            "refresh" => Some(TemplateType::Refresh),
            "stake" => Some(TemplateType::Stake),
            "unstake" => Some(TemplateType::Unstake),
            "nft_mint" | "mint" => Some(TemplateType::NftMint),
            "nft_transfer" => Some(TemplateType::NftTransfer),
            "token_transfer" => Some(TemplateType::TokenTransfer),
            "contract_call" | "call" => Some(TemplateType::ContractCall),
            _ => None,
        }
    }
}

/// Recurring frequency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Frequency {
    /// Execute once (not recurring).
    Once,
    /// Every N hours.
    Hourly(u64),
    /// Every N days.
    Daily(u64),
    /// Every N weeks.
    Weekly(u64),
    /// Every N months.
    Monthly(u64),
}

impl Frequency {
    pub fn label(&self) -> String {
        match self {
            Frequency::Once => "once".to_string(),
            Frequency::Hourly(n) => format!("every {}h", n),
            Frequency::Daily(n) => {
                if *n == 1 {
                    "daily".to_string()
                } else {
                    format!("every {}d", n)
                }
            }
            Frequency::Weekly(n) => {
                if *n == 1 {
                    "weekly".to_string()
                } else {
                    format!("every {}w", n)
                }
            }
            Frequency::Monthly(n) => {
                if *n == 1 {
                    "monthly".to_string()
                } else {
                    format!("every {}mo", n)
                }
            }
        }
    }

    /// Parse frequency from string: "once", "daily", "weekly", "monthly",
    /// "hourly:6", "daily:3", "weekly:2", "monthly:3"
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Result<Self, TemplateError> {
        let parts: Vec<&str> = s.split(':').collect();
        match parts[0].to_lowercase().as_str() {
            "once" => Ok(Frequency::Once),
            "hourly" => {
                let n = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(1);
                if n == 0 {
                    return Err(TemplateError::InvalidFrequency(s.to_string()));
                }
                Ok(Frequency::Hourly(n))
            }
            "daily" => {
                let n = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(1);
                if n == 0 {
                    return Err(TemplateError::InvalidFrequency(s.to_string()));
                }
                Ok(Frequency::Daily(n))
            }
            "weekly" => {
                let n = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(1);
                if n == 0 {
                    return Err(TemplateError::InvalidFrequency(s.to_string()));
                }
                Ok(Frequency::Weekly(n))
            }
            "monthly" => {
                let n = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(1);
                if n == 0 {
                    return Err(TemplateError::InvalidFrequency(s.to_string()));
                }
                Ok(Frequency::Monthly(n))
            }
            _ => Err(TemplateError::InvalidFrequency(s.to_string())),
        }
    }

    /// Interval in seconds (approximate for months = 30 days).
    pub fn interval_secs(&self) -> Option<u64> {
        match self {
            Frequency::Once => None,
            Frequency::Hourly(n) => Some(n * 3600),
            Frequency::Daily(n) => Some(n * 86400),
            Frequency::Weekly(n) => Some(n * 604800),
            Frequency::Monthly(n) => Some(n * 2592000),
        }
    }
}

/// A saved transaction template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Template {
    /// Unique template name (user-chosen).
    pub name: String,
    /// Description.
    pub description: String,
    /// Transaction type.
    pub tx_type: TemplateType,
    /// Parameters as key-value pairs.
    pub params: HashMap<String, String>,
    /// Recurrence frequency.
    pub frequency: Frequency,
    /// Whether template is active.
    pub enabled: bool,
    /// Number of times executed.
    pub exec_count: u64,
    /// Last execution timestamp (RFC3339).
    pub last_executed: Option<String>,
    /// Next scheduled execution (RFC3339), if recurring.
    pub next_execution: Option<String>,
    /// Creation timestamp.
    pub created_at: String,
    /// Optional tags for organization.
    pub tags: Vec<String>,
}

impl Template {
    /// Check if this template is due for execution based on current time.
    pub fn is_due(&self) -> bool {
        if !self.enabled {
            return false;
        }
        match &self.next_execution {
            Some(next) => {
                let now = chrono::Utc::now().to_rfc3339();
                now >= *next
            }
            None => {
                // No next execution scheduled — due if never executed and recurring
                self.exec_count == 0 && self.frequency != Frequency::Once
            }
        }
    }

    /// Record an execution and compute next execution time.
    pub fn record_execution(&mut self) {
        let now = chrono::Utc::now();
        self.last_executed = Some(now.to_rfc3339());
        self.exec_count += 1;

        // Compute next execution
        self.next_execution = match self.frequency.interval_secs() {
            Some(secs) => {
                let next = now + chrono::Duration::seconds(secs as i64);
                Some(next.to_rfc3339())
            }
            None => None, // Once — no next
        };
    }

    /// Get a parameter value.
    pub fn param(&self, key: &str) -> Option<&str> {
        self.params.get(key).map(|s| s.as_str())
    }
}

// ──────────────────────────── Store ──────────────────────────────────────

/// Persistent store for transaction templates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateStore {
    pub templates: Vec<Template>,
}

impl TemplateStore {
    pub fn new() -> Self {
        Self {
            templates: Vec::new(),
        }
    }

    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, TemplateError> {
        let data = std::fs::read_to_string(path)?;
        let store: TemplateStore = serde_json::from_str(&data)?;
        Ok(store)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), TemplateError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Create a new template.
    pub fn create(
        &mut self,
        name: &str,
        description: &str,
        tx_type: TemplateType,
        params: HashMap<String, String>,
        frequency: Frequency,
        tags: Vec<String>,
    ) -> Result<&Template, TemplateError> {
        if self.templates.iter().any(|t| t.name == name) {
            return Err(TemplateError::AlreadyExists(name.to_string()));
        }

        let now = chrono::Utc::now();

        // Compute initial next execution for recurring
        let next_execution = match frequency.interval_secs() {
            Some(secs) => {
                let next = now + chrono::Duration::seconds(secs as i64);
                Some(next.to_rfc3339())
            }
            None => None,
        };

        let template = Template {
            name: name.to_string(),
            description: description.to_string(),
            tx_type,
            params,
            frequency,
            enabled: true,
            exec_count: 0,
            last_executed: None,
            next_execution,
            created_at: now.to_rfc3339(),
            tags,
        };

        self.templates.push(template);
        Ok(self.templates.last().unwrap())
    }

    /// Create a transfer template (convenience).
    pub fn create_transfer(
        &mut self,
        name: &str,
        to: &str,
        amount: u64,
        frequency: Frequency,
    ) -> Result<&Template, TemplateError> {
        let mut params = HashMap::new();
        params.insert("to".to_string(), to.to_string());
        params.insert("amount".to_string(), amount.to_string());
        self.create(
            name,
            &format!("Send {} EVAP to {}", amount, to),
            TemplateType::Transfer,
            params,
            frequency,
            vec![],
        )
    }

    /// Create a refresh template (convenience).
    pub fn create_refresh(
        &mut self,
        name: &str,
        object_id: &str,
        energy: u64,
        frequency: Frequency,
    ) -> Result<&Template, TemplateError> {
        let mut params = HashMap::new();
        params.insert("object_id".to_string(), object_id.to_string());
        params.insert("energy".to_string(), energy.to_string());
        self.create(
            name,
            &format!("Refresh {} with {} energy", object_id, energy),
            TemplateType::Refresh,
            params,
            frequency,
            vec![],
        )
    }

    /// Get a template by name.
    pub fn get(&self, name: &str) -> Option<&Template> {
        self.templates.iter().find(|t| t.name == name)
    }

    /// Get a mutable template by name.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Template> {
        self.templates.iter_mut().find(|t| t.name == name)
    }

    /// Remove a template by name.
    pub fn remove(&mut self, name: &str) -> Result<(), TemplateError> {
        let idx = self
            .templates
            .iter()
            .position(|t| t.name == name)
            .ok_or_else(|| TemplateError::NotFound(name.to_string()))?;
        self.templates.remove(idx);
        Ok(())
    }

    /// Enable a template.
    pub fn enable(&mut self, name: &str) -> Result<(), TemplateError> {
        let t = self
            .get_mut(name)
            .ok_or_else(|| TemplateError::NotFound(name.to_string()))?;
        t.enabled = true;
        Ok(())
    }

    /// Disable a template.
    pub fn disable(&mut self, name: &str) -> Result<(), TemplateError> {
        let t = self
            .get_mut(name)
            .ok_or_else(|| TemplateError::NotFound(name.to_string()))?;
        t.enabled = false;
        Ok(())
    }

    /// List all templates.
    pub fn list(&self) -> &[Template] {
        &self.templates
    }

    /// List enabled recurring templates.
    pub fn recurring(&self) -> Vec<&Template> {
        self.templates
            .iter()
            .filter(|t| t.enabled && t.frequency != Frequency::Once)
            .collect()
    }

    /// Get all templates that are due for execution.
    pub fn due(&self) -> Vec<&Template> {
        self.templates.iter().filter(|t| t.is_due()).collect()
    }

    /// Record execution of a template by name.
    pub fn record_execution(&mut self, name: &str) -> Result<(), TemplateError> {
        let t = self
            .get_mut(name)
            .ok_or_else(|| TemplateError::NotFound(name.to_string()))?;
        if !t.enabled {
            return Err(TemplateError::Disabled(name.to_string()));
        }
        t.record_execution();
        Ok(())
    }

    /// Search templates by tag.
    pub fn by_tag(&self, tag: &str) -> Vec<&Template> {
        let tag_lower = tag.to_lowercase();
        self.templates
            .iter()
            .filter(|t| {
                t.tags
                    .iter()
                    .any(|tg| tg.to_lowercase().contains(&tag_lower))
            })
            .collect()
    }

    /// Search templates by name substring.
    pub fn search(&self, query: &str) -> Vec<&Template> {
        let q = query.to_lowercase();
        self.templates
            .iter()
            .filter(|t| {
                t.name.to_lowercase().contains(&q) || t.description.to_lowercase().contains(&q)
            })
            .collect()
    }

    /// Total number of templates.
    pub fn count(&self) -> usize {
        self.templates.len()
    }

    /// Total executions across all templates.
    pub fn total_executions(&self) -> u64 {
        self.templates.iter().map(|t| t.exec_count).sum()
    }
}

impl Default for TemplateStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Default path for templates store.
pub fn default_templates_path() -> std::path::PathBuf {
    crate::config::default_data_dir().join("templates.json")
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store() -> TemplateStore {
        let mut store = TemplateStore::new();
        store
            .create_transfer("rent", "0xlandlord", 5000, Frequency::Monthly(1))
            .unwrap();
        store
            .create_refresh("keep-nft-alive", "obj_42", 1000, Frequency::Weekly(1))
            .unwrap();
        store
    }

    #[test]
    fn test_create_template() {
        let store = make_store();
        assert_eq!(store.count(), 2);
        let t = store.get("rent").unwrap();
        assert_eq!(t.tx_type, TemplateType::Transfer);
        assert_eq!(t.param("to").unwrap(), "0xlandlord");
        assert_eq!(t.param("amount").unwrap(), "5000");
        assert!(t.enabled);
        assert_eq!(t.exec_count, 0);
    }

    #[test]
    fn test_create_refresh_template() {
        let store = make_store();
        let t = store.get("keep-nft-alive").unwrap();
        assert_eq!(t.tx_type, TemplateType::Refresh);
        assert_eq!(t.param("object_id").unwrap(), "obj_42");
        assert_eq!(t.param("energy").unwrap(), "1000");
    }

    #[test]
    fn test_duplicate_name_rejected() {
        let mut store = make_store();
        let err = store.create_transfer("rent", "0xother", 100, Frequency::Once);
        assert!(err.is_err());
    }

    #[test]
    fn test_remove_template() {
        let mut store = make_store();
        store.remove("rent").unwrap();
        assert_eq!(store.count(), 1);
        assert!(store.get("rent").is_none());
    }

    #[test]
    fn test_remove_nonexistent() {
        let mut store = make_store();
        assert!(store.remove("nonexistent").is_err());
    }

    #[test]
    fn test_enable_disable() {
        let mut store = make_store();
        store.disable("rent").unwrap();
        assert!(!store.get("rent").unwrap().enabled);
        store.enable("rent").unwrap();
        assert!(store.get("rent").unwrap().enabled);
    }

    #[test]
    fn test_record_execution() {
        let mut store = make_store();
        store.record_execution("rent").unwrap();
        let t = store.get("rent").unwrap();
        assert_eq!(t.exec_count, 1);
        assert!(t.last_executed.is_some());
        assert!(t.next_execution.is_some());
    }

    #[test]
    fn test_record_execution_disabled() {
        let mut store = make_store();
        store.disable("rent").unwrap();
        let err = store.record_execution("rent");
        assert!(err.is_err());
    }

    #[test]
    fn test_record_execution_increments() {
        let mut store = make_store();
        store.record_execution("rent").unwrap();
        store.record_execution("rent").unwrap();
        store.record_execution("rent").unwrap();
        assert_eq!(store.get("rent").unwrap().exec_count, 3);
    }

    #[test]
    fn test_recurring_filter() {
        let mut store = make_store();
        store
            .create_transfer("one-time", "0xfoo", 100, Frequency::Once)
            .unwrap();
        let recurring = store.recurring();
        assert_eq!(recurring.len(), 2); // rent + keep-nft-alive
    }

    #[test]
    fn test_total_executions() {
        let mut store = make_store();
        store.record_execution("rent").unwrap();
        store.record_execution("rent").unwrap();
        store.record_execution("keep-nft-alive").unwrap();
        assert_eq!(store.total_executions(), 3);
    }

    #[test]
    fn test_search() {
        let store = make_store();
        let results = store.search("rent");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "rent");
    }

    #[test]
    fn test_search_description() {
        let store = make_store();
        let results = store.search("landlord");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_by_tag() {
        let mut store = TemplateStore::new();
        let mut params = HashMap::new();
        params.insert("to".into(), "0x1".into());
        params.insert("amount".into(), "100".into());
        store
            .create(
                "tagged",
                "test",
                TemplateType::Transfer,
                params,
                Frequency::Once,
                vec!["bills".into(), "monthly".into()],
            )
            .unwrap();
        let results = store.by_tag("bills");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_frequency_parse() {
        assert_eq!(Frequency::from_str("once").unwrap(), Frequency::Once);
        assert_eq!(Frequency::from_str("daily").unwrap(), Frequency::Daily(1));
        assert_eq!(Frequency::from_str("daily:3").unwrap(), Frequency::Daily(3));
        assert_eq!(Frequency::from_str("weekly").unwrap(), Frequency::Weekly(1));
        assert_eq!(
            Frequency::from_str("monthly:2").unwrap(),
            Frequency::Monthly(2)
        );
        assert_eq!(
            Frequency::from_str("hourly:6").unwrap(),
            Frequency::Hourly(6)
        );
    }

    #[test]
    fn test_frequency_parse_invalid() {
        assert!(Frequency::from_str("never").is_err());
        assert!(Frequency::from_str("daily:0").is_err());
    }

    #[test]
    fn test_frequency_label() {
        assert_eq!(Frequency::Once.label(), "once");
        assert_eq!(Frequency::Daily(1).label(), "daily");
        assert_eq!(Frequency::Daily(3).label(), "every 3d");
        assert_eq!(Frequency::Weekly(1).label(), "weekly");
        assert_eq!(Frequency::Monthly(1).label(), "monthly");
        assert_eq!(Frequency::Hourly(6).label(), "every 6h");
    }

    #[test]
    fn test_frequency_interval() {
        assert_eq!(Frequency::Once.interval_secs(), None);
        assert_eq!(Frequency::Daily(1).interval_secs(), Some(86400));
        assert_eq!(Frequency::Hourly(2).interval_secs(), Some(7200));
        assert_eq!(Frequency::Weekly(1).interval_secs(), Some(604800));
    }

    #[test]
    fn test_template_type_from_str() {
        assert_eq!(
            TemplateType::from_str("transfer"),
            Some(TemplateType::Transfer)
        );
        assert_eq!(TemplateType::from_str("send"), Some(TemplateType::Transfer));
        assert_eq!(
            TemplateType::from_str("refresh"),
            Some(TemplateType::Refresh)
        );
        assert_eq!(TemplateType::from_str("stake"), Some(TemplateType::Stake));
        assert_eq!(TemplateType::from_str("unknown"), None);
    }

    #[test]
    fn test_is_due_disabled() {
        let mut store = make_store();
        store.disable("rent").unwrap();
        assert!(!store.get("rent").unwrap().is_due());
    }

    #[test]
    fn test_once_template_not_recurring() {
        let mut store = TemplateStore::new();
        store
            .create_transfer("one-off", "0xbar", 50, Frequency::Once)
            .unwrap();
        assert!(store.recurring().is_empty());
    }

    #[test]
    fn test_create_with_custom_params() {
        let mut store = TemplateStore::new();
        let mut params = HashMap::new();
        params.insert("contract".into(), "0xdefi".into());
        params.insert("method".into(), "swap".into());
        params.insert("args".into(), "100,EVAP,USDC".into());
        store
            .create(
                "swap-evap",
                "Swap EVAP for USDC",
                TemplateType::ContractCall,
                params,
                Frequency::Once,
                vec!["defi".into()],
            )
            .unwrap();
        let t = store.get("swap-evap").unwrap();
        assert_eq!(t.tx_type, TemplateType::ContractCall);
        assert_eq!(t.param("method").unwrap(), "swap");
    }

    #[test]
    fn test_persistence_roundtrip() {
        let dir = std::env::temp_dir().join("evap_tmpl_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("templates.json");

        let mut store = make_store();
        store.record_execution("rent").unwrap();
        store.save(&path).unwrap();

        let loaded = TemplateStore::load(&path).unwrap();
        assert_eq!(loaded.count(), 2);
        assert_eq!(loaded.get("rent").unwrap().exec_count, 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_list() {
        let store = make_store();
        assert_eq!(store.list().len(), 2);
    }

    #[test]
    fn test_due_initial_state() {
        let store = make_store();
        // Recurring templates with no execution and next_execution in the future
        // should not be immediately due (next_execution is set to future on create)
        let due = store.due();
        assert_eq!(due.len(), 0);
    }
}
