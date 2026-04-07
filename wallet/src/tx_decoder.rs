//! Human-readable transaction decoder for the EvaporChain wallet.
//!
//! Decodes raw transaction selectors and parameters into human-friendly
//! summaries. Maintains a registry of known method signatures and contract
//! addresses, with an LRU-style decode cache. Persistent JSON storage.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum TxDecoderError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid selector: {0}")]
    InvalidSelector(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ──────────────────────────── Enums ──────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TxCategory {
    Transfer,
    ContractCall,
    ObjectCreation,
    ObjectRefresh,
    NftMint,
    NftTransfer,
    TokenDeploy,
    TokenTransfer,
    Staking,
    Governance,
    Bridge,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParamType {
    Address,
    Uint,
    Int,
    String,
    Bool,
    Bytes,
    Array,
}

// ──────────────────────────── MethodSignature ─────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodSignature {
    pub selector: String,
    pub name: String,
    pub params: Vec<(String, ParamType)>,
    pub category: TxCategory,
}

impl MethodSignature {
    pub fn new(selector: &str, name: &str, category: TxCategory) -> Self {
        Self {
            selector: selector.to_string(),
            name: name.to_string(),
            params: Vec::new(),
            category,
        }
    }

    pub fn add_param(mut self, name: &str, param_type: ParamType) -> Self {
        self.params.push((name.to_string(), param_type));
        self
    }

    /// Format like "transfer(address to, uint256 amount)".
    pub fn display(&self) -> String {
        let params: Vec<String> = self
            .params
            .iter()
            .map(|(name, pt)| {
                let type_str = match pt {
                    ParamType::Address => "address",
                    ParamType::Uint => "uint256",
                    ParamType::Int => "int256",
                    ParamType::String => "string",
                    ParamType::Bool => "bool",
                    ParamType::Bytes => "bytes",
                    ParamType::Array => "array",
                };
                format!("{} {}", type_str, name)
            })
            .collect();
        format!("{}({})", self.name, params.join(", "))
    }
}

// ──────────────────────────── DecodedParam ────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecodedParam {
    pub name: String,
    pub param_type: ParamType,
    pub value: String,
}

impl DecodedParam {
    pub fn new(name: &str, param_type: ParamType, value: &str) -> Self {
        Self {
            name: name.to_string(),
            param_type,
            value: value.to_string(),
        }
    }
}

// ──────────────────────────── DecodedTx ──────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecodedTx {
    pub tx_hash: String,
    pub category: TxCategory,
    pub method_name: String,
    pub params: Vec<DecodedParam>,
    pub summary: String,
    pub from_address: String,
    pub to_address: Option<String>,
    pub value: u64,
    pub fee: u64,
    pub decoded_at: String,
    pub raw_data: Option<String>,
}

impl DecodedTx {
    pub fn new(tx_hash: &str, category: TxCategory, method_name: &str) -> Self {
        Self {
            tx_hash: tx_hash.to_string(),
            category,
            method_name: method_name.to_string(),
            params: Vec::new(),
            summary: String::new(),
            from_address: String::new(),
            to_address: None,
            value: 0,
            fee: 0,
            decoded_at: chrono::Utc::now().to_rfc3339(),
            raw_data: None,
        }
    }

    pub fn with_param(mut self, param: DecodedParam) -> Self {
        self.params.push(param);
        self
    }

    pub fn with_summary(mut self, summary: &str) -> Self {
        self.summary = summary.to_string();
        self
    }

    pub fn with_addresses(mut self, from: &str, to: Option<&str>) -> Self {
        self.from_address = from.to_string();
        self.to_address = to.map(|s| s.to_string());
        self
    }

    pub fn with_value(mut self, value: u64) -> Self {
        self.value = value;
        self
    }

    pub fn with_fee(mut self, fee: u64) -> Self {
        self.fee = fee;
        self
    }

