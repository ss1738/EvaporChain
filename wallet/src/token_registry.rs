//! Token metadata registry — look up token info, flag scams, verify tokens.
//!
//! Stores token metadata keyed by contract address, persisted to JSON.
//! Supports register, update, remove, search, scam flagging, CSV import/export.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum TokenRegistryError {
    #[error("token already exists: {0}")]
    AlreadyExists(String),
    #[error("token not found: {0}")]
    NotFound(String),
    #[error("invalid amount: {0}")]
    InvalidAmount(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ──────────────────────────── TokenInfo ──────────────────────────────────

/// Metadata for a single token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenInfo {
    pub address: String,
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    pub verified: bool,
    pub flagged_scam: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scam_reason: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub added_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_supply: Option<u64>,
    #[serde(default)]
    pub custom_fields: HashMap<String, String>,
}

impl TokenInfo {
    /// Create a new token with defaults and timestamps.
    pub fn new(address: &str, name: &str, symbol: &str, decimals: u8) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            address: address.to_string(),
            name: name.to_string(),
            symbol: symbol.to_string(),
            decimals,
            logo_url: None,
            website: None,
            verified: false,
            flagged_scam: false,
            scam_reason: None,
            tags: Vec::new(),
            added_at: now.clone(),
            updated_at: now,
            total_supply: None,
            custom_fields: HashMap::new(),
        }
    }

    /// Builder: set logo URL.
    pub fn with_logo(mut self, url: &str) -> Self {
        self.logo_url = Some(url.to_string());
        self
    }

    /// Builder: set website URL.
    pub fn with_website(mut self, url: &str) -> Self {
        self.website = Some(url.to_string());
        self
    }

    /// Builder: mark as verified.
    pub fn verify(mut self) -> Self {
        self.verified = true;
        self.updated_at = chrono::Utc::now().to_rfc3339();
        self
    }

    /// Flag this token as a scam.
    pub fn flag_scam(&mut self, reason: &str) {
        self.flagged_scam = true;
        self.scam_reason = Some(reason.to_string());
        self.updated_at = chrono::Utc::now().to_rfc3339();
    }

    /// Clear the scam flag.
    pub fn clear_scam_flag(&mut self) {
        self.flagged_scam = false;
        self.scam_reason = None;
        self.updated_at = chrono::Utc::now().to_rfc3339();
    }

    /// Add a tag.
    pub fn add_tag(&mut self, tag: &str) {
        let tag = tag.to_string();
        if !self.tags.contains(&tag) {
            self.tags.push(tag);
            self.updated_at = chrono::Utc::now().to_rfc3339();
        }
    }

    /// Check whether the token has a given tag.
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }

    /// Format a raw integer amount using the token's decimals.
    ///
    /// E.g. `1000000` with `decimals=6` produces `"1.000000"`.
    pub fn display_amount(&self, raw: u64) -> String {
        if self.decimals == 0 {
            return raw.to_string();
        }
        let divisor = 10u64.pow(self.decimals as u32);
        let whole = raw / divisor;
        let frac = raw % divisor;
        format!(
            "{}.{:0>width$}",
            whole,
            frac,
            width = self.decimals as usize
        )
    }

    /// Parse a display string back into a raw integer amount.
    pub fn parse_amount(&self, display: &str) -> Result<u64, TokenRegistryError> {
        if self.decimals == 0 {
            return display
                .parse::<u64>()
                .map_err(|e| TokenRegistryError::InvalidAmount(e.to_string()));
        }

        let parts: Vec<&str> = display.split('.').collect();
        match parts.len() {
            1 => {
                let whole: u64 = parts[0].parse().map_err(|e: std::num::ParseIntError| {
                    TokenRegistryError::InvalidAmount(e.to_string())
                })?;
                Ok(whole * 10u64.pow(self.decimals as u32))
            }
            2 => {
                let whole: u64 = parts[0].parse().map_err(|e: std::num::ParseIntError| {
                    TokenRegistryError::InvalidAmount(e.to_string())
                })?;
                let frac_str = parts[1];
                if frac_str.len() > self.decimals as usize {
                    return Err(TokenRegistryError::InvalidAmount(format!(
                        "too many decimal places: expected at most {}, got {}",
                        self.decimals,
                        frac_str.len()
                    )));
                }
                // Pad to the right so "1.5" with 6 decimals becomes 500000
                let padded = format!("{:0<width$}", frac_str, width = self.decimals as usize);
                let frac: u64 = padded.parse().map_err(|e: std::num::ParseIntError| {
                    TokenRegistryError::InvalidAmount(e.to_string())
                })?;
                Ok(whole * 10u64.pow(self.decimals as u32) + frac)
            }
            _ => Err(TokenRegistryError::InvalidAmount(
                "multiple decimal points".to_string(),
            )),
        }
    }
}

