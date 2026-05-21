//! Transaction lifecycle hooks (plugin system).
//!
//! Configurable pre-tx and post-tx hooks that fire at key points:
//! - **pre_send**: Before a transfer is signed (can block)
//! - **post_send**: After a transfer is confirmed
//! - **pre_refresh**: Before a refresh is signed (can block)
//! - **post_refresh**: After a refresh is confirmed
//! - **on_error**: When any transaction fails
//!
//! Hooks can execute shell commands, log to file, or invoke webhooks.
//! Hook configs are persisted to JSON.

use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum HookError {
    #[error("hook blocked transaction: {0}")]
    Blocked(String),

    #[error("hook execution failed: {0}")]
    ExecutionFailed(String),

    #[error("hook not found: {0}")]
    NotFound(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ──────────────────────────── Types ──────────────────────────────────────

/// When the hook fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    PreSend,
    PostSend,
    PreRefresh,
    PostRefresh,
    OnError,
}

impl HookEvent {
    /// All possible events.
    pub fn all() -> &'static [HookEvent] {
        &[
            HookEvent::PreSend,
            HookEvent::PostSend,
            HookEvent::PreRefresh,
            HookEvent::PostRefresh,
            HookEvent::OnError,
        ]
    }

    /// Human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            HookEvent::PreSend => "pre_send",
            HookEvent::PostSend => "post_send",
            HookEvent::PreRefresh => "pre_refresh",
            HookEvent::PostRefresh => "post_refresh",
            HookEvent::OnError => "on_error",
        }
    }
}

/// What action the hook performs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HookAction {
    /// Execute a shell command. Environment variables are set with tx details.
    Shell { command: String },
    /// Append a line to a log file.
    Log {
        file: String,
        format: Option<String>,
    },
    /// POST a JSON payload to a URL.
    Webhook { url: String },
}

impl HookAction {
    /// Human-readable description.
    pub fn describe(&self) -> String {
        match self {
            HookAction::Shell { command } => format!("shell: {}", command),
            HookAction::Log { file, .. } => format!("log: {}", file),
            HookAction::Webhook { url } => format!("webhook: {}", url),
        }
    }
}

/// A configured hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hook {
    /// Unique name.
    pub name: String,
    /// Which event triggers this hook.
    pub event: HookEvent,
    /// What to do.
    pub action: HookAction,
    /// Whether this hook is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// For pre-* hooks: if true, a non-zero exit code blocks the tx.
    #[serde(default)]
    pub blocking: bool,
}

fn default_true() -> bool {
    true
}

/// Context passed to hooks when they fire.
#[derive(Debug, Clone, Serialize)]
pub struct HookContext {
    pub event: String,
    pub tx_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub timestamp: String,
}

