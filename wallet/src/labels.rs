//! Address labeling and transaction annotation.
//!
//! Enrich wallet UX with human-readable context:
//! - Tag addresses with categories (exchange, defi, personal, contract, etc.)
//! - Annotate transactions with notes, tags, and categories
//! - Search and filter by labels
//! - Persistent JSON storage
//!
//! This is a local-only enrichment layer — labels never leave the device.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum LabelError {
    #[error("label not found: {0}")]
    NotFound(String),
    #[error("duplicate label for address: {0}")]
    Duplicate(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ──────────────────────────── Address Labels ─────────────────────────────

/// Predefined address categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AddressCategory {
    Personal,
    Exchange,
    Defi,
    Contract,
    Dao,
    Staking,
    Nft,
    Faucet,
    Unknown,
}

impl AddressCategory {
    /// All categories.
    pub fn all() -> &'static [AddressCategory] {
        &[
            Self::Personal, Self::Exchange, Self::Defi, Self::Contract,
            Self::Dao, Self::Staking, Self::Nft, Self::Faucet, Self::Unknown,
        ]
    }

    /// Parse from string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "personal" => Some(Self::Personal),
            "exchange" => Some(Self::Exchange),
            "defi" => Some(Self::Defi),
            "contract" => Some(Self::Contract),
            "dao" => Some(Self::Dao),
            "staking" => Some(Self::Staking),
            "nft" => Some(Self::Nft),
            "faucet" => Some(Self::Faucet),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }

    /// Display label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Personal => "personal",
            Self::Exchange => "exchange",
            Self::Defi => "defi",
            Self::Contract => "contract",
            Self::Dao => "dao",
            Self::Staking => "staking",
            Self::Nft => "nft",
            Self::Faucet => "faucet",
            Self::Unknown => "unknown",
        }
    }
}

/// A label for an address.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressLabel {
    /// The address (0x-prefixed, lowercase).
    pub address: String,
    /// Human-readable name.
    pub name: String,
    /// Category.
    pub category: AddressCategory,
    /// Custom tags (e.g., "high-value", "trusted").
    #[serde(default)]
    pub tags: Vec<String>,
    /// Optional note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// When this label was created.
    pub created_at: String,
    /// When this label was last updated.
    pub updated_at: String,
}

/// A note attached to a transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxAnnotation {
    /// Transaction hash.
    pub tx_hash: String,
    /// User note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Tags (e.g., "salary", "swap", "gas-refund").
    #[serde(default)]
    pub tags: Vec<String>,
    /// Category.
    #[serde(default)]
    pub category: Option<String>,
    /// When annotated.
    pub created_at: String,
}

// ──────────────────────────── LabelStore ─────────────────────────────────

/// Persistent store for address labels and tx annotations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelStore {
    pub address_labels: Vec<AddressLabel>,
    pub tx_annotations: Vec<TxAnnotation>,
    #[serde(skip)]
    addr_index: HashMap<String, usize>,
    #[serde(skip)]
    tx_index: HashMap<String, usize>,
}