// ──────────────────────────── RegistryStats ──────────────────────────────

/// Summary statistics for the token registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryStats {
    pub total: usize,
    pub verified: usize,
    pub flagged: usize,
    pub unique_tags: usize,
}

// ──────────────────────────── TokenRegistry ──────────────────────────────

/// Persistent token metadata store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenRegistry {
    pub tokens: HashMap<String, TokenInfo>,
    pub aliases: HashMap<String, String>,
}

impl TokenRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            tokens: HashMap::new(),
            aliases: HashMap::new(),
        }
    }

    /// Register a new token. Fails if the address already exists.
    pub fn register(&mut self, token: TokenInfo) -> Result<(), TokenRegistryError> {
        if self.tokens.contains_key(&token.address) {
            return Err(TokenRegistryError::AlreadyExists(token.address.clone()));
        }
        self.aliases
            .insert(token.symbol.clone(), token.address.clone());
        self.tokens.insert(token.address.clone(), token);
        Ok(())
    }

    /// Update an existing token. Fails if the address is not registered.
    pub fn update(&mut self, address: &str, token: TokenInfo) -> Result<(), TokenRegistryError> {
        if !self.tokens.contains_key(address) {
            return Err(TokenRegistryError::NotFound(address.to_string()));
        }
        // Remove old alias if the symbol changed
        if let Some(old) = self.tokens.get(address) {
            self.aliases.remove(&old.symbol);
        }
        self.aliases
            .insert(token.symbol.clone(), token.address.clone());
        self.tokens.insert(address.to_string(), token);
        Ok(())
    }

    /// Remove a token by address. Returns the removed token info.
    pub fn remove(&mut self, address: &str) -> Result<TokenInfo, TokenRegistryError> {
        let token = self
            .tokens
            .remove(address)
            .ok_or_else(|| TokenRegistryError::NotFound(address.to_string()))?;
        self.aliases.remove(&token.symbol);
        Ok(token)
    }

    /// Get token info by address.
    pub fn get(&self, address: &str) -> Option<&TokenInfo> {
        self.tokens.get(address)
    }

    /// Look up a token by its symbol via the alias map.
    pub fn get_by_symbol(&self, symbol: &str) -> Option<&TokenInfo> {
        self.aliases
            .get(symbol)
            .and_then(|addr| self.tokens.get(addr))
    }

    /// Case-insensitive search across name, symbol, and address.
    pub fn search(&self, query: &str) -> Vec<&TokenInfo> {
        let q = query.to_lowercase();
        self.tokens
            .values()
            .filter(|t| {
                t.name.to_lowercase().contains(&q)
                    || t.symbol.to_lowercase().contains(&q)
                    || t.address.to_lowercase().contains(&q)
            })
            .collect()
    }

    /// List all verified tokens.
    pub fn list_verified(&self) -> Vec<&TokenInfo> {
        self.tokens.values().filter(|t| t.verified).collect()
    }

    /// List all tokens flagged as scams.
    pub fn list_scams(&self) -> Vec<&TokenInfo> {
        self.tokens.values().filter(|t| t.flagged_scam).collect()
    }

    /// List all tokens that have a given tag.
    pub fn list_by_tag(&self, tag: &str) -> Vec<&TokenInfo> {
        self.tokens.values().filter(|t| t.has_tag(tag)).collect()
    }

    /// Check whether an address is flagged as a scam.
    pub fn is_scam(&self, address: &str) -> bool {
        self.tokens.get(address).is_some_and(|t| t.flagged_scam)
    }

    /// Mark a token as verified. Fails if the address is not registered.
    pub fn verify_token(&mut self, address: &str) -> Result<(), TokenRegistryError> {
        let token = self
            .tokens
            .get_mut(address)
            .ok_or_else(|| TokenRegistryError::NotFound(address.to_string()))?;
        token.verified = true;
        token.updated_at = chrono::Utc::now().to_rfc3339();
        Ok(())
    }

    /// Flag a token as a scam. Fails if the address is not registered.
    pub fn flag_token(&mut self, address: &str, reason: &str) -> Result<(), TokenRegistryError> {
        let token = self
            .tokens
            .get_mut(address)
            .ok_or_else(|| TokenRegistryError::NotFound(address.to_string()))?;
        token.flag_scam(reason);
        Ok(())
    }

    /// Return summary statistics for the registry.
    pub fn stats(&self) -> RegistryStats {
        let mut all_tags: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for t in self.tokens.values() {
            for tag in &t.tags {
                all_tags.insert(tag.as_str());
            }
        }
        RegistryStats {
            total: self.tokens.len(),
            verified: self.tokens.values().filter(|t| t.verified).count(),
            flagged: self.tokens.values().filter(|t| t.flagged_scam).count(),
            unique_tags: all_tags.len(),
        }
    }

    /// Import tokens from CSV (address,name,symbol,decimals per line).
    /// Returns the number of tokens successfully imported.
    pub fn import_csv(&mut self, csv: &str) -> usize {
        let mut count = 0;
        for line in csv.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.split(',').collect();
            if fields.len() < 4 {
                continue;
            }
            let decimals: u8 = match fields[3].trim().parse() {
                Ok(d) => d,
                Err(_) => continue,
            };
            let token = TokenInfo::new(
                fields[0].trim(),
                fields[1].trim(),
                fields[2].trim(),
                decimals,
            );
            if self.register(token).is_ok() {
                count += 1;
            }
        }
        count
    }

    /// Export all tokens as CSV (address,name,symbol,decimals per line).
    pub fn export_csv(&self) -> String {
        let mut lines: Vec<String> = self
            .tokens
            .values()
            .map(|t| format!("{},{},{},{}", t.address, t.name, t.symbol, t.decimals))
            .collect();
        lines.sort();
        lines.join("\n")
    }

    /// Load from a JSON file.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, TokenRegistryError> {
        let data = std::fs::read_to_string(path)?;
        let registry: TokenRegistry = serde_json::from_str(&data)?;
        Ok(registry)
    }

    /// Save to a JSON file.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), TokenRegistryError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load from file if it exists, otherwise return a new default registry.
    pub fn load_or_default<P: AsRef<Path>>(path: P) -> Result<Self, TokenRegistryError> {
        let path = path.as_ref();
        if path.exists() {
            let data = std::fs::read_to_string(path)?;
            let registry: TokenRegistry = serde_json::from_str(&data)?;
            Ok(registry)
        } else {
            let registry = Self::new();
            registry.save(path)?;
            Ok(registry)
        }
    }
}