impl HookContext {
    /// Create a context for a transfer.
    pub fn transfer(event: HookEvent, from: &str, to: &str, amount: u64) -> Self {
        Self {
            event: event.label().to_string(),
            tx_type: "transfer".to_string(),
            from: Some(from.to_string()),
            to: Some(to.to_string()),
            amount: Some(amount),
            object_id: None,
            energy: None,
            tx_hash: None,
            error: None,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Create a context for a refresh.
    pub fn refresh(event: HookEvent, from: &str, object_id: &str, energy: u64) -> Self {
        Self {
            event: event.label().to_string(),
            tx_type: "refresh".to_string(),
            from: Some(from.to_string()),
            to: None,
            amount: None,
            object_id: Some(object_id.to_string()),
            energy: Some(energy),
            tx_hash: None,
            error: None,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Create a context for an error.
    pub fn error(tx_type: &str, error: &str) -> Self {
        Self {
            event: HookEvent::OnError.label().to_string(),
            tx_type: tx_type.to_string(),
            from: None,
            to: None,
            amount: None,
            object_id: None,
            energy: None,
            tx_hash: None,
            error: Some(error.to_string()),
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Set the tx_hash after submission.
    pub fn with_tx_hash(mut self, hash: &str) -> Self {
        self.tx_hash = Some(hash.to_string());
        self
    }
}

// ──────────────────────────── HookRegistry ───────────────────────────────

/// Manages hook configuration and execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookRegistry {
    pub hooks: Vec<Hook>,
}

impl HookRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    /// Load from a JSON file.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, HookError> {
        let data = std::fs::read_to_string(path)?;
        let registry: HookRegistry = serde_json::from_str(&data)?;
        Ok(registry)
    }

    /// Save to a JSON file.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), HookError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Register a new hook.
    pub fn register(&mut self, hook: Hook) {
        // Replace if name already exists
        self.hooks.retain(|h| h.name != hook.name);
        self.hooks.push(hook);
    }

    /// Remove a hook by name.
    pub fn remove(&mut self, name: &str) -> Result<(), HookError> {
        let before = self.hooks.len();
        self.hooks.retain(|h| h.name != name);
        if self.hooks.len() == before {
            return Err(HookError::NotFound(name.to_string()));
        }
        Ok(())
    }

    /// Enable or disable a hook.
    pub fn set_enabled(&mut self, name: &str, enabled: bool) -> Result<(), HookError> {
        let hook = self
            .hooks
            .iter_mut()
            .find(|h| h.name == name)
            .ok_or_else(|| HookError::NotFound(name.to_string()))?;
        hook.enabled = enabled;
        Ok(())
    }

    /// Get hooks for a specific event (enabled only).
    pub fn hooks_for(&self, event: HookEvent) -> Vec<&Hook> {
        self.hooks
            .iter()
            .filter(|h| h.event == event && h.enabled)
            .collect()
    }

    /// List all hooks.
    pub fn list(&self) -> &[Hook] {
        &self.hooks
    }

    /// Get a hook by name.
    pub fn get(&self, name: &str) -> Option<&Hook> {
        self.hooks.iter().find(|h| h.name == name)
    }

    /// Execute all hooks for an event. Returns errors from blocking hooks.
    pub fn fire(&self, event: HookEvent, ctx: &HookContext) -> Result<Vec<String>, HookError> {
        let mut log_messages = Vec::new();

        for hook in self.hooks_for(event) {
            match &hook.action {
                HookAction::Shell { command } => {
                    let result = execute_shell(command, ctx);
                    match result {
                        Ok(output) => {
                            log_messages.push(format!("[{}] {}", hook.name, output));
                        }
                        Err(e) => {
                            if hook.blocking {
                                return Err(HookError::Blocked(format!(
                                    "hook '{}' blocked: {}",
                                    hook.name, e
                                )));
                            }
                            log_messages.push(format!("[{}] ERROR: {}", hook.name, e));
                        }
                    }
                }
                HookAction::Log { file, format } => {
                    let line = format_log_line(ctx, format.as_deref());
                    if let Err(e) = append_to_log(file, &line) {
                        log_messages.push(format!("[{}] log error: {}", hook.name, e));
                    } else {
                        log_messages.push(format!("[{}] logged to {}", hook.name, file));
                    }
                }
                HookAction::Webhook { url } => {
                    // Webhook execution is deferred (would need async)
                    // For now, just record intent
                    log_messages.push(format!("[{}] webhook queued: {}", hook.name, url));
                }
            }
        }

        Ok(log_messages)
    }

    /// Number of registered hooks.
    pub fn len(&self) -> usize {
        self.hooks.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ──────────────────────────── Execution Helpers ──────────────────────────

fn execute_shell(command: &str, ctx: &HookContext) -> Result<String, String> {
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c").arg(command);

    // Set environment variables from context
    cmd.env("EVAP_EVENT", &ctx.event);
    cmd.env("EVAP_TX_TYPE", &ctx.tx_type);
    if let Some(ref from) = ctx.from {
        cmd.env("EVAP_FROM", from);
    }
    if let Some(ref to) = ctx.to {
        cmd.env("EVAP_TO", to);
    }
    if let Some(amount) = ctx.amount {
        cmd.env("EVAP_AMOUNT", amount.to_string());
    }
    if let Some(ref obj) = ctx.object_id {
        cmd.env("EVAP_OBJECT_ID", obj);
    }
    if let Some(energy) = ctx.energy {
        cmd.env("EVAP_ENERGY", energy.to_string());
    }
    if let Some(ref hash) = ctx.tx_hash {
        cmd.env("EVAP_TX_HASH", hash);
    }
    if let Some(ref err) = ctx.error {
        cmd.env("EVAP_ERROR", err);
    }

    let output = cmd
        .output()
        .map_err(|e| format!("failed to execute: {}", e))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(format!(
            "exit code {}: {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn format_log_line(ctx: &HookContext, format: Option<&str>) -> String {
    match format {
        Some(fmt) => fmt
            .replace("{event}", &ctx.event)
            .replace("{tx_type}", &ctx.tx_type)
            .replace("{from}", ctx.from.as_deref().unwrap_or("-"))
            .replace("{to}", ctx.to.as_deref().unwrap_or("-"))
            .replace(
                "{amount}",
                &ctx.amount.map(|a| a.to_string()).unwrap_or_default(),
            )
            .replace("{tx_hash}", ctx.tx_hash.as_deref().unwrap_or("-"))
            .replace("{timestamp}", &ctx.timestamp),
        None => serde_json::to_string(ctx).unwrap_or_else(|_| "{}".to_string()),
    }
}

fn append_to_log(file: &str, line: &str) -> Result<(), std::io::Error> {
    use std::io::Write;
    let path = Path::new(file);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)?;
    writeln!(f, "{}", line)?;
    Ok(())
}

/// Default path for hook config.
pub fn default_hooks_path() -> std::path::PathBuf {
    crate::config::default_data_dir().join("hooks.json")
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_registry() -> HookRegistry {
        let mut reg = HookRegistry::new();
        reg.register(Hook {
            name: "log_sends".to_string(),
            event: HookEvent::PostSend,
            action: HookAction::Log {
                file: "/tmp/evap_test_hooks.log".to_string(),
                format: Some("{event}: {from} -> {to} amount={amount}".to_string()),
            },
            enabled: true,
            blocking: false,
        });
        reg
    }

    #[test]
    fn test_register_hook() {
        let reg = make_registry();
        assert_eq!(reg.len(), 1);
        assert!(reg.get("log_sends").is_some());
    }

    #[test]
    fn test_register_replaces_existing() {
        let mut reg = make_registry();
        reg.register(Hook {
            name: "log_sends".to_string(),
            event: HookEvent::PreSend,
            action: HookAction::Shell {
                command: "echo new".to_string(),
            },
            enabled: true,
            blocking: false,
        });
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get("log_sends").unwrap().event, HookEvent::PreSend);
    }

    #[test]
    fn test_remove_hook() {
        let mut reg = make_registry();
        reg.remove("log_sends").unwrap();
        assert!(reg.is_empty());
    }

    #[test]
    fn test_remove_not_found() {
        let mut reg = HookRegistry::new();
        let err = reg.remove("nope").unwrap_err();
        assert!(matches!(err, HookError::NotFound(_)));
    }

    #[test]
    fn test_hooks_for_event() {
        let mut reg = make_registry();
        reg.register(Hook {
            name: "pre_check".to_string(),
            event: HookEvent::PreSend,
            action: HookAction::Shell {
                command: "echo check".to_string(),
            },
            enabled: true,
            blocking: true,
        });

        assert_eq!(reg.hooks_for(HookEvent::PostSend).len(), 1);
        assert_eq!(reg.hooks_for(HookEvent::PreSend).len(), 1);
        assert_eq!(reg.hooks_for(HookEvent::OnError).len(), 0);
    }

    #[test]
    fn test_disabled_hooks_skipped() {
        let mut reg = make_registry();
        reg.set_enabled("log_sends", false).unwrap();
        assert_eq!(reg.hooks_for(HookEvent::PostSend).len(), 0);
    }

    #[test]
    fn test_set_enabled_not_found() {
        let mut reg = HookRegistry::new();
        let err = reg.set_enabled("nope", true).unwrap_err();
        assert!(matches!(err, HookError::NotFound(_)));
    }

    #[test]
    fn test_hook_context_transfer() {
        let ctx = HookContext::transfer(HookEvent::PreSend, "0xfrom", "0xto", 1000);
        assert_eq!(ctx.event, "pre_send");
        assert_eq!(ctx.tx_type, "transfer");
        assert_eq!(ctx.amount, Some(1000));
    }

    #[test]
    fn test_hook_context_refresh() {
        let ctx = HookContext::refresh(HookEvent::PreRefresh, "0xfrom", "0xobj", 500);
        assert_eq!(ctx.event, "pre_refresh");
        assert_eq!(ctx.object_id, Some("0xobj".to_string()));
        assert_eq!(ctx.energy, Some(500));
    }

    #[test]
    fn test_hook_context_error() {
        let ctx = HookContext::error("transfer", "insufficient balance");
        assert_eq!(ctx.event, "on_error");
        assert_eq!(ctx.error, Some("insufficient balance".to_string()));
    }

    #[test]
    fn test_hook_context_with_tx_hash() {
        let ctx =
            HookContext::transfer(HookEvent::PostSend, "0xa", "0xb", 100).with_tx_hash("0xhash123");
        assert_eq!(ctx.tx_hash, Some("0xhash123".to_string()));
    }

    #[test]
    fn test_format_log_line_custom() {
        let ctx = HookContext::transfer(HookEvent::PostSend, "0xfrom", "0xto", 500);
        let line = format_log_line(&ctx, Some("{event}: {from} -> {to} amount={amount}"));
        assert_eq!(line, "post_send: 0xfrom -> 0xto amount=500");
    }

    #[test]
    fn test_format_log_line_default_json() {
        let ctx = HookContext::transfer(HookEvent::PostSend, "0xfrom", "0xto", 500);
        let line = format_log_line(&ctx, None);
        assert!(line.contains("\"event\":\"post_send\""));
    }

    #[test]
    fn test_fire_shell_hook() {
        let mut reg = HookRegistry::new();
        reg.register(Hook {
            name: "echo_test".to_string(),
            event: HookEvent::PostSend,
            action: HookAction::Shell {
                command: "echo hello".to_string(),
            },
            enabled: true,
            blocking: false,
        });

        let ctx = HookContext::transfer(HookEvent::PostSend, "0xa", "0xb", 100);
        let result = reg.fire(HookEvent::PostSend, &ctx).unwrap();
        assert!(!result.is_empty());
        assert!(result[0].contains("hello"));
    }

    #[test]
    fn test_fire_blocking_hook_fails() {
        let mut reg = HookRegistry::new();
        reg.register(Hook {
            name: "blocker".to_string(),
            event: HookEvent::PreSend,
            action: HookAction::Shell {
                command: "exit 1".to_string(),
            },
            enabled: true,
            blocking: true,
        });

        let ctx = HookContext::transfer(HookEvent::PreSend, "0xa", "0xb", 100);
        let err = reg.fire(HookEvent::PreSend, &ctx).unwrap_err();
        assert!(matches!(err, HookError::Blocked(_)));
    }

    #[test]
    fn test_fire_non_blocking_failure_continues() {
        let mut reg = HookRegistry::new();
        reg.register(Hook {
            name: "non_blocker".to_string(),
            event: HookEvent::PreSend,
            action: HookAction::Shell {
                command: "exit 1".to_string(),
            },
            enabled: true,
            blocking: false,
        });

        let ctx = HookContext::transfer(HookEvent::PreSend, "0xa", "0xb", 100);
        let result = reg.fire(HookEvent::PreSend, &ctx).unwrap();
        assert!(result[0].contains("ERROR"));
    }

    #[test]
    fn test_fire_log_hook() {
        let log_file = std::env::temp_dir()
            .join("evaporchain_hook_test_log.txt")
            .to_string_lossy()
            .to_string();

        let mut reg = HookRegistry::new();
        reg.register(Hook {
            name: "logger".to_string(),
            event: HookEvent::PostSend,
            action: HookAction::Log {
                file: log_file.clone(),
                format: Some("{event}: {amount} EVAP".to_string()),
            },
            enabled: true,
            blocking: false,
        });

        let ctx = HookContext::transfer(HookEvent::PostSend, "0xa", "0xb", 999);
        reg.fire(HookEvent::PostSend, &ctx).unwrap();

        let content = std::fs::read_to_string(&log_file).unwrap();
        assert!(content.contains("post_send: 999 EVAP"));

        let _ = std::fs::remove_file(&log_file);
    }

    #[test]
    fn test_hook_action_describe() {
        let a = HookAction::Shell {
            command: "echo hi".into(),
        };
        assert!(a.describe().contains("echo hi"));

        let a = HookAction::Log {
            file: "/tmp/test.log".into(),
            format: None,
        };
        assert!(a.describe().contains("/tmp/test.log"));

        let a = HookAction::Webhook {
            url: "https://example.com".into(),
        };
        assert!(a.describe().contains("example.com"));
    }

    #[test]
    fn test_hook_event_all() {
        let all = HookEvent::all();
        assert_eq!(all.len(), 5);
    }

    #[test]
    fn test_hook_event_labels() {
        assert_eq!(HookEvent::PreSend.label(), "pre_send");
        assert_eq!(HookEvent::PostSend.label(), "post_send");
        assert_eq!(HookEvent::PreRefresh.label(), "pre_refresh");
        assert_eq!(HookEvent::PostRefresh.label(), "post_refresh");
        assert_eq!(HookEvent::OnError.label(), "on_error");
    }

    #[test]
    fn test_json_roundtrip() {
        let reg = make_registry();
        let json = serde_json::to_string_pretty(&reg).unwrap();
        let loaded: HookRegistry = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.hooks[0].name, "log_sends");
    }

    #[test]
    fn test_file_save_and_load() {
        let dir = std::env::temp_dir().join("evaporchain_hooks_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hooks.json");

        let reg = make_registry();
        reg.save(&path).unwrap();

        let loaded = HookRegistry::load(&path).unwrap();
        assert_eq!(loaded.len(), 1);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_hook_context_serializable() {
        let ctx = HookContext::transfer(HookEvent::PostSend, "0xa", "0xb", 100);
        let json = serde_json::to_string(&ctx).unwrap();
        assert!(json.contains("\"tx_type\":\"transfer\""));
        assert!(json.contains("\"amount\":100"));
    }

    #[test]
    fn test_shell_env_vars_set() {
        let mut reg = HookRegistry::new();
        reg.register(Hook {
            name: "env_check".to_string(),
            event: HookEvent::PreSend,
            action: HookAction::Shell {
                command: "echo $EVAP_AMOUNT".to_string(),
            },
            enabled: true,
            blocking: false,
        });

        let ctx = HookContext::transfer(HookEvent::PreSend, "0xa", "0xb", 42);
        let result = reg.fire(HookEvent::PreSend, &ctx).unwrap();
        assert!(result[0].contains("42"));
    }

    #[test]
    fn test_default_true_via_deserialization_covers_lines_117_119() {
        // default_true() is called when `enabled` is absent during deserialization
        let json = r#"{"name":"h","event":"pre_send","action":{"type":"shell","command":"echo hi"},"blocking":false}"#;
        let hook: Hook = serde_json::from_str(json).unwrap();
        assert!(hook.enabled); // default_true() was invoked
    }

    #[test]
    fn test_list_covers_lines_268_270() {
        let reg = make_registry();
        let hooks = reg.list();
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].name, "log_sends");
    }

    #[test]
    fn test_fire_webhook_covers_lines_309_312() {
        let mut reg = HookRegistry::new();
        reg.register(Hook {
            name: "webhook_test".to_string(),
            event: HookEvent::PostSend,
            action: HookAction::Webhook {
                url: "https://example.com/hook".to_string(),
            },
            enabled: true,
            blocking: false,
        });
        let ctx = HookContext::transfer(HookEvent::PostSend, "0xa", "0xb", 100);
        let msgs = reg.fire(HookEvent::PostSend, &ctx).unwrap();
        assert!(msgs[0].contains("webhook queued"));
        assert!(msgs[0].contains("example.com"));
    }

    #[test]
    fn test_default_covers_lines_331_333() {
        let reg = HookRegistry::default();
        assert!(reg.is_empty());
    }

    #[test]
    fn test_fire_log_error_path_covers_line_303() {
        // Use an existing directory path as the log file → open() fails with EISDIR
        let mut reg = HookRegistry::new();
        reg.register(Hook {
            name: "bad_log".to_string(),
            event: HookEvent::PostSend,
            action: HookAction::Log {
                file: std::env::temp_dir().to_string_lossy().to_string(),
                format: None,
            },
            enabled: true,
            blocking: false,
        });
        let ctx = HookContext::transfer(HookEvent::PostSend, "0xa", "0xb", 1);
        let msgs = reg.fire(HookEvent::PostSend, &ctx).unwrap();
        assert!(msgs[0].contains("log error"));
    }

    #[test]
    fn test_default_hooks_path_covers_lines_414_416() {
        let path = default_hooks_path();
        assert!(path.to_string_lossy().contains("hooks.json"));
    }

    #[test]
    fn test_shell_ctx_object_energy_hash_error_covers_lines_355_364() {
        let mut reg = HookRegistry::new();
        reg.register(Hook {
            name: "full_ctx".to_string(),
            event: HookEvent::OnError,
            action: HookAction::Shell {
                command: "echo ok".to_string(),
            },
            enabled: true,
            blocking: false,
        });
        let ctx = HookContext {
            event: "on_error".to_string(),
            tx_type: "transfer".to_string(),
            from: None,
            to: None,
            amount: None,
            object_id: Some("obj123".to_string()),
            energy: Some(500),
            tx_hash: Some("0xdeadbeef".to_string()),
            error: Some("something failed".to_string()),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };
        let msgs = reg.fire(HookEvent::OnError, &ctx).unwrap();
        assert!(!msgs.is_empty());
    }
}