impl LabelStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self {
            address_labels: Vec::new(),
            tx_annotations: Vec::new(),
            addr_index: HashMap::new(),
            tx_index: HashMap::new(),
        }
    }

    /// Rebuild internal indexes.
    fn rebuild_indexes(&mut self) {
        self.addr_index.clear();
        for (i, l) in self.address_labels.iter().enumerate() {
            self.addr_index.insert(l.address.to_lowercase(), i);
        }
        self.tx_index.clear();
        for (i, a) in self.tx_annotations.iter().enumerate() {
            self.tx_index.insert(a.tx_hash.clone(), i);
        }
    }

    /// Load from a JSON file.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, LabelError> {
        let data = std::fs::read_to_string(path)?;
        let mut store: LabelStore = serde_json::from_str(&data)?;
        store.rebuild_indexes();
        Ok(store)
    }

    /// Save to a JSON file.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), LabelError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    // ── Address Labels ──────────────────────────────────────────────────

    /// Add a label for an address.
    pub fn label_address(
        &mut self,
        address: &str,
        name: &str,
        category: AddressCategory,
        tags: Vec<String>,
        note: Option<&str>,
    ) -> Result<(), LabelError> {
        let addr = address.to_lowercase();
        if self.addr_index.contains_key(&addr) {
            return Err(LabelError::Duplicate(address.to_string()));
        }

        let now = chrono::Utc::now().to_rfc3339();
        let label = AddressLabel {
            address: addr.clone(),
            name: name.to_string(),
            category,
            tags,
            note: note.map(|s| s.to_string()),
            created_at: now.clone(),
            updated_at: now,
        };

        let idx = self.address_labels.len();
        self.address_labels.push(label);
        self.addr_index.insert(addr, idx);
        Ok(())
    }

    /// Get label for an address.
    pub fn get_address_label(&self, address: &str) -> Option<&AddressLabel> {
        let addr = address.to_lowercase();
        self.addr_index.get(&addr).map(|&i| &self.address_labels[i])
    }

    /// Update an address label.
    pub fn update_address_label(
        &mut self,
        address: &str,
        name: Option<&str>,
        category: Option<AddressCategory>,
        tags: Option<Vec<String>>,
        note: Option<Option<&str>>,
    ) -> Result<(), LabelError> {
        let addr = address.to_lowercase();
        let idx = *self
            .addr_index
            .get(&addr)
            .ok_or_else(|| LabelError::NotFound(address.to_string()))?;

        if let Some(n) = name {
            self.address_labels[idx].name = n.to_string();
        }
        if let Some(c) = category {
            self.address_labels[idx].category = c;
        }
        if let Some(t) = tags {
            self.address_labels[idx].tags = t;
        }
        if let Some(n) = note {
            self.address_labels[idx].note = n.map(|s| s.to_string());
        }
        self.address_labels[idx].updated_at = chrono::Utc::now().to_rfc3339();
        Ok(())
    }

    /// Remove an address label.
    pub fn remove_address_label(&mut self, address: &str) -> Result<(), LabelError> {
        let addr = address.to_lowercase();
        if !self.addr_index.contains_key(&addr) {
            return Err(LabelError::NotFound(address.to_string()));
        }
        self.address_labels.retain(|l| l.address.to_lowercase() != addr);
        self.rebuild_indexes();
        Ok(())
    }

    /// List all address labels.
    pub fn list_address_labels(&self) -> &[AddressLabel] {
        &self.address_labels
    }

    /// Filter address labels by category.
    pub fn filter_by_category(&self, category: AddressCategory) -> Vec<&AddressLabel> {
        self.address_labels
            .iter()
            .filter(|l| l.category == category)
            .collect()
    }

    /// Filter address labels by tag.
    pub fn filter_by_tag(&self, tag: &str) -> Vec<&AddressLabel> {
        let tag_lower = tag.to_lowercase();
        self.address_labels
            .iter()
            .filter(|l| l.tags.iter().any(|t| t.to_lowercase() == tag_lower))
            .collect()
    }

    /// Search address labels by name (substring match).
    pub fn search_addresses(&self, query: &str) -> Vec<&AddressLabel> {
        let q = query.to_lowercase();
        self.address_labels
            .iter()
            .filter(|l| {
                l.name.to_lowercase().contains(&q)
                    || l.address.to_lowercase().contains(&q)
                    || l.tags.iter().any(|t| t.to_lowercase().contains(&q))
            })
            .collect()
    }

    /// Resolve an address to its label name (if labeled).
    pub fn resolve_name(&self, address: &str) -> Option<&str> {
        self.get_address_label(address).map(|l| l.name.as_str())
    }

    // ── Transaction Annotations ─────────────────────────────────────────

    /// Annotate a transaction.
    pub fn annotate_tx(
        &mut self,
        tx_hash: &str,
        note: Option<&str>,
        tags: Vec<String>,
        category: Option<&str>,
    ) -> Result<(), LabelError> {
        let now = chrono::Utc::now().to_rfc3339();

        // Update if exists, else create
        if let Some(&idx) = self.tx_index.get(tx_hash) {
            if let Some(n) = note {
                self.tx_annotations[idx].note = Some(n.to_string());
            }
            if !tags.is_empty() {
                self.tx_annotations[idx].tags = tags;
            }
            if let Some(c) = category {
                self.tx_annotations[idx].category = Some(c.to_string());
            }
        } else {
            let ann = TxAnnotation {
                tx_hash: tx_hash.to_string(),
                note: note.map(|s| s.to_string()),
                tags,
                category: category.map(|s| s.to_string()),
                created_at: now,
            };
            let idx = self.tx_annotations.len();
            self.tx_annotations.push(ann);
            self.tx_index.insert(tx_hash.to_string(), idx);
        }
        Ok(())
    }

    /// Get annotation for a transaction.
    pub fn get_tx_annotation(&self, tx_hash: &str) -> Option<&TxAnnotation> {
        self.tx_index.get(tx_hash).map(|&i| &self.tx_annotations[i])
    }

    /// Remove a transaction annotation.
    pub fn remove_tx_annotation(&mut self, tx_hash: &str) -> Result<(), LabelError> {
        if !self.tx_index.contains_key(tx_hash) {
            return Err(LabelError::NotFound(tx_hash.to_string()));
        }
        self.tx_annotations.retain(|a| a.tx_hash != tx_hash);
        self.rebuild_indexes();
        Ok(())
    }

    /// List all tx annotations.
    pub fn list_tx_annotations(&self) -> &[TxAnnotation] {
        &self.tx_annotations
    }

    /// Filter tx annotations by tag.
    pub fn filter_tx_by_tag(&self, tag: &str) -> Vec<&TxAnnotation> {
        let tag_lower = tag.to_lowercase();
        self.tx_annotations
            .iter()
            .filter(|a| a.tags.iter().any(|t| t.to_lowercase() == tag_lower))
            .collect()
    }

    /// Filter tx annotations by category.
    pub fn filter_tx_by_category(&self, category: &str) -> Vec<&TxAnnotation> {
        let cat = category.to_lowercase();
        self.tx_annotations
            .iter()
            .filter(|a| a.category.as_deref().map(|c| c.to_lowercase()) == Some(cat.clone()))
            .collect()
    }

    /// Count labels.
    pub fn address_count(&self) -> usize {
        self.address_labels.len()
    }

    /// Count annotations.
    pub fn annotation_count(&self) -> usize {
        self.tx_annotations.len()
    }
}

