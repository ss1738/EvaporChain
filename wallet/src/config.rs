//! Persistent wallet configuration.
//!
//! Stores user preferences (node URL, active account, keystore path)
//! in `~/.evaporchain/config.json`. Created on first use with defaults.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ──────────────────────────── Config ──────────────────────────────────

/// Wallet configuration persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletConfig {
    /// Node RPC base URL.
    pub node_url: String,
    /// Path to the keystore file.
    pub keystore_path: String,
    /// Path to the address book file.
    pub contacts_path: String,
    /// Path to the transaction history file.
    pub history_path: String,
    /// Currently active account name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_account: Option<String>,
    /// Default half-life for new objects (epochs).
    pub default_half_life: u64,
    /// Default energy for new objects.
    pub default_energy: u64,
}

impl Default for WalletConfig {
    fn default() -> Self {
        let base = default_data_dir();
        Self {
            node_url: "http://localhost:3000".to_string(),
            keystore_path: base.join("keystore.json").to_string_lossy().to_string(),
            contacts_path: base.join("contacts.json").to_string_lossy().to_string(),
            history_path: base.join("history.json").to_string_lossy().to_string(),
            active_account: None,
            default_half_life: 100,
            default_energy: 1000,
        }
    }
}

impl WalletConfig {
    /// Load config from file, or create default if it doesn't exist.
    pub fn load_or_default<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        if path.exists() {
            let data = std::fs::read_to_string(path)?;
            let config: WalletConfig = serde_json::from_str(&data)?;
            Ok(config)
        } else {
            let config = Self::default();
            config.save(path)?;
            Ok(config)
        }
    }

    /// Save config to file (creating parent dirs if needed).
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), ConfigError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Get the default config file path.
    pub fn default_path() -> PathBuf {
        default_data_dir().join("config.json")
    }
}

/// Default data directory: ~/.evaporchain/
pub fn default_data_dir() -> PathBuf {
    dirs_next().join(".evaporchain")
}

fn dirs_next() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

// ──────────────────────────── Tests ──────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = WalletConfig::default();
        assert_eq!(config.node_url, "http://localhost:3000");
        assert!(config.active_account.is_none());
        assert_eq!(config.default_half_life, 100);
    }

    #[test]
    fn test_json_roundtrip() {
        let config = WalletConfig {
            active_account: Some("alice".to_string()),
            node_url: "http://mainnet:3000".to_string(),
            ..WalletConfig::default()
        };

        let json = serde_json::to_string_pretty(&config).unwrap();
        let loaded: WalletConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.node_url, "http://mainnet:3000");
        assert_eq!(loaded.active_account.as_deref(), Some("alice"));
    }

    #[test]
    fn test_save_and_load() {
        let dir = std::env::temp_dir().join("evaporchain_config_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");

        let config = WalletConfig {
            node_url: "http://custom:8080".to_string(),
            ..WalletConfig::default()
        };
        config.save(&path).unwrap();

        let loaded = WalletConfig::load_or_default(&path).unwrap();
        assert_eq!(loaded.node_url, "http://custom:8080");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_load_creates_default_if_missing() {
        let dir = std::env::temp_dir().join("evaporchain_config_test_create");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("new_config.json");

        assert!(!path.exists());
        let config = WalletConfig::load_or_default(&path).unwrap();
        assert!(path.exists());
        assert_eq!(config.node_url, "http://localhost:3000");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_default_path_not_empty() {
        let path = WalletConfig::default_path();
        assert!(path.to_string_lossy().contains("evaporchain"));
    }
}