    /// Multi-line formatted display.
    pub fn display(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!("Category: {:?}", self.category));
        lines.push(format!("Method:   {}", self.method_name));
        lines.push(format!("From:     {}", self.from_address));
        if let Some(ref to) = self.to_address {
            lines.push(format!("To:       {}", to));
        }
        lines.push(format!("Value:    {}", self.value));
        lines.push(format!("Fee:      {}", self.fee));
        if !self.params.is_empty() {
            lines.push("Params:".to_string());
            for p in &self.params {
                lines.push(format!("  {} ({:?}): {}", p.name, p.param_type, p.value));
            }
        }
        if !self.summary.is_empty() {
            lines.push(format!("Summary:  {}", self.summary));
        }
        lines.join("\n")
    }
}

// ──────────────────────────── DecoderStats ───────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecoderStats {
    pub total_methods: usize,
    pub total_contracts: usize,
    pub cached_decodings: usize,
}

// ──────────────────────────── TxDecoder ──────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxDecoder {
    pub signatures: HashMap<String, MethodSignature>,
    pub known_contracts: HashMap<String, String>,
    pub decoded_cache: HashMap<String, DecodedTx>,
    pub max_cache: usize,
}

impl Default for TxDecoder {
    fn default() -> Self {
        Self {
            signatures: HashMap::new(),
            known_contracts: HashMap::new(),
            decoded_cache: HashMap::new(),
            max_cache: 1000,
        }
    }
}