impl Default for LabelStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Default path for label store.
pub fn default_labels_path() -> std::path::PathBuf {
    crate::config::default_data_dir().join("labels.json")
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store() -> LabelStore {
        let mut store = LabelStore::new();
        store
            .label_address(
                "0xabc123",
                "Binance Hot",
                AddressCategory::Exchange,
                vec!["cex".into(), "high-volume".into()],
                Some("Main deposit address"),
            )
            .unwrap();
        store
            .label_address(
                "0xdef456",
                "My Staking",
                AddressCategory::Staking,
                vec!["validator".into()],
                None,
            )
            .unwrap();
        store
    }

    #[test]
    fn test_label_address() {
        let store = make_store();
        assert_eq!(store.address_count(), 2);
        let label = store.get_address_label("0xabc123").unwrap();
        assert_eq!(label.name, "Binance Hot");
        assert_eq!(label.category, AddressCategory::Exchange);
        assert_eq!(label.tags.len(), 2);
    }

    #[test]
    fn test_label_duplicate_rejected() {
        let mut store = make_store();
        let err = store
            .label_address("0xabc123", "Dup", AddressCategory::Unknown, vec![], None)
            .unwrap_err();
        assert!(matches!(err, LabelError::Duplicate(_)));
    }

    #[test]
    fn test_label_case_insensitive_lookup() {
        let store = make_store();
        assert!(store.get_address_label("0xABC123").is_some());
        assert!(store.get_address_label("0xabc123").is_some());
    }

    #[test]
    fn test_update_address_label() {
        let mut store = make_store();
        store
            .update_address_label(
                "0xabc123",
                Some("Binance Cold"),
                Some(AddressCategory::Personal),
                None,
                Some(Some("Updated")),
            )
            .unwrap();
        let label = store.get_address_label("0xabc123").unwrap();
        assert_eq!(label.name, "Binance Cold");
        assert_eq!(label.category, AddressCategory::Personal);
        assert_eq!(label.note.as_deref(), Some("Updated"));
    }

    #[test]
    fn test_update_not_found() {
        let mut store = LabelStore::new();
        let err = store
            .update_address_label("0xnope", None, None, None, None)
            .unwrap_err();
        assert!(matches!(err, LabelError::NotFound(_)));
    }

    #[test]
    fn test_remove_address_label() {
        let mut store = make_store();
        store.remove_address_label("0xabc123").unwrap();
        assert_eq!(store.address_count(), 1);
        assert!(store.get_address_label("0xabc123").is_none());
    }

    #[test]
    fn test_remove_not_found() {
        let mut store = LabelStore::new();
        let err = store.remove_address_label("0xnope").unwrap_err();
        assert!(matches!(err, LabelError::NotFound(_)));
    }

    #[test]
    fn test_filter_by_category() {
        let store = make_store();
        let exchanges = store.filter_by_category(AddressCategory::Exchange);
        assert_eq!(exchanges.len(), 1);
        assert_eq!(exchanges[0].name, "Binance Hot");
    }

    #[test]
    fn test_filter_by_tag() {
        let store = make_store();
        let results = store.filter_by_tag("cex");
        assert_eq!(results.len(), 1);
        let results = store.filter_by_tag("validator");
        assert_eq!(results.len(), 1);
        let results = store.filter_by_tag("nonexistent");
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_search_addresses() {
        let store = make_store();
        let results = store.search_addresses("binance");
        assert_eq!(results.len(), 1);
        let results = store.search_addresses("abc");
        assert_eq!(results.len(), 1);
        let results = store.search_addresses("staking");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_resolve_name() {
        let store = make_store();
        assert_eq!(store.resolve_name("0xabc123"), Some("Binance Hot"));
        assert!(store.resolve_name("0xunknown").is_none());
    }

    #[test]
    fn test_annotate_tx() {
        let mut store = LabelStore::new();
        store
            .annotate_tx("0xhash1", Some("Salary payment"), vec!["salary".into()], Some("income"))
            .unwrap();
        let ann = store.get_tx_annotation("0xhash1").unwrap();
        assert_eq!(ann.note.as_deref(), Some("Salary payment"));
        assert_eq!(ann.tags, vec!["salary"]);
        assert_eq!(ann.category.as_deref(), Some("income"));
    }

    #[test]
    fn test_annotate_tx_update() {
        let mut store = LabelStore::new();
        store
            .annotate_tx("0xhash1", Some("First note"), vec!["tag1".into()], None)
            .unwrap();
        store
            .annotate_tx("0xhash1", Some("Updated note"), vec!["tag2".into()], Some("expense"))
            .unwrap();

        // Should update, not duplicate
        assert_eq!(store.annotation_count(), 1);
        let ann = store.get_tx_annotation("0xhash1").unwrap();
        assert_eq!(ann.note.as_deref(), Some("Updated note"));
        assert_eq!(ann.tags, vec!["tag2"]);
    }

    #[test]
    fn test_remove_tx_annotation() {
        let mut store = LabelStore::new();
        store.annotate_tx("0xhash1", Some("test"), vec![], None).unwrap();
        store.remove_tx_annotation("0xhash1").unwrap();
        assert_eq!(store.annotation_count(), 0);
    }

    #[test]
    fn test_remove_tx_annotation_not_found() {
        let mut store = LabelStore::new();
        let err = store.remove_tx_annotation("0xnope").unwrap_err();
        assert!(matches!(err, LabelError::NotFound(_)));
    }

    #[test]
    fn test_filter_tx_by_tag() {
        let mut store = LabelStore::new();
        store.annotate_tx("0x1", None, vec!["swap".into()], None).unwrap();
        store.annotate_tx("0x2", None, vec!["salary".into()], None).unwrap();
        store.annotate_tx("0x3", None, vec!["swap".into(), "defi".into()], None).unwrap();

        let swaps = store.filter_tx_by_tag("swap");
        assert_eq!(swaps.len(), 2);
    }

    #[test]
    fn test_filter_tx_by_category() {
        let mut store = LabelStore::new();
        store.annotate_tx("0x1", None, vec![], Some("income")).unwrap();
        store.annotate_tx("0x2", None, vec![], Some("expense")).unwrap();
        store.annotate_tx("0x3", None, vec![], Some("income")).unwrap();

        let income = store.filter_tx_by_category("income");
        assert_eq!(income.len(), 2);
    }

    #[test]
    fn test_address_category_all() {
        assert_eq!(AddressCategory::all().len(), 9);
    }

    #[test]
    fn test_address_category_from_str() {
        assert_eq!(AddressCategory::from_str("exchange"), Some(AddressCategory::Exchange));
        assert_eq!(AddressCategory::from_str("DEFI"), Some(AddressCategory::Defi));
        assert_eq!(AddressCategory::from_str("invalid"), None);
    }

    #[test]
    fn test_address_category_label() {
        assert_eq!(AddressCategory::Exchange.label(), "exchange");
        assert_eq!(AddressCategory::Personal.label(), "personal");
    }

    #[test]
    fn test_json_roundtrip() {
        let mut store = make_store();
        store.annotate_tx("0xhash1", Some("test"), vec!["tag".into()], None).unwrap();

        let json = serde_json::to_string_pretty(&store).unwrap();
        let mut loaded: LabelStore = serde_json::from_str(&json).unwrap();
        loaded.rebuild_indexes();

        assert_eq!(loaded.address_count(), 2);
        assert_eq!(loaded.annotation_count(), 1);
        assert!(loaded.get_address_label("0xabc123").is_some());
        assert!(loaded.get_tx_annotation("0xhash1").is_some());
    }

    #[test]
    fn test_file_save_and_load() {
        let dir = std::env::temp_dir().join("evaporchain_labels_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("labels.json");

        let store = make_store();
        store.save(&path).unwrap();

        let loaded = LabelStore::load(&path).unwrap();
        assert_eq!(loaded.address_count(), 2);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_address_label_serializable() {
        let label = AddressLabel {
            address: "0xabc".to_string(),
            name: "Test".to_string(),
            category: AddressCategory::Exchange,
            tags: vec!["tag1".into()],
            note: Some("note".into()),
            created_at: "2026-01-01".into(),
            updated_at: "2026-01-01".into(),
        };
        let json = serde_json::to_string(&label).unwrap();
        assert!(json.contains("\"category\":\"exchange\""));
    }

    #[test]
    fn test_tx_annotation_serializable() {
        let ann = TxAnnotation {
            tx_hash: "0xhash".into(),
            note: Some("test".into()),
            tags: vec!["t1".into()],
            category: Some("income".into()),
            created_at: "2026-01-01".into(),
        };
        let json = serde_json::to_string(&ann).unwrap();
        assert!(json.contains("\"tx_hash\":\"0xhash\""));
    }
}