impl Default for TokenRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ──────────────────────────── Tests ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("token_reg_test_{}_{name}", std::process::id()))
    }

    fn sample_token(addr: &str, name: &str, symbol: &str) -> TokenInfo {
        TokenInfo::new(addr, name, symbol, 6)
    }

    #[test]
    fn test_register_and_get() {
        let mut reg = TokenRegistry::new();
        let token = sample_token("0xAAA", "Alpha", "ALP");
        reg.register(token).unwrap();
        let got = reg.get("0xAAA").unwrap();
        assert_eq!(got.name, "Alpha");
        assert_eq!(got.symbol, "ALP");
        assert_eq!(got.decimals, 6);
    }

    #[test]
    fn test_register_duplicate_rejected() {
        let mut reg = TokenRegistry::new();
        reg.register(sample_token("0xAAA", "Alpha", "ALP")).unwrap();
        let res = reg.register(sample_token("0xAAA", "Alpha2", "ALP2"));
        assert!(res.is_err());
        assert!(matches!(res, Err(TokenRegistryError::AlreadyExists(_))));
    }

    #[test]
    fn test_remove_token() {
        let mut reg = TokenRegistry::new();
        reg.register(sample_token("0xAAA", "Alpha", "ALP")).unwrap();
        let removed = reg.remove("0xAAA").unwrap();
        assert_eq!(removed.name, "Alpha");
        assert!(reg.get("0xAAA").is_none());
        assert!(reg.get_by_symbol("ALP").is_none());
    }

    #[test]
    fn test_remove_not_found() {
        let mut reg = TokenRegistry::new();
        let res = reg.remove("0xNONE");
        assert!(matches!(res, Err(TokenRegistryError::NotFound(_))));
    }

    #[test]
    fn test_get_by_symbol() {
        let mut reg = TokenRegistry::new();
        reg.register(sample_token("0xAAA", "Alpha", "ALP")).unwrap();
        let got = reg.get_by_symbol("ALP").unwrap();
        assert_eq!(got.address, "0xAAA");
    }

    #[test]
    fn test_search_by_name() {
        let mut reg = TokenRegistry::new();
        reg.register(sample_token("0xAAA", "Alpha", "ALP")).unwrap();
        reg.register(sample_token("0xBBB", "Beta", "BET")).unwrap();
        let results = reg.search("Alpha");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Alpha");
    }

    #[test]
    fn test_search_by_symbol() {
        let mut reg = TokenRegistry::new();
        reg.register(sample_token("0xAAA", "Alpha", "ALP")).unwrap();
        let results = reg.search("ALP");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_search_case_insensitive() {
        let mut reg = TokenRegistry::new();
        reg.register(sample_token("0xAAA", "Alpha", "ALP")).unwrap();
        let results = reg.search("alpha");
        assert_eq!(results.len(), 1);
        let results2 = reg.search("alp");
        assert_eq!(results2.len(), 1);
    }

    #[test]
    fn test_list_verified() {
        let mut reg = TokenRegistry::new();
        reg.register(sample_token("0xAAA", "Alpha", "ALP").verify())
            .unwrap();
        reg.register(sample_token("0xBBB", "Beta", "BET")).unwrap();
        let verified = reg.list_verified();
        assert_eq!(verified.len(), 1);
        assert_eq!(verified[0].address, "0xAAA");
    }

    #[test]
    fn test_list_scams() {
        let mut reg = TokenRegistry::new();
        let mut scam = sample_token("0xSCAM", "ScamCoin", "SCM");
        scam.flag_scam("rug pull");
        reg.register(scam).unwrap();
        reg.register(sample_token("0xBBB", "Beta", "BET")).unwrap();
        let scams = reg.list_scams();
        assert_eq!(scams.len(), 1);
        assert_eq!(scams[0].address, "0xSCAM");
    }

    #[test]
    fn test_flag_and_clear_scam() {
        let mut reg = TokenRegistry::new();
        reg.register(sample_token("0xAAA", "Alpha", "ALP")).unwrap();
        reg.flag_token("0xAAA", "suspicious activity").unwrap();
        assert!(reg.is_scam("0xAAA"));
        assert_eq!(
            reg.get("0xAAA").unwrap().scam_reason.as_deref(),
            Some("suspicious activity")
        );

        // Clear
        reg.tokens.get_mut("0xAAA").unwrap().clear_scam_flag();
        assert!(!reg.is_scam("0xAAA"));
        assert!(reg.get("0xAAA").unwrap().scam_reason.is_none());
    }

    #[test]
    fn test_verify_token() {
        let mut reg = TokenRegistry::new();
        reg.register(sample_token("0xAAA", "Alpha", "ALP")).unwrap();
        assert!(!reg.get("0xAAA").unwrap().verified);
        reg.verify_token("0xAAA").unwrap();
        assert!(reg.get("0xAAA").unwrap().verified);
    }

    #[test]
    fn test_tags() {
        let mut token = sample_token("0xAAA", "Alpha", "ALP");
        token.add_tag("defi");
        token.add_tag("stablecoin");
        token.add_tag("defi"); // duplicate — should not add
        assert!(token.has_tag("defi"));
        assert!(token.has_tag("stablecoin"));
        assert!(!token.has_tag("meme"));
        assert_eq!(token.tags.len(), 2);
    }

    #[test]
    fn test_list_by_tag() {
        let mut reg = TokenRegistry::new();
        let mut t1 = sample_token("0xAAA", "Alpha", "ALP");
        t1.add_tag("defi");
        reg.register(t1).unwrap();

        let mut t2 = sample_token("0xBBB", "Beta", "BET");
        t2.add_tag("meme");
        reg.register(t2).unwrap();

        let defi = reg.list_by_tag("defi");
        assert_eq!(defi.len(), 1);
        assert_eq!(defi[0].address, "0xAAA");
    }

    #[test]
    fn test_display_amount() {
        let token = sample_token("0xAAA", "Alpha", "ALP"); // decimals=6
        assert_eq!(token.display_amount(1_000_000), "1.000000");
        assert_eq!(token.display_amount(1_234_567), "1.234567");
        assert_eq!(token.display_amount(500_000), "0.500000");
        assert_eq!(token.display_amount(0), "0.000000");
    }

    #[test]
    fn test_display_amount_zero_decimals() {
        let token = TokenInfo::new("0xAAA", "NoDec", "ND", 0);
        assert_eq!(token.display_amount(42), "42");
        assert_eq!(token.display_amount(0), "0");
    }

    #[test]
    fn test_parse_amount() {
        let token = sample_token("0xAAA", "Alpha", "ALP"); // decimals=6
        assert_eq!(token.parse_amount("1.000000").unwrap(), 1_000_000);
        assert_eq!(token.parse_amount("1.234567").unwrap(), 1_234_567);
        assert_eq!(token.parse_amount("0.5").unwrap(), 500_000);
        assert_eq!(token.parse_amount("100").unwrap(), 100_000_000);
    }

    #[test]
    fn test_parse_amount_invalid() {
        let token = sample_token("0xAAA", "Alpha", "ALP");
        assert!(token.parse_amount("not_a_number").is_err());
        assert!(token.parse_amount("1.2.3").is_err());
        assert!(token.parse_amount("1.1234567890").is_err()); // too many decimals
    }

    #[test]
    fn test_import_csv() {
        let mut reg = TokenRegistry::new();
        let csv = "0xAAA,Alpha,ALP,6\n0xBBB,Beta,BET,8\n\ninvalid_line\n0xCCC,Gamma,GAM,18";
        let count = reg.import_csv(csv);
        assert_eq!(count, 3);
        assert!(reg.get("0xAAA").is_some());
        assert!(reg.get("0xBBB").is_some());
        assert!(reg.get("0xCCC").is_some());
        assert_eq!(reg.get("0xBBB").unwrap().decimals, 8);
    }

    #[test]
    fn test_export_csv() {
        let mut reg = TokenRegistry::new();
        reg.register(TokenInfo::new("0xAAA", "Alpha", "ALP", 6))
            .unwrap();
        reg.register(TokenInfo::new("0xBBB", "Beta", "BET", 8))
            .unwrap();
        let csv = reg.export_csv();
        assert!(csv.contains("0xAAA,Alpha,ALP,6"));
        assert!(csv.contains("0xBBB,Beta,BET,8"));
    }

    #[test]
    fn test_stats() {
        let mut reg = TokenRegistry::new();

        let mut t1 = sample_token("0xAAA", "Alpha", "ALP");
        t1.add_tag("defi");
        let t1 = t1.verify();
        reg.register(t1).unwrap();

        let mut t2 = sample_token("0xBBB", "Beta", "BET");
        t2.add_tag("meme");
        t2.flag_scam("rug pull");
        reg.register(t2).unwrap();

        let stats = reg.stats();
        assert_eq!(stats.total, 2);
        assert_eq!(stats.verified, 1);
        assert_eq!(stats.flagged, 1);
        assert_eq!(stats.unique_tags, 2);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path("roundtrip.json");
        let mut reg = TokenRegistry::new();
        reg.register(sample_token("0xAAA", "Alpha", "ALP")).unwrap();
        reg.register(sample_token("0xBBB", "Beta", "BET")).unwrap();
        reg.save(&path).unwrap();

        let loaded = TokenRegistry::load(&path).unwrap();
        assert_eq!(loaded.tokens.len(), 2);
        assert_eq!(loaded.get("0xAAA").unwrap().name, "Alpha");
        assert_eq!(loaded.get_by_symbol("BET").unwrap().address, "0xBBB");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_custom_fields() {
        let mut token = sample_token("0xAAA", "Alpha", "ALP");
        token
            .custom_fields
            .insert("coingecko_id".to_string(), "alpha-token".to_string());
        token
            .custom_fields
            .insert("chain".to_string(), "evaporchain".to_string());

        assert_eq!(
            token.custom_fields.get("coingecko_id").unwrap(),
            "alpha-token"
        );
        assert_eq!(token.custom_fields.get("chain").unwrap(), "evaporchain");
        assert_eq!(token.custom_fields.len(), 2);
    }

    #[test]
    fn test_update_token() {
        let mut reg = TokenRegistry::new();
        reg.register(sample_token("0xAAA", "Alpha", "ALP")).unwrap();

        let updated = TokenInfo::new("0xAAA", "AlphaV2", "ALP2", 8);
        reg.update("0xAAA", updated).unwrap();

        let got = reg.get("0xAAA").unwrap();
        assert_eq!(got.name, "AlphaV2");
        assert_eq!(got.symbol, "ALP2");
        assert_eq!(got.decimals, 8);

        // Old alias removed, new alias works
        assert!(reg.get_by_symbol("ALP").is_none());
        assert!(reg.get_by_symbol("ALP2").is_some());

        // Update non-existent fails
        let res = reg.update("0xNONE", sample_token("0xNONE", "X", "X"));
        assert!(matches!(res, Err(TokenRegistryError::NotFound(_))));
    }

    // ─── Additional coverage tests ────────────────────────────────────────────

    #[test]
    fn test_with_logo_and_website_covers_lines_77_86() {
        let token = sample_token("0xAAA", "Alpha", "ALP")
            .with_logo("https://example.com/logo.png")
            .with_website("https://example.com");
        assert_eq!(token.logo_url.as_deref(), Some("https://example.com/logo.png"));
        assert_eq!(token.website.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn test_parse_amount_zero_decimals_covers_lines_144_146() {
        let token = TokenInfo::new("0xAAA", "NoDec", "ND", 0);
        assert_eq!(token.parse_amount("42").unwrap(), 42);
        // parse error for zero-decimal token
        assert!(token.parse_amount("not_a_num").is_err());
    }

    #[test]
    fn test_parse_amount_bad_whole_part_covers_lines_159_160() {
        let token = sample_token("0xAAA", "Alpha", "ALP"); // decimals=6
        // "abc.5" → whole="abc" fails parse → InvalidAmount
        let err = token.parse_amount("abc.5").unwrap_err();
        assert!(matches!(err, TokenRegistryError::InvalidAmount(_)));
    }

    #[test]
    fn test_parse_amount_bad_frac_part_covers_lines_172_173() {
        let token = sample_token("0xAAA", "Alpha", "ALP"); // decimals=6
        // "1.abc" → padded="abc000" fails parse::<u64>() → InvalidAmount
        let err = token.parse_amount("1.abc").unwrap_err();
        assert!(matches!(err, TokenRegistryError::InvalidAmount(_)));
    }

    #[test]
    fn test_import_csv_bad_decimals_covers_line_345() {
        let mut reg = TokenRegistry::new();
        // Third line has non-numeric decimals → parse fails → continue
        let csv = "0xAAA,Alpha,ALP,6\n0xBBB,Bad,BAD,notanumber\n0xCCC,Gamma,GAM,18";
        let count = reg.import_csv(csv);
        assert_eq!(count, 2);
        assert!(reg.get("0xAAA").is_some());
        assert!(reg.get("0xBBB").is_none());
    }

    #[test]
    fn test_load_or_default_existing_file_covers_lines_386_390() {
        let path = test_path("load_or_default_existing.json");
        let mut reg = TokenRegistry::new();
        reg.register(sample_token("0xAAA", "Alpha", "ALP")).unwrap();
        reg.save(&path).unwrap();

        let loaded = TokenRegistry::load_or_default(&path).unwrap();
        assert_eq!(loaded.tokens.len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_nonexistent_covers_lines_393_395() {
        let path = test_path("load_or_default_new.json");
        // Ensure file doesn't exist
        let _ = std::fs::remove_file(&path);
        let reg = TokenRegistry::load_or_default(&path).unwrap();
        assert!(reg.tokens.is_empty());
        // Should have created the file
        assert!(path.exists());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_token_registry_default_covers_lines_401_403() {
        let reg = TokenRegistry::default();
        assert!(reg.tokens.is_empty());
    }
}
