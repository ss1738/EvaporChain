//! Contract ABI storage and interaction builder for the EvaporChain wallet.
//!
//! Stores contract ABIs keyed by address, builds typed call data from ABI
//! entries, and records event logs. Persisted to JSON on disk.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum ContractAbiError {
    #[error("contract already exists: {0}")]
    AlreadyExists(String),
    #[error("contract not found: {0}")]
    NotFound(String),
    #[error("function not found: {0}")]
    FunctionNotFound(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ──────────────────────────── Enums ─────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AbiType {
    Function,
    Event,
    Constructor,
    Fallback,
    Receive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParamKind {
    Uint256,
    Int256,
    Address,
    Bool,
    String,
    Bytes,
    BytesN(u8),
    Array(Box<ParamKind>),
    Tuple(Vec<ParamKind>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StateMutability {
    Pure,
    View,
    Nonpayable,
    Payable,
}

// ──────────────────────────── AbiParam ──────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbiParam {
    pub name: String,
    pub kind: ParamKind,
    pub indexed: bool,
}

impl AbiParam {
    pub fn new(name: &str, kind: ParamKind) -> Self {
        Self {
            name: name.to_string(),
            kind,
            indexed: false,
        }
    }

    pub fn with_indexed(mut self) -> Self {
        self.indexed = true;
        self
    }

    /// Returns canonical type string ("uint256", "address", "bytes32", etc.).
    pub fn type_string(&self) -> String {
        param_kind_to_string(&self.kind)
    }
}

fn param_kind_to_string(kind: &ParamKind) -> String {
    match kind {
        ParamKind::Uint256 => "uint256".to_string(),
        ParamKind::Int256 => "int256".to_string(),
        ParamKind::Address => "address".to_string(),
        ParamKind::Bool => "bool".to_string(),
        ParamKind::String => "string".to_string(),
        ParamKind::Bytes => "bytes".to_string(),
        ParamKind::BytesN(n) => format!("bytes{}", n),
        ParamKind::Array(inner) => format!("{}[]", param_kind_to_string(inner)),
        ParamKind::Tuple(fields) => {
            let inner: Vec<String> = fields.iter().map(param_kind_to_string).collect();
            format!("({})", inner.join(","))
        }
    }
}

// ──────────────────────────── AbiEntry ──────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbiEntry {
    pub name: String,
    pub abi_type: AbiType,
    pub inputs: Vec<AbiParam>,
    pub outputs: Vec<AbiParam>,
    pub state_mutability: StateMutability,
    pub selector: String,
}

impl AbiEntry {
    /// Create a new function entry. Selector = first 8 hex chars of blake3(name).
    pub fn new_function(name: &str, mutability: StateMutability) -> Self {
        let hash = blake3::hash(name.as_bytes());
        let selector = hash.to_hex().to_string()[..8].to_string();
        Self {
            name: name.to_string(),
            abi_type: AbiType::Function,
            inputs: Vec::new(),
            outputs: Vec::new(),
            state_mutability: mutability,
            selector,
        }
    }

    /// Create a new event entry.
    pub fn new_event(name: &str) -> Self {
        let hash = blake3::hash(name.as_bytes());
        let selector = hash.to_hex().to_string()[..8].to_string();
        Self {
            name: name.to_string(),
            abi_type: AbiType::Event,
            inputs: Vec::new(),
            outputs: Vec::new(),
            state_mutability: StateMutability::Nonpayable,
            selector,
        }
    }

    pub fn add_input(mut self, param: AbiParam) -> Self {
        self.inputs.push(param);
        self
    }

    pub fn add_output(mut self, param: AbiParam) -> Self {
        self.outputs.push(param);
        self
    }

    /// Returns "name(type1,type2)" format.
    pub fn signature(&self) -> String {
        let types: Vec<String> = self.inputs.iter().map(|p| p.type_string()).collect();
        format!("{}({})", self.name, types.join(","))
    }

    /// True if state mutability is Pure or View.
    pub fn is_readonly(&self) -> bool {
        matches!(self.state_mutability, StateMutability::Pure | StateMutability::View)
    }
}

// ──────────────────────────── ContractAbi ───────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractAbi {
    pub address: String,
    pub name: String,
    pub entries: Vec<AbiEntry>,
    pub verified: bool,
    pub added_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compiler_version: Option<String>,
}

impl ContractAbi {
    pub fn new(address: &str, name: &str) -> Self {
        Self {
            address: address.to_string(),
            name: name.to_string(),
            entries: Vec::new(),
            verified: false,
            added_at: chrono::Utc::now().to_rfc3339(),
            source_url: None,
            compiler_version: None,
        }
    }

    pub fn add_entry(&mut self, entry: AbiEntry) {
        self.entries.push(entry);
    }

    pub fn functions(&self) -> Vec<&AbiEntry> {
        self.entries.iter().filter(|e| e.abi_type == AbiType::Function).collect()
    }

    pub fn events(&self) -> Vec<&AbiEntry> {
        self.entries.iter().filter(|e| e.abi_type == AbiType::Event).collect()
    }

    pub fn get_function(&self, name: &str) -> Option<&AbiEntry> {
        self.entries
            .iter()
            .find(|e| e.abi_type == AbiType::Function && e.name == name)
    }

    pub fn get_event(&self, name: &str) -> Option<&AbiEntry> {
        self.entries
            .iter()
            .find(|e| e.abi_type == AbiType::Event && e.name == name)
    }

    pub fn get_by_selector(&self, selector: &str) -> Option<&AbiEntry> {
        self.entries.iter().find(|e| e.selector == selector)
    }

    pub fn readonly_functions(&self) -> Vec<&AbiEntry> {
        self.entries
            .iter()
            .filter(|e| e.abi_type == AbiType::Function && e.is_readonly())
            .collect()
    }

    pub fn payable_functions(&self) -> Vec<&AbiEntry> {
        self.entries
            .iter()
            .filter(|e| {
                e.abi_type == AbiType::Function
                    && e.state_mutability == StateMutability::Payable
            })
            .collect()
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

// ──────────────────────────── CallBuilder / CallData ────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallData {
    pub contract: String,
    pub function: String,
    pub selector: String,
    pub args: HashMap<String, String>,
    pub value: u64,
    pub sender: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CallBuilder {
    pub contract_address: String,
    pub function_name: String,
    pub selector: String,
    pub args: Vec<(String, String)>,
    pub value: u64,
    pub sender: Option<String>,
}

impl CallBuilder {
    pub fn new(contract_address: &str, function_name: &str, selector: &str) -> Self {
        Self {
            contract_address: contract_address.to_string(),
            function_name: function_name.to_string(),
            selector: selector.to_string(),
            args: Vec::new(),
            value: 0,
            sender: None,
        }
    }

    pub fn with_arg(mut self, name: &str, value: &str) -> Self {
        self.args.push((name.to_string(), value.to_string()));
        self
    }

    pub fn with_value(mut self, v: u64) -> Self {
        self.value = v;
        self
    }

    pub fn with_sender(mut self, addr: &str) -> Self {
        self.sender = Some(addr.to_string());
        self
    }

    pub fn build(&self) -> CallData {
        let args: HashMap<String, String> = self.args.iter().cloned().collect();
        CallData {
            contract: self.contract_address.clone(),
            function: self.function_name.clone(),
            selector: self.selector.clone(),
            args,
            value: self.value,
            sender: self.sender.clone(),
        }
    }

    /// "call contract.function(arg1=val1, arg2=val2) value=V"
    pub fn display(&self) -> String {
        let args_str: Vec<String> = self
            .args
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        format!(
            "call {}.{}({}) value={}",
            self.contract_address,
            self.function_name,
            args_str.join(", "),
            self.value,
        )
    }
}

// ──────────────────────────── EventLog ──────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventLog {
    pub event_name: String,
    pub contract: String,
    pub params: Vec<(String, String)>,
    pub block_height: u64,
    pub tx_hash: String,
    pub log_index: u32,
    pub timestamp: String,
}

impl EventLog {
    pub fn new(
        event_name: &str,
        contract: &str,
        block_height: u64,
        tx_hash: &str,
        log_index: u32,
    ) -> Self {
        Self {
            event_name: event_name.to_string(),
            contract: contract.to_string(),
            params: Vec::new(),
            block_height,
            tx_hash: tx_hash.to_string(),
            log_index,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub fn add_param(mut self, name: &str, value: &str) -> Self {
        self.params.push((name.to_string(), value.to_string()));
        self
    }
}

// ──────────────────────────── AbiStats ──────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct AbiStats {
    pub total_contracts: usize,
    pub total_functions: usize,
    pub total_events: usize,
    pub total_event_logs: usize,
    pub verified_contracts: usize,
}

// ──────────────────────────── AbiStore ──────────────────────────────────

const MAX_EVENT_LOGS: usize = 5000;

#[derive(Debug, Serialize, Deserialize)]
pub struct AbiStore {
    pub contracts: HashMap<String, ContractAbi>,
    pub event_logs: Vec<EventLog>,
}

impl Default for AbiStore {
    fn default() -> Self {
        Self::new()
    }
}

impl AbiStore {
    pub fn new() -> Self {
        Self {
            contracts: HashMap::new(),
            event_logs: Vec::new(),
        }
    }

    /// Register a new contract ABI. Fails if the address is already registered.
    pub fn register(&mut self, abi: ContractAbi) -> Result<(), ContractAbiError> {
        if self.contracts.contains_key(&abi.address) {
            return Err(ContractAbiError::AlreadyExists(abi.address));
        }
        self.contracts.insert(abi.address.clone(), abi);
        Ok(())
    }

    /// Replace an existing contract ABI (or insert if new).
    pub fn update(&mut self, abi: ContractAbi) {
        self.contracts.insert(abi.address.clone(), abi);
    }

    /// Remove a contract by address, returning the removed ABI.
    pub fn remove(&mut self, address: &str) -> Result<ContractAbi, ContractAbiError> {
        self.contracts
            .remove(address)
            .ok_or_else(|| ContractAbiError::NotFound(address.to_string()))
    }

    pub fn get(&self, address: &str) -> Option<&ContractAbi> {
        self.contracts.get(address)
    }

    pub fn list(&self) -> Vec<&ContractAbi> {
        self.contracts.values().collect()
    }

    /// Case-insensitive search on contract name and address.
    pub fn search(&self, query: &str) -> Vec<&ContractAbi> {
        let q = query.to_lowercase();
        self.contracts
            .values()
            .filter(|c| {
                c.name.to_lowercase().contains(&q) || c.address.to_lowercase().contains(&q)
            })
            .collect()
    }

    /// Build a call for the given contract address and function name.
    pub fn build_call(
        &self,
        address: &str,
        function: &str,
    ) -> Result<CallBuilder, ContractAbiError> {
        let contract = self
            .contracts
            .get(address)
            .ok_or_else(|| ContractAbiError::NotFound(address.to_string()))?;
        let entry = contract
            .get_function(function)
            .ok_or_else(|| ContractAbiError::FunctionNotFound(function.to_string()))?;
        Ok(CallBuilder::new(address, function, &entry.selector))
    }

    /// Push an event log, pruning oldest entries if over the cap.
    pub fn log_event(&mut self, log: EventLog) {
        self.event_logs.push(log);
        if self.event_logs.len() > MAX_EVENT_LOGS {
            let excess = self.event_logs.len() - MAX_EVENT_LOGS;
            self.event_logs.drain(..excess);
        }
    }

    pub fn events_for_contract(&self, address: &str) -> Vec<&EventLog> {
        self.event_logs
            .iter()
            .filter(|e| e.contract == address)
            .collect()
    }

    pub fn events_for_tx(&self, tx_hash: &str) -> Vec<&EventLog> {
        self.event_logs
            .iter()
            .filter(|e| e.tx_hash == tx_hash)
            .collect()
    }

    pub fn recent_events(&self, n: usize) -> Vec<&EventLog> {
        self.event_logs.iter().rev().take(n).collect()
    }

    pub fn stats(&self) -> AbiStats {
        let total_functions: usize = self
            .contracts
            .values()
            .map(|c| c.functions().len())
            .sum();
        let total_events: usize = self.contracts.values().map(|c| c.events().len()).sum();
        let verified_contracts = self.contracts.values().filter(|c| c.verified).count();
        AbiStats {
            total_contracts: self.contracts.len(),
            total_functions,
            total_events,
            total_event_logs: self.event_logs.len(),
            verified_contracts,
        }
    }

    // ── Persistence ──────────────────────────────────────────────────

    pub fn save(&self, path: &Path) -> Result<(), ContractAbiError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, ContractAbiError> {
        let data = std::fs::read_to_string(path)?;
        let store: Self = serde_json::from_str(&data)?;
        Ok(store)
    }

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
            "contract_abi_test_{}_{}.json",
            name,
            std::process::id()
        ))
    }

    fn sample_contract() -> ContractAbi {
        let mut abi = ContractAbi::new("0xABC123", "TestToken");
        abi.add_entry(
            AbiEntry::new_function("transfer", StateMutability::Nonpayable)
                .add_input(AbiParam::new("to", ParamKind::Address))
                .add_input(AbiParam::new("amount", ParamKind::Uint256))
                .add_output(AbiParam::new("success", ParamKind::Bool)),
        );
        abi.add_entry(
            AbiEntry::new_function("balanceOf", StateMutability::View)
                .add_input(AbiParam::new("owner", ParamKind::Address))
                .add_output(AbiParam::new("balance", ParamKind::Uint256)),
        );
        abi.add_entry(
            AbiEntry::new_function("deposit", StateMutability::Payable),
        );
        abi.add_entry(
            AbiEntry::new_event("Transfer")
                .add_input(AbiParam::new("from", ParamKind::Address).with_indexed())
                .add_input(AbiParam::new("to", ParamKind::Address).with_indexed())
                .add_input(AbiParam::new("value", ParamKind::Uint256)),
        );
        abi
    }

    #[test]
    fn test_new_function_entry() {
        let entry = AbiEntry::new_function("transfer", StateMutability::Nonpayable);
        assert_eq!(entry.abi_type, AbiType::Function);
        assert_eq!(entry.name, "transfer");
        assert_eq!(entry.state_mutability, StateMutability::Nonpayable);
        assert_eq!(entry.selector.len(), 8);
    }

    #[test]
    fn test_new_event_entry() {
        let entry = AbiEntry::new_event("Transfer");
        assert_eq!(entry.abi_type, AbiType::Event);
        assert_eq!(entry.name, "Transfer");
        assert_eq!(entry.selector.len(), 8);
    }

    #[test]
    fn test_entry_signature() {
        let entry = AbiEntry::new_function("transfer", StateMutability::Nonpayable)
            .add_input(AbiParam::new("to", ParamKind::Address))
            .add_input(AbiParam::new("amount", ParamKind::Uint256));
        assert_eq!(entry.signature(), "transfer(address,uint256)");
    }

    #[test]
    fn test_abi_param_type_string() {
        assert_eq!(AbiParam::new("a", ParamKind::Uint256).type_string(), "uint256");
        assert_eq!(AbiParam::new("b", ParamKind::Address).type_string(), "address");
        assert_eq!(AbiParam::new("c", ParamKind::BytesN(32)).type_string(), "bytes32");
        assert_eq!(
            AbiParam::new("d", ParamKind::Array(Box::new(ParamKind::Uint256))).type_string(),
            "uint256[]"
        );
        assert_eq!(
            AbiParam::new("e", ParamKind::Tuple(vec![ParamKind::Address, ParamKind::Bool]))
                .type_string(),
            "(address,bool)"
        );
    }

    #[test]
    fn test_contract_abi_functions() {
        let abi = sample_contract();
        let fns = abi.functions();
        assert_eq!(fns.len(), 3);
        assert!(fns.iter().all(|f| f.abi_type == AbiType::Function));
    }

    #[test]
    fn test_contract_abi_events() {
        let abi = sample_contract();
        let evs = abi.events();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].name, "Transfer");
    }

    #[test]
    fn test_get_function() {
        let abi = sample_contract();
        let f = abi.get_function("transfer").unwrap();
        assert_eq!(f.name, "transfer");
        assert!(abi.get_function("nonexistent").is_none());
    }

    #[test]
    fn test_get_by_selector() {
        let abi = sample_contract();
        let entry = &abi.entries[0];
        let sel = entry.selector.clone();
        let found = abi.get_by_selector(&sel).unwrap();
        assert_eq!(found.name, entry.name);
        assert!(abi.get_by_selector("00000000").is_none());
    }

    #[test]
    fn test_readonly_functions() {
        let abi = sample_contract();
        let ro = abi.readonly_functions();
        assert_eq!(ro.len(), 1);
        assert_eq!(ro[0].name, "balanceOf");
    }

    #[test]
    fn test_payable_functions() {
        let abi = sample_contract();
        let pay = abi.payable_functions();
        assert_eq!(pay.len(), 1);
        assert_eq!(pay[0].name, "deposit");
    }

    #[test]
    fn test_register_and_get() {
        let mut store = AbiStore::new();
        let abi = sample_contract();
        store.register(abi).unwrap();
        assert!(store.get("0xABC123").is_some());
        assert_eq!(store.get("0xABC123").unwrap().name, "TestToken");
    }

    #[test]
    fn test_register_duplicate_rejected() {
        let mut store = AbiStore::new();
        store.register(sample_contract()).unwrap();
        let result = store.register(sample_contract());
        assert!(result.is_err());
        match result.unwrap_err() {
            ContractAbiError::AlreadyExists(addr) => assert_eq!(addr, "0xABC123"),
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn test_remove_contract() {
        let mut store = AbiStore::new();
        store.register(sample_contract()).unwrap();
        let removed = store.remove("0xABC123").unwrap();
        assert_eq!(removed.name, "TestToken");
        assert!(store.get("0xABC123").is_none());
        assert!(store.remove("0xABC123").is_err());
    }

    #[test]
    fn test_search_contracts() {
        let mut store = AbiStore::new();
        store.register(sample_contract()).unwrap();
        let mut abi2 = ContractAbi::new("0xDEF456", "VaultContract");
        abi2.add_entry(AbiEntry::new_function("lock", StateMutability::Nonpayable));
        store.register(abi2).unwrap();

        let results = store.search("token");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "TestToken");

        let results2 = store.search("0xdef");
        assert_eq!(results2.len(), 1);
        assert_eq!(results2[0].name, "VaultContract");
    }

    #[test]
    fn test_build_call() {
        let mut store = AbiStore::new();
        store.register(sample_contract()).unwrap();
        let builder = store.build_call("0xABC123", "transfer").unwrap();
        assert_eq!(builder.function_name, "transfer");
        assert_eq!(builder.contract_address, "0xABC123");
        assert!(!builder.selector.is_empty());
    }

    #[test]
    fn test_build_call_with_args() {
        let mut store = AbiStore::new();
        store.register(sample_contract()).unwrap();
        let builder = store
            .build_call("0xABC123", "transfer")
            .unwrap()
            .with_arg("to", "0x999")
            .with_arg("amount", "1000")
            .with_value(50)
            .with_sender("0xSENDER");

        let call = builder.build();
        assert_eq!(call.contract, "0xABC123");
        assert_eq!(call.function, "transfer");
        assert_eq!(call.args.get("to").unwrap(), "0x999");
        assert_eq!(call.args.get("amount").unwrap(), "1000");
        assert_eq!(call.value, 50);
        assert_eq!(call.sender.as_deref(), Some("0xSENDER"));
    }

    #[test]
    fn test_call_builder_display() {
        let builder = CallBuilder::new("0xABC", "transfer", "abcd1234")
            .with_arg("to", "0x999")
            .with_arg("amount", "100")
            .with_value(5);
        let d = builder.display();
        assert!(d.contains("call 0xABC.transfer("));
        assert!(d.contains("to=0x999"));
        assert!(d.contains("amount=100"));
        assert!(d.contains("value=5"));
    }

    #[test]
    fn test_log_event() {
        let mut store = AbiStore::new();
        let log = EventLog::new("Transfer", "0xABC123", 100, "0xTX1", 0)
            .add_param("from", "0x111")
            .add_param("to", "0x222");
        store.log_event(log);
        assert_eq!(store.event_logs.len(), 1);
        assert_eq!(store.event_logs[0].params.len(), 2);
    }

    #[test]
    fn test_events_for_contract() {
        let mut store = AbiStore::new();
        store.log_event(EventLog::new("Transfer", "0xABC", 1, "0xT1", 0));
        store.log_event(EventLog::new("Approval", "0xDEF", 2, "0xT2", 0));
        store.log_event(EventLog::new("Mint", "0xABC", 3, "0xT3", 0));
        let logs = store.events_for_contract("0xABC");
        assert_eq!(logs.len(), 2);
    }

    #[test]
    fn test_events_for_tx() {
        let mut store = AbiStore::new();
        store.log_event(EventLog::new("Transfer", "0xABC", 1, "0xTX1", 0));
        store.log_event(EventLog::new("Approval", "0xABC", 1, "0xTX1", 1));
        store.log_event(EventLog::new("Transfer", "0xABC", 2, "0xTX2", 0));
        let logs = store.events_for_tx("0xTX1");
        assert_eq!(logs.len(), 2);
    }

    #[test]
    fn test_recent_events() {
        let mut store = AbiStore::new();
        for i in 0..10 {
            store.log_event(EventLog::new(
                &format!("Event{}", i),
                "0xABC",
                i as u64,
                &format!("0xTX{}", i),
                0,
            ));
        }
        let recent = store.recent_events(3);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].event_name, "Event9");
        assert_eq!(recent[1].event_name, "Event8");
        assert_eq!(recent[2].event_name, "Event7");
    }

    #[test]
    fn test_stats() {
        let mut store = AbiStore::new();
        let mut abi = sample_contract();
        abi.verified = true;
        store.register(abi).unwrap();
        store.log_event(EventLog::new("Transfer", "0xABC123", 1, "0xT1", 0));

        let stats = store.stats();
        assert_eq!(stats.total_contracts, 1);
        assert_eq!(stats.total_functions, 3);
        assert_eq!(stats.total_events, 1);
        assert_eq!(stats.total_event_logs, 1);
        assert_eq!(stats.verified_contracts, 1);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path("roundtrip");
        let mut store = AbiStore::new();
        store.register(sample_contract()).unwrap();
        store.log_event(
            EventLog::new("Transfer", "0xABC123", 42, "0xTXHASH", 0)
                .add_param("from", "0x111"),
        );

        store.save(&path).unwrap();
        let loaded = AbiStore::load(&path).unwrap();
        assert_eq!(loaded.contracts.len(), 1);
        assert_eq!(loaded.event_logs.len(), 1);
        assert_eq!(
            loaded.get("0xABC123").unwrap().name,
            "TestToken"
        );

        // Clean up
        let _ = std::fs::remove_file(&path);
    }
}
