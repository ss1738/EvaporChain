// wallet/src/plugin.rs — Plugin system for extensible wallet functionality
//
// Plugins are registered with a manifest (name, version, hooks, permissions).
// The registry manages lifecycle: install, enable, disable, uninstall.
// Plugins hook into wallet events via a typed hook system.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("plugin not found: {0}")]
    NotFound(String),
    #[error("plugin already exists: {0}")]
    AlreadyExists(String),
    #[error("plugin disabled: {0}")]
    Disabled(String),
    #[error("invalid manifest: {0}")]
    InvalidManifest(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("hook error: {0}")]
    HookError(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
}

// ── Plugin manifest ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub hooks: Vec<HookPoint>,
    pub permissions: Vec<Permission>,
    pub config_schema: Option<HashMap<String, String>>,
}

impl PluginManifest {
    pub fn validate(&self) -> Result<(), PluginError> {
        if self.name.is_empty() {
            return Err(PluginError::InvalidManifest("name is required".into()));
        }
        if self.version.is_empty() {
            return Err(PluginError::InvalidManifest("version is required".into()));
        }
        if self.name.contains(' ') {
            return Err(PluginError::InvalidManifest(
                "name cannot contain spaces".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HookPoint {
    BeforeTransfer,
    AfterTransfer,
    BeforeSign,
    AfterSign,
    BeforeRefresh,
    AfterRefresh,
    OnBalanceChange,
    OnEnergyAlert,
    OnNewBlock,
    OnError,
    OnStartup,
    OnShutdown,
}

impl HookPoint {
    pub fn name(&self) -> &'static str {
        match self {
            HookPoint::BeforeTransfer => "before_transfer",
            HookPoint::AfterTransfer => "after_transfer",
            HookPoint::BeforeSign => "before_sign",
            HookPoint::AfterSign => "after_sign",
            HookPoint::BeforeRefresh => "before_refresh",
            HookPoint::AfterRefresh => "after_refresh",
            HookPoint::OnBalanceChange => "on_balance_change",
            HookPoint::OnEnergyAlert => "on_energy_alert",
            HookPoint::OnNewBlock => "on_new_block",
            HookPoint::OnError => "on_error",
            HookPoint::OnStartup => "on_startup",
            HookPoint::OnShutdown => "on_shutdown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Permission {
    ReadBalance,
    ReadHistory,
    ReadContacts,
    SendTransactions,
    SignMessages,
    ManageKeys,
    NetworkAccess,
    FileAccess,
    Notifications,
}

impl Permission {
    pub fn is_dangerous(&self) -> bool {
        matches!(
            self,
            Permission::SendTransactions
                | Permission::SignMessages
                | Permission::ManageKeys
                | Permission::FileAccess
        )
    }

    pub fn name(&self) -> &'static str {
        match self {
            Permission::ReadBalance => "read_balance",
            Permission::ReadHistory => "read_history",
            Permission::ReadContacts => "read_contacts",
            Permission::SendTransactions => "send_transactions",
            Permission::SignMessages => "sign_messages",
            Permission::ManageKeys => "manage_keys",
            Permission::NetworkAccess => "network_access",
            Permission::FileAccess => "file_access",
            Permission::Notifications => "notifications",
        }
    }
}

// ── Plugin entry ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plugin {
    pub manifest: PluginManifest,
    pub enabled: bool,
    pub installed_at: String,
    pub config: HashMap<String, String>,
    pub execution_count: u64,
    pub last_executed: Option<String>,
    pub errors: Vec<String>,
}

impl Plugin {
    pub fn new(manifest: PluginManifest) -> Self {
        Self {
            manifest,
            enabled: true,
            installed_at: chrono::Utc::now().to_rfc3339(),
            config: HashMap::new(),
            execution_count: 0,
            last_executed: None,
            errors: Vec::new(),
        }
    }

    pub fn record_execution(&mut self) {
        self.execution_count += 1;
        self.last_executed = Some(chrono::Utc::now().to_rfc3339());
    }

    pub fn record_error(&mut self, msg: &str) {
        self.errors
            .push(format!("[{}] {}", chrono::Utc::now().to_rfc3339(), msg));
        // Cap error log at 100
        if self.errors.len() > 100 {
            self.errors.drain(0..self.errors.len() - 100);
        }
    }

    pub fn has_permission(&self, perm: Permission) -> bool {
        self.manifest.permissions.contains(&perm)
    }

    pub fn hooks_into(&self, hook: HookPoint) -> bool {
        self.manifest.hooks.contains(&hook)
    }

    pub fn dangerous_permissions(&self) -> Vec<Permission> {
        self.manifest
            .permissions
            .iter()
            .filter(|p| p.is_dangerous())
            .copied()
            .collect()
    }
}

// ── Plugin registry ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginRegistry {
    pub plugins: HashMap<String, Plugin>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    /// Install a plugin from its manifest
    pub fn install(&mut self, manifest: PluginManifest) -> Result<(), PluginError> {
        manifest.validate()?;
        if self.plugins.contains_key(&manifest.name) {
            return Err(PluginError::AlreadyExists(manifest.name.clone()));
        }
        let plugin = Plugin::new(manifest.clone());
        self.plugins.insert(manifest.name, plugin);
        Ok(())
    }

    /// Uninstall a plugin
    pub fn uninstall(&mut self, name: &str) -> Result<Plugin, PluginError> {
        self.plugins
            .remove(name)
            .ok_or_else(|| PluginError::NotFound(name.into()))
    }

    /// Enable a plugin
    pub fn enable(&mut self, name: &str) -> Result<(), PluginError> {
        let plugin = self
            .plugins
            .get_mut(name)
            .ok_or_else(|| PluginError::NotFound(name.into()))?;
        plugin.enabled = true;
        Ok(())
    }

    /// Disable a plugin
    pub fn disable(&mut self, name: &str) -> Result<(), PluginError> {
        let plugin = self
            .plugins
            .get_mut(name)
            .ok_or_else(|| PluginError::NotFound(name.into()))?;
        plugin.enabled = false;
        Ok(())
    }

    /// Get a plugin by name
    pub fn get(&self, name: &str) -> Option<&Plugin> {
        self.plugins.get(name)
    }

    /// List all plugins
    pub fn list(&self) -> Vec<&Plugin> {
        self.plugins.values().collect()
    }

    /// List enabled plugins
    pub fn enabled(&self) -> Vec<&Plugin> {
        self.plugins.values().filter(|p| p.enabled).collect()
    }

    /// List disabled plugins
    pub fn disabled(&self) -> Vec<&Plugin> {
        self.plugins.values().filter(|p| !p.enabled).collect()
    }

    /// Get all plugins that hook into a specific point
    pub fn hooks_for(&self, hook: HookPoint) -> Vec<&Plugin> {
        self.plugins
            .values()
            .filter(|p| p.enabled && p.hooks_into(hook))
            .collect()
    }

    /// Check if any plugin would block a permission-gated action
    pub fn check_permission(&self, name: &str, perm: Permission) -> Result<(), PluginError> {
        let plugin = self
            .plugins
            .get(name)
            .ok_or_else(|| PluginError::NotFound(name.into()))?;
        if !plugin.enabled {
            return Err(PluginError::Disabled(name.into()));
        }
        if !plugin.has_permission(perm) {
            return Err(PluginError::PermissionDenied(format!(
                "{} does not have {} permission",
                name,
                perm.name()
            )));
        }
        Ok(())
    }

    /// Update plugin config
    pub fn set_config(&mut self, name: &str, key: &str, value: &str) -> Result<(), PluginError> {
        let plugin = self
            .plugins
            .get_mut(name)
            .ok_or_else(|| PluginError::NotFound(name.into()))?;
        plugin.config.insert(key.to_string(), value.to_string());
        Ok(())
    }

    /// Get plugin config value
    pub fn get_config(&self, name: &str, key: &str) -> Result<Option<&String>, PluginError> {
        let plugin = self
            .plugins
            .get(name)
            .ok_or_else(|| PluginError::NotFound(name.into()))?;
        Ok(plugin.config.get(key))
    }

    /// Audit: list all dangerous permissions across all plugins
    pub fn audit_permissions(&self) -> Vec<(String, Vec<Permission>)> {
        self.plugins
            .iter()
            .filter_map(|(name, plugin)| {
                let dangerous = plugin.dangerous_permissions();
                if dangerous.is_empty() {
                    None
                } else {
                    Some((name.clone(), dangerous))
                }
            })
            .collect()
    }

    /// JSON persistence
    pub fn save(&self, path: &Path) -> Result<(), PluginError> {
        let json =
            serde_json::to_string_pretty(self).map_err(|e| PluginError::Json(e.to_string()))?;
        std::fs::write(path, json).map_err(|e| PluginError::Io(e.to_string()))
    }

    pub fn load(path: &Path) -> Result<Self, PluginError> {
        let data = std::fs::read_to_string(path).map_err(|e| PluginError::Io(e.to_string()))?;
        serde_json::from_str(&data).map_err(|e| PluginError::Json(e.to_string()))
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_manifest() -> PluginManifest {
        PluginManifest {
            name: "test-plugin".to_string(),
            version: "1.0.0".to_string(),
            author: "Test".to_string(),
            description: "A test plugin".to_string(),
            hooks: vec![HookPoint::AfterTransfer, HookPoint::OnBalanceChange],
            permissions: vec![Permission::ReadBalance, Permission::Notifications],
            config_schema: None,
        }
    }

    fn dangerous_manifest() -> PluginManifest {
        PluginManifest {
            name: "dangerous-plugin".to_string(),
            version: "0.1.0".to_string(),
            author: "Evil".to_string(),
            description: "Needs lots of perms".to_string(),
            hooks: vec![HookPoint::BeforeSign],
            permissions: vec![
                Permission::SendTransactions,
                Permission::ManageKeys,
                Permission::FileAccess,
            ],
            config_schema: None,
        }
    }

    #[test]
    fn test_manifest_validate_ok() {
        assert!(test_manifest().validate().is_ok());
    }

    #[test]
    fn test_manifest_validate_empty_name() {
        let mut m = test_manifest();
        m.name = "".to_string();
        assert!(m.validate().is_err());
    }

    #[test]
    fn test_manifest_validate_empty_version() {
        let mut m = test_manifest();
        m.version = "".to_string();
        assert!(m.validate().is_err());
    }

    #[test]
    fn test_manifest_validate_spaces_in_name() {
        let mut m = test_manifest();
        m.name = "my plugin".to_string();
        assert!(m.validate().is_err());
    }

    #[test]
    fn test_install_plugin() {
        let mut reg = PluginRegistry::new();
        reg.install(test_manifest()).unwrap();
        assert_eq!(reg.list().len(), 1);
        assert!(reg.get("test-plugin").is_some());
    }

    #[test]
    fn test_install_duplicate() {
        let mut reg = PluginRegistry::new();
        reg.install(test_manifest()).unwrap();
        assert!(reg.install(test_manifest()).is_err());
    }

    #[test]
    fn test_uninstall_plugin() {
        let mut reg = PluginRegistry::new();
        reg.install(test_manifest()).unwrap();
        let p = reg.uninstall("test-plugin").unwrap();
        assert_eq!(p.manifest.name, "test-plugin");
        assert!(reg.get("test-plugin").is_none());
    }

    #[test]
    fn test_uninstall_not_found() {
        let mut reg = PluginRegistry::new();
        assert!(reg.uninstall("nope").is_err());
    }

    #[test]
    fn test_enable_disable() {
        let mut reg = PluginRegistry::new();
        reg.install(test_manifest()).unwrap();
        reg.disable("test-plugin").unwrap();
        assert!(!reg.get("test-plugin").unwrap().enabled);
        reg.enable("test-plugin").unwrap();
        assert!(reg.get("test-plugin").unwrap().enabled);
    }

    #[test]
    fn test_enabled_disabled_lists() {
        let mut reg = PluginRegistry::new();
        reg.install(test_manifest()).unwrap();
        reg.install(dangerous_manifest()).unwrap();
        reg.disable("dangerous-plugin").unwrap();
        assert_eq!(reg.enabled().len(), 1);
        assert_eq!(reg.disabled().len(), 1);
    }

    #[test]
    fn test_hooks_for() {
        let mut reg = PluginRegistry::new();
        reg.install(test_manifest()).unwrap();
        let hooks = reg.hooks_for(HookPoint::AfterTransfer);
        assert_eq!(hooks.len(), 1);
        let hooks = reg.hooks_for(HookPoint::BeforeSign);
        assert_eq!(hooks.len(), 0);
    }

    #[test]
    fn test_hooks_for_disabled_plugin() {
        let mut reg = PluginRegistry::new();
        reg.install(test_manifest()).unwrap();
        reg.disable("test-plugin").unwrap();
        let hooks = reg.hooks_for(HookPoint::AfterTransfer);
        assert_eq!(hooks.len(), 0);
    }

    #[test]
    fn test_check_permission_ok() {
        let mut reg = PluginRegistry::new();
        reg.install(test_manifest()).unwrap();
        assert!(reg
            .check_permission("test-plugin", Permission::ReadBalance)
            .is_ok());
    }

    #[test]
    fn test_check_permission_denied() {
        let mut reg = PluginRegistry::new();
        reg.install(test_manifest()).unwrap();
        assert!(reg
            .check_permission("test-plugin", Permission::SendTransactions)
            .is_err());
    }

    #[test]
    fn test_check_permission_disabled() {
        let mut reg = PluginRegistry::new();
        reg.install(test_manifest()).unwrap();
        reg.disable("test-plugin").unwrap();
        assert!(reg
            .check_permission("test-plugin", Permission::ReadBalance)
            .is_err());
    }

    #[test]
    fn test_plugin_config() {
        let mut reg = PluginRegistry::new();
        reg.install(test_manifest()).unwrap();
        reg.set_config("test-plugin", "key", "value").unwrap();
        let val = reg.get_config("test-plugin", "key").unwrap();
        assert_eq!(val, Some(&"value".to_string()));
    }

    #[test]
    fn test_plugin_config_not_found() {
        let reg = PluginRegistry::new();
        assert!(reg.get_config("nope", "key").is_err());
    }

    #[test]
    fn test_dangerous_permissions() {
        let p = Plugin::new(dangerous_manifest());
        let dangerous = p.dangerous_permissions();
        assert_eq!(dangerous.len(), 3);
    }

    #[test]
    fn test_audit_permissions() {
        let mut reg = PluginRegistry::new();
        reg.install(test_manifest()).unwrap();
        reg.install(dangerous_manifest()).unwrap();
        let audit = reg.audit_permissions();
        // Only dangerous-plugin should appear
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].0, "dangerous-plugin");
    }

    #[test]
    fn test_plugin_execution_tracking() {
        let mut p = Plugin::new(test_manifest());
        assert_eq!(p.execution_count, 0);
        assert!(p.last_executed.is_none());
        p.record_execution();
        assert_eq!(p.execution_count, 1);
        assert!(p.last_executed.is_some());
    }

    #[test]
    fn test_plugin_error_tracking() {
        let mut p = Plugin::new(test_manifest());
        p.record_error("something broke");
        assert_eq!(p.errors.len(), 1);
        assert!(p.errors[0].contains("something broke"));
    }

    #[test]
    fn test_plugin_error_cap() {
        let mut p = Plugin::new(test_manifest());
        for i in 0..120 {
            p.record_error(&format!("error {}", i));
        }
        assert_eq!(p.errors.len(), 100);
    }

    #[test]
    fn test_hook_point_name() {
        assert_eq!(HookPoint::BeforeTransfer.name(), "before_transfer");
        assert_eq!(HookPoint::OnShutdown.name(), "on_shutdown");
    }

    #[test]
    fn test_permission_is_dangerous() {
        assert!(Permission::SendTransactions.is_dangerous());
        assert!(Permission::ManageKeys.is_dangerous());
        assert!(!Permission::ReadBalance.is_dangerous());
        assert!(!Permission::Notifications.is_dangerous());
    }

    #[test]
    fn test_save_load() {
        let path = std::env::temp_dir().join(format!("evap_plugins_{}.json", std::process::id()));
        let mut reg = PluginRegistry::new();
        reg.install(test_manifest()).unwrap();
        reg.save(&path).unwrap();
        let loaded = PluginRegistry::load(&path).unwrap();
        assert_eq!(loaded.plugins.len(), 1);
        assert!(loaded.get("test-plugin").is_some());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default() {
        let path = std::env::temp_dir().join("evap_plugins_noexist.json");
        let reg = PluginRegistry::load_or_default(&path);
        assert_eq!(reg.plugins.len(), 0);
    }

    #[test]
    fn test_hook_point_name_all_variants_covers_lines_82_91() {
        assert_eq!(HookPoint::BeforeSign.name(), "before_sign");
        assert_eq!(HookPoint::AfterSign.name(), "after_sign");
        assert_eq!(HookPoint::BeforeRefresh.name(), "before_refresh");
        assert_eq!(HookPoint::AfterRefresh.name(), "after_refresh");
        assert_eq!(HookPoint::OnBalanceChange.name(), "on_balance_change");
        assert_eq!(HookPoint::OnEnergyAlert.name(), "on_energy_alert");
        assert_eq!(HookPoint::OnNewBlock.name(), "on_new_block");
        assert_eq!(HookPoint::OnError.name(), "on_error");
        assert_eq!(HookPoint::OnStartup.name(), "on_startup");
        assert_eq!(HookPoint::OnShutdown.name(), "on_shutdown");
    }

    #[test]
    fn test_permission_name_all_variants_covers_lines_123_131() {
        assert_eq!(Permission::ReadContacts.name(), "read_contacts");
        assert_eq!(Permission::SendTransactions.name(), "send_transactions");
        assert_eq!(Permission::SignMessages.name(), "sign_messages");
        assert_eq!(Permission::ManageKeys.name(), "manage_keys");
        assert_eq!(Permission::NetworkAccess.name(), "network_access");
        assert_eq!(Permission::FileAccess.name(), "file_access");
        assert_eq!(Permission::Notifications.name(), "notifications");
    }
}