impl TxDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a method signature, keyed by its selector.
    pub fn register_method(&mut self, sig: MethodSignature) {
        self.signatures.insert(sig.selector.clone(), sig);
    }

    /// Register a known contract address → name mapping.
    pub fn register_contract(&mut self, address: &str, name: &str) {
        self.known_contracts
            .insert(address.to_string(), name.to_string());
    }

    /// Look up a contract name by address.
    pub fn get_contract_name(&self, address: &str) -> Option<&str> {
        self.known_contracts.get(address).map(|s| s.as_str())
    }

    /// Decode a transaction from its selector and raw parameters.
    pub fn decode(
        &mut self,
        tx_hash: &str,
        selector: &str,
        from: &str,
        to: Option<&str>,
        value: u64,
        fee: u64,
        raw_params: &[(&str, &str)],
    ) -> DecodedTx {
        // Prune cache if at capacity
        if self.decoded_cache.len() >= self.max_cache {
            self.decoded_cache.clear();
        }

        let (category, method_name, decoded_params) =
            if let Some(sig) = self.signatures.get(selector) {
                let params: Vec<DecodedParam> = raw_params
                    .iter()
                    .enumerate()
                    .map(|(i, (_raw_name, raw_value))| {
                        if let Some((name, ptype)) = sig.params.get(i) {
                            DecodedParam::new(name, ptype.clone(), raw_value)
                        } else {
                            DecodedParam::new(_raw_name, ParamType::String, raw_value)
                        }
                    })
                    .collect();
                (sig.category.clone(), sig.name.clone(), params)
            } else {
                let params: Vec<DecodedParam> = raw_params
                    .iter()
                    .map(|(name, val)| DecodedParam::new(name, ParamType::String, val))
                    .collect();
                (TxCategory::Unknown, "unknown_method".to_string(), params)
            };

        let summary = self.generate_summary(&method_name, value, from, to);

        let tx = DecodedTx {
            tx_hash: tx_hash.to_string(),
            category,
            method_name,
            params: decoded_params,
            summary,
            from_address: from.to_string(),
            to_address: to.map(|s| s.to_string()),
            value,
            fee,
            decoded_at: chrono::Utc::now().to_rfc3339(),
            raw_data: None,
        };

        self.decoded_cache.insert(tx_hash.to_string(), tx.clone());
        tx
    }

    /// Get a previously decoded transaction from the cache.
    pub fn get_cached(&self, tx_hash: &str) -> Option<&DecodedTx> {
        self.decoded_cache.get(tx_hash)
    }

    /// Clear the decode cache; returns the number of entries removed.
    pub fn clear_cache(&mut self) -> usize {
        let count = self.decoded_cache.len();
        self.decoded_cache.clear();
        count
    }

    /// Register common EvaporChain method signatures.
    pub fn register_defaults(&mut self) {
        let defaults = vec![
            MethodSignature::new("0xa9059cbb", "transfer", TxCategory::Transfer)
                .add_param("to", ParamType::Address)
                .add_param("amount", ParamType::Uint),
            MethodSignature::new("0x01000001", "create_object", TxCategory::ObjectCreation)
                .add_param("object_type", ParamType::String)
                .add_param("data", ParamType::Bytes),
            MethodSignature::new("0x01000002", "refresh_object", TxCategory::ObjectRefresh)
                .add_param("object_id", ParamType::String),
            MethodSignature::new("0x02000001", "deploy_contract", TxCategory::ContractCall)
                .add_param("code", ParamType::Bytes)
                .add_param("init_params", ParamType::Bytes),
            MethodSignature::new("0x02000002", "call_contract", TxCategory::ContractCall)
                .add_param("contract", ParamType::Address)
                .add_param("method", ParamType::String)
                .add_param("args", ParamType::Bytes),
            MethodSignature::new("0x03000001", "mint_nft", TxCategory::NftMint)
                .add_param("collection", ParamType::Address)
                .add_param("metadata", ParamType::String),
            MethodSignature::new("0x03000002", "transfer_nft", TxCategory::NftTransfer)
                .add_param("to", ParamType::Address)
                .add_param("token_id", ParamType::Uint),
            MethodSignature::new("0x04000001", "deploy_token", TxCategory::TokenDeploy)
                .add_param("name", ParamType::String)
                .add_param("symbol", ParamType::String)
                .add_param("supply", ParamType::Uint),
            MethodSignature::new("0x04000002", "transfer_token", TxCategory::TokenTransfer)
                .add_param("token", ParamType::Address)
                .add_param("to", ParamType::Address)
                .add_param("amount", ParamType::Uint),
            MethodSignature::new("0x05000001", "stake", TxCategory::Staking)
                .add_param("validator", ParamType::Address)
                .add_param("amount", ParamType::Uint),
            MethodSignature::new("0x05000002", "unstake", TxCategory::Staking)
                .add_param("validator", ParamType::Address)
                .add_param("amount", ParamType::Uint),
            MethodSignature::new("0x06000001", "vote", TxCategory::Governance)
                .add_param("proposal_id", ParamType::Uint)
                .add_param("vote", ParamType::Bool),
        ];
        for sig in defaults {
            self.register_method(sig);
        }
    }

    /// List all registered method signatures.
    pub fn list_methods(&self) -> Vec<&MethodSignature> {
        self.signatures.values().collect()
    }

    /// List all known contracts as (address, name) pairs.
    pub fn list_contracts(&self) -> Vec<(&str, &str)> {
        self.known_contracts
            .iter()
            .map(|(a, n)| (a.as_str(), n.as_str()))
            .collect()
    }

    /// Quick one-liner summary without creating a full DecodedTx.
    pub fn decode_summary(
        &self,
        selector: &str,
        value: u64,
        from: &str,
        to: Option<&str>,
    ) -> String {
        let method_name = self
            .signatures
            .get(selector)
            .map(|s| s.name.as_str())
            .unwrap_or("unknown_method");
        self.generate_summary(method_name, value, from, to)
    }

    /// Return decoder statistics.
    pub fn stats(&self) -> DecoderStats {
        DecoderStats {
            total_methods: self.signatures.len(),
            total_contracts: self.known_contracts.len(),
            cached_decodings: self.decoded_cache.len(),
        }
    }

    // ── Persistence ───────────────────────────────────────────────

    pub fn save(&self, path: &Path) -> Result<(), TxDecoderError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, TxDecoderError> {
        let data = std::fs::read_to_string(path)?;
        let decoder: Self = serde_json::from_str(&data)?;
        Ok(decoder)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }

    // ── Internal helpers ──────────────────────────────────────────

    fn generate_summary(&self, method_name: &str, value: u64, from: &str, to: Option<&str>) -> String {
        let from_short = if from.len() > 8 {
            format!("{}...", &from[..8])
        } else {
            from.to_string()
        };
        let to_part = match to {
            Some(addr) => {
                let short = if addr.len() > 8 {
                    format!("{}...", &addr[..8])
                } else {
                    addr.to_string()
                };
                format!(" to {}", short)
            }
            None => String::new(),
        };
        format!(
            "{} {} from {}{}",
            method_name.replace('_', " "),
            value,
            from_short,
            to_part
        )
    }
}

// ──────────────────────────── Tests ──────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("tx_decoder_test_{}_{}", name, std::process::id()))
    }

    #[test]
    fn test_register_method() {
        let mut dec = TxDecoder::new();
        let sig = MethodSignature::new("0xaa", "foo", TxCategory::Transfer);
        dec.register_method(sig);
        assert!(dec.signatures.contains_key("0xaa"));
    }

    #[test]
    fn test_method_display() {
        let sig = MethodSignature::new("0xa9059cbb", "transfer", TxCategory::Transfer)
            .add_param("to", ParamType::Address)
            .add_param("amount", ParamType::Uint);
        assert_eq!(sig.display(), "transfer(address to, uint256 amount)");
    }

    #[test]
    fn test_register_contract() {
        let mut dec = TxDecoder::new();
        dec.register_contract("evap1abc", "MyToken");
        assert_eq!(
            dec.known_contracts.get("evap1abc").unwrap(),
            "MyToken"
        );
    }

    #[test]
    fn test_get_contract_name() {
        let mut dec = TxDecoder::new();
        dec.register_contract("evap1abc", "MyToken");
        assert_eq!(dec.get_contract_name("evap1abc"), Some("MyToken"));
        assert_eq!(dec.get_contract_name("evap1xyz"), None);
    }

    #[test]
    fn test_decode_known_method() {
        let mut dec = TxDecoder::new();
        dec.register_method(
            MethodSignature::new("0xaa", "transfer", TxCategory::Transfer)
                .add_param("to", ParamType::Address),
        );
        let tx = dec.decode("hash1", "0xaa", "evap1from", Some("evap1to"), 100, 5, &[("to", "evap1to")]);
        assert_eq!(tx.category, TxCategory::Transfer);
        assert_eq!(tx.method_name, "transfer");
        assert_eq!(tx.params.len(), 1);
        assert_eq!(tx.params[0].name, "to");
        assert_eq!(tx.params[0].value, "evap1to");
    }

    #[test]
    fn test_decode_unknown_method() {
        let mut dec = TxDecoder::new();
        let tx = dec.decode("hash2", "0xdeadbeef", "evap1from", None, 0, 1, &[]);
        assert_eq!(tx.category, TxCategory::Unknown);
        assert_eq!(tx.method_name, "unknown_method");
    }

    #[test]
    fn test_decode_with_params() {
        let mut dec = TxDecoder::new();
        dec.register_method(
            MethodSignature::new("0xbb", "stake", TxCategory::Staking)
                .add_param("validator", ParamType::Address)
                .add_param("amount", ParamType::Uint),
        );
        let tx = dec.decode(
            "hash3", "0xbb", "evap1from", None, 500, 2,
            &[("validator", "evap1val"), ("amount", "500")],
        );
        assert_eq!(tx.params.len(), 2);
        assert_eq!(tx.params[0].name, "validator");
        assert_eq!(tx.params[0].param_type, ParamType::Address);
        assert_eq!(tx.params[1].name, "amount");
        assert_eq!(tx.params[1].param_type, ParamType::Uint);
        assert_eq!(tx.params[1].value, "500");
    }

    #[test]
    fn test_decode_caches_result() {
        let mut dec = TxDecoder::new();
        dec.register_method(MethodSignature::new("0xcc", "foo", TxCategory::Transfer));
        dec.decode("hash4", "0xcc", "evap1a", None, 0, 0, &[]);
        assert!(dec.decoded_cache.contains_key("hash4"));
    }

    #[test]
    fn test_get_cached() {
        let mut dec = TxDecoder::new();
        dec.register_method(MethodSignature::new("0xdd", "bar", TxCategory::Transfer));
        dec.decode("hash5", "0xdd", "evap1a", None, 0, 0, &[]);
        let cached = dec.get_cached("hash5");
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().method_name, "bar");
        assert!(dec.get_cached("nonexistent").is_none());
    }

    #[test]
    fn test_clear_cache() {
        let mut dec = TxDecoder::new();
        dec.register_method(MethodSignature::new("0xee", "baz", TxCategory::Transfer));
        dec.decode("h1", "0xee", "a", None, 0, 0, &[]);
        dec.decode("h2", "0xee", "a", None, 0, 0, &[]);
        let cleared = dec.clear_cache();
        assert_eq!(cleared, 2);
        assert!(dec.decoded_cache.is_empty());
    }

    #[test]
    fn test_register_defaults() {
        let mut dec = TxDecoder::new();
        dec.register_defaults();
        assert!(dec.signatures.contains_key("0xa9059cbb"));
        assert!(dec.signatures.contains_key("0x01000001"));
        assert!(dec.signatures.contains_key("0x01000002"));
        assert!(dec.signatures.contains_key("0x02000001"));
        assert!(dec.signatures.contains_key("0x02000002"));
        assert!(dec.signatures.contains_key("0x03000001"));
        assert!(dec.signatures.contains_key("0x03000002"));
        assert!(dec.signatures.contains_key("0x04000001"));
        assert!(dec.signatures.contains_key("0x04000002"));
        assert!(dec.signatures.contains_key("0x05000001"));
        assert!(dec.signatures.contains_key("0x05000002"));
        assert!(dec.signatures.contains_key("0x06000001"));
        assert_eq!(dec.signatures.len(), 12);
    }

    #[test]
    fn test_decode_transfer() {
        let mut dec = TxDecoder::new();
        dec.register_defaults();
        let tx = dec.decode(
            "txhash_transfer", "0xa9059cbb", "evap1sender", Some("evap1receiver"),
            1000, 10, &[("to", "evap1receiver"), ("amount", "1000")],
        );
        assert_eq!(tx.category, TxCategory::Transfer);
        assert_eq!(tx.method_name, "transfer");
        assert_eq!(tx.value, 1000);
        assert_eq!(tx.fee, 10);
        assert!(tx.summary.contains("transfer"));
    }

    #[test]
    fn test_decode_create_object() {
        let mut dec = TxDecoder::new();
        dec.register_defaults();
        let tx = dec.decode(
            "txhash_obj", "0x01000001", "evap1creator", None,
            0, 5, &[("object_type", "document"), ("data", "0xabcdef")],
        );
        assert_eq!(tx.category, TxCategory::ObjectCreation);
        assert_eq!(tx.method_name, "create_object");
        assert_eq!(tx.params.len(), 2);
    }

    #[test]
    fn test_decode_mint_nft() {
        let mut dec = TxDecoder::new();
        dec.register_defaults();
        let tx = dec.decode(
            "txhash_nft", "0x03000001", "evap1artist", None,
            0, 3, &[("collection", "evap1col"), ("metadata", "ipfs://abc")],
        );
        assert_eq!(tx.category, TxCategory::NftMint);
        assert_eq!(tx.method_name, "mint_nft");
    }

    #[test]
    fn test_decode_summary() {
        let mut dec = TxDecoder::new();
        dec.register_defaults();
        let summary = dec.decode_summary("0xa9059cbb", 500, "evap1sender_long", Some("evap1recv_long"));
        assert!(summary.contains("transfer"));
        assert!(summary.contains("500"));
        assert!(summary.contains("evap1sen..."));
    }

    #[test]
    fn test_decoded_tx_display() {
        let tx = DecodedTx::new("hash", TxCategory::Transfer, "transfer")
            .with_addresses("evap1from", Some("evap1to"))
            .with_value(100)
            .with_fee(5)
            .with_param(DecodedParam::new("to", ParamType::Address, "evap1to"))
            .with_summary("Transfer 100 to evap1to");
        let display = tx.display();
        assert!(display.contains("Category: Transfer"));
        assert!(display.contains("Method:   transfer"));
        assert!(display.contains("From:     evap1from"));
        assert!(display.contains("To:       evap1to"));
        assert!(display.contains("Value:    100"));
        assert!(display.contains("Fee:      5"));
        assert!(display.contains("to (Address): evap1to"));
        assert!(display.contains("Summary:  Transfer 100 to evap1to"));
    }

    #[test]
    fn test_decoded_tx_with_addresses() {
        let tx = DecodedTx::new("h", TxCategory::Unknown, "x")
            .with_addresses("from_addr", Some("to_addr"));
        assert_eq!(tx.from_address, "from_addr");
        assert_eq!(tx.to_address, Some("to_addr".to_string()));

        let tx2 = DecodedTx::new("h2", TxCategory::Unknown, "x")
            .with_addresses("from_addr", None);
        assert!(tx2.to_address.is_none());
    }

    #[test]
    fn test_list_methods() {
        let mut dec = TxDecoder::new();
        dec.register_defaults();
        let methods = dec.list_methods();
        assert_eq!(methods.len(), 12);
    }

    #[test]
    fn test_list_contracts() {
        let mut dec = TxDecoder::new();
        dec.register_contract("evap1aaa", "TokenA");
        dec.register_contract("evap1bbb", "TokenB");
        let contracts = dec.list_contracts();
        assert_eq!(contracts.len(), 2);
        let names: Vec<&str> = contracts.iter().map(|(_, n)| *n).collect();
        assert!(names.contains(&"TokenA"));
        assert!(names.contains(&"TokenB"));
    }

    #[test]
    fn test_cache_prune_at_max() {
        let mut dec = TxDecoder::new();
        dec.max_cache = 3;
        dec.register_method(MethodSignature::new("0x01", "m", TxCategory::Transfer));
        dec.decode("h1", "0x01", "a", None, 0, 0, &[]);
        dec.decode("h2", "0x01", "a", None, 0, 0, &[]);
        dec.decode("h3", "0x01", "a", None, 0, 0, &[]);
        assert_eq!(dec.decoded_cache.len(), 3);
        // This should trigger a prune (clear) then insert
        dec.decode("h4", "0x01", "a", None, 0, 0, &[]);
        assert_eq!(dec.decoded_cache.len(), 1);
        assert!(dec.decoded_cache.contains_key("h4"));
    }

    #[test]
    fn test_stats() {
        let mut dec = TxDecoder::new();
        dec.register_defaults();
        dec.register_contract("evap1aaa", "Token");
        dec.decode("h1", "0xa9059cbb", "a", None, 0, 0, &[]);
        let stats = dec.stats();
        assert_eq!(stats.total_methods, 12);
        assert_eq!(stats.total_contracts, 1);
        assert_eq!(stats.cached_decodings, 1);
    }

    #[test]
    fn test_param_types() {
        // Ensure all ParamType variants are distinct and clonable.
        let types = vec![
            ParamType::Address,
            ParamType::Uint,
            ParamType::Int,
            ParamType::String,
            ParamType::Bool,
            ParamType::Bytes,
            ParamType::Array,
        ];
        for (i, a) in types.iter().enumerate() {
            for (j, b) in types.iter().enumerate() {
                if i == j {
                    assert_eq!(a, b);
                } else {
                    assert_ne!(a, b);
                }
            }
        }
        let cloned = types[0].clone();
        assert_eq!(cloned, ParamType::Address);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path("roundtrip");
        let mut dec = TxDecoder::new();
        dec.register_defaults();
        dec.register_contract("evap1contract", "TestContract");
        dec.decode("h_persist", "0xa9059cbb", "evap1from", Some("evap1to"), 42, 1, &[("to", "evap1to"), ("amount", "42")]);

        dec.save(&path).expect("save failed");
        let loaded = TxDecoder::load(&path).expect("load failed");
        assert_eq!(loaded.signatures.len(), 12);
        assert_eq!(loaded.known_contracts.get("evap1contract").unwrap(), "TestContract");
        assert!(loaded.decoded_cache.contains_key("h_persist"));
        assert_eq!(loaded.decoded_cache["h_persist"].value, 42);

        // Clean up
        let _ = std::fs::remove_file(&path);

        // load_or_default on missing file
        let default = TxDecoder::load_or_default(&test_path("nonexistent"));
        assert!(default.signatures.is_empty());
        assert_eq!(default.max_cache, 1000);
    }
}
