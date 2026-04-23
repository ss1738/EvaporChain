pub mod compiler;
pub mod parser;
pub mod vm;
#[cfg(test)]
mod audit_tests;

use evaporchain_types::{energy_at_epoch, AccountAddress, Energy, Epoch, HalfLife};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

// ─── Shared Types ───────────────────────────────────────────────────────────

/// Runtime value in EvaporScript.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    U64(u64),
    Bool(bool),
    Str(String),
    Address([u8; 32]),
    Map(HashMap<String, Value>),
    Null,
}

impl Value {
    pub fn as_u64(&self) -> Result<u64, ScriptError> {
        match self {
            Value::U64(n) => Ok(*n),
            other => Err(ScriptError::Runtime(format!("expected u64, got {other:?}"))),
        }
    }

    pub fn as_bool(&self) -> Result<bool, ScriptError> {
        match self {
            Value::Bool(b) => Ok(*b),
            other => Err(ScriptError::Runtime(format!("expected bool, got {other:?}"))),
        }
    }

    pub fn as_str(&self) -> Result<&str, ScriptError> {
        match self {
            Value::Str(s) => Ok(s),
            other => Err(ScriptError::Runtime(format!(
                "expected string, got {other:?}"
            ))),
        }
    }

    pub fn as_address(&self) -> Result<[u8; 32], ScriptError> {
        match self {
            Value::Address(a) => Ok(*a),
            other => Err(ScriptError::Runtime(format!(
                "expected address, got {other:?}"
            ))),
        }
    }

    /// Convert a value to a string key for map indexing.
    pub fn to_map_key(&self) -> String {
        match self {
            Value::U64(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Str(s) => s.clone(),
            Value::Address(a) => hex::encode(a),
            Value::Null => "null".to_string(),
            Value::Map(_) => "map".to_string(),
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::U64(n) => write!(f, "{n}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Str(s) => write!(f, "{s}"),
            Value::Address(a) => write!(f, "0x{}", hex::encode(a)),
            Value::Null => write!(f, "null"),
            Value::Map(m) => write!(f, "map({} entries)", m.len()),
        }
    }
}

/// Type in the EvaporScript type system.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ScriptType {
    U64,
    Bool,
    String,
    Address,
    Map(Box<ScriptType>, Box<ScriptType>),
}

// ─── Errors ─────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum ScriptError {
    #[error("parse error at line {line}: {message}")]
    Parse { line: usize, message: String },
    #[error("compile error: {0}")]
    Compile(String),
    #[error("runtime error: {0}")]
    Runtime(String),
    #[error("gas limit exceeded: used {used}, limit {limit}")]
    GasLimitExceeded { used: u64, limit: u64 },
    #[error("require failed: {0}")]
    RequireFailed(String),
}

// ─── State Schema ───────────────────────────────────────────────────────────

/// Schema describing the state fields of a script contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSchema {
    pub fields: Vec<StateFieldSchema>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateFieldSchema {
    pub name: String,
    pub ty: ScriptType,
    pub default: Option<Value>,
}

// ─── Contract Events ───────────────────────────────────────────────────────

/// Structured contract event for indexed querying.
/// Similar to Ethereum logs but with EvaporChain's Value type system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractEvent {
    /// Event name (e.g., "Transfer", "Approval", "Swap").
    pub name: String,
    /// Indexed fields — up to 4 topics for efficient filtering.
    pub topics: Vec<Value>,
    /// Non-indexed payload data.
    pub data: Vec<Value>,
}

// ─── Execution Context ──────────────────────────────────────────────────────

/// Context passed to the VM for built-in function access.
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    pub caller: AccountAddress,
    pub owner: AccountAddress,
    pub epoch: Epoch,
    pub energy: Energy,
    /// VRF randomness beacon value for this block (32 bytes).
    /// Deterministic: same block = same randomness for all nodes.
    pub vrf_randomness: [u8; 32],
}

// ─── Script Engine ──────────────────────────────────────────────────────────

/// A deployed script contract.
#[derive(Debug, Clone)]
pub struct ScriptContract {
    pub id: u64,
    pub name: String,
    pub bytecode: compiler::EvaporBytecode,
    pub state: HashMap<String, Value>,
    pub creator: AccountAddress,
    pub created_epoch: Epoch,
    pub energy: Energy,
    pub half_life: HalfLife,
    pub last_refreshed: Epoch,
    pub evaporated: bool,
}

impl ScriptContract {
    /// Compute remaining energy at the given epoch.
    pub fn energy_at(&self, epoch: Epoch) -> u64 {
        energy_at_epoch(
            self.energy,
            self.half_life,
            epoch.saturating_sub(self.last_refreshed),
        )
    }
}

/// Result of calling a script contract method.
#[derive(Debug, Clone)]
pub struct ScriptCallResult {
    pub return_value: Value,
    pub events: Vec<String>,
    pub structured_events: Vec<ContractEvent>,
    pub gas_used: u64,
    pub state_changes: HashMap<String, Value>,
}

/// Result of ticking all script contracts.
#[derive(Debug)]
pub struct ScriptTickResult {
    pub contracts_ticked: usize,
    pub contracts_evaporated: Vec<u64>,
    pub events: Vec<String>,
    pub structured_events: Vec<ContractEvent>,
}

/// Engine managing all deployed script contracts.
pub struct ScriptEngine {
    contracts: HashMap<u64, ScriptContract>,
    next_id: u64,
    /// VRF randomness beacon for the current block. Set before executing txs.
    pub vrf_randomness: [u8; 32],
}

impl ScriptEngine {
    pub fn new() -> Self {
        Self {
            contracts: HashMap::new(),
            next_id: 1,
            vrf_randomness: [0u8; 32],
        }
    }

    /// Deploy a script contract from source code.
    pub fn deploy(
        &mut self,
        source: &str,
        creator: AccountAddress,
        energy: Energy,
        half_life: HalfLife,
        current_epoch: Epoch,
    ) -> Result<u64, ScriptError> {
        // Parse
        let contract_ast = parser::parse(source)?;
        let contract_name = contract_ast.name.clone();

        // Compile
        let bytecode = compiler::compile(&contract_ast)?;

        // Initialize state from schema defaults
        let mut state = HashMap::new();
        for field in &bytecode.state_schema.fields {
            let default_val = field.default.clone().unwrap_or_else(|| match &field.ty {
                ScriptType::U64 => Value::U64(0),
                ScriptType::Bool => Value::Bool(false),
                ScriptType::String => Value::Str(String::new()),
                ScriptType::Address => Value::Address([0u8; 32]),
                ScriptType::Map(_, _) => Value::Map(HashMap::new()),
            });
            state.insert(field.name.clone(), default_val);
        }

        let id = self.next_id;
        self.next_id += 1;

        self.contracts.insert(
            id,
            ScriptContract {
                id,
                name: contract_name,
                bytecode,
                state,
                creator,
                created_epoch: current_epoch,
                energy,
                half_life,
                last_refreshed: current_epoch,
                evaporated: false,
            },
        );

        Ok(id)
    }

    /// Call a method on a deployed script contract.
    pub fn call(
        &mut self,
        contract_id: u64,
        method: &str,
        args: Vec<Value>,
        caller: AccountAddress,
        current_epoch: Epoch,
    ) -> Result<ScriptCallResult, ScriptError> {
        let contract = self
            .contracts
            .get(&contract_id)
            .ok_or_else(|| ScriptError::Runtime(format!("contract {contract_id} not found")))?;

        if contract.evaporated {
            return Err(ScriptError::Runtime(format!(
                "contract {contract_id} has evaporated"
            )));
        }

        let current_energy = contract.energy_at(current_epoch);
        if current_energy == 0 {
            return Err(ScriptError::Runtime(format!(
                "contract {contract_id} has no energy"
            )));
        }

        let ctx = ExecutionContext {
            caller,
            owner: contract.creator,
            epoch: current_epoch,
            energy: current_energy,
            vrf_randomness: self.vrf_randomness,
        };

        let bytecode = contract.bytecode.clone();
        let state = contract.state.clone();

        let result = vm::EvaporVM::execute(&bytecode, method, args, state, &ctx)?;

        // Apply state changes
        let contract = self.contracts.get_mut(&contract_id).unwrap();
        for (key, value) in &result.state_changes {
            contract.state.insert(key.clone(), value.clone());
        }

        Ok(result)
    }

    /// Call a lifecycle hook (on_evaporate, on_grace, on_refresh) on a contract.
    pub fn call_lifecycle_hook(
        &mut self,
        contract_id: u64,
        hook: &str,
        caller: AccountAddress,
        current_epoch: Epoch,
    ) -> Result<ScriptCallResult, ScriptError> {
        let contract = self
            .contracts
            .get(&contract_id)
            .ok_or_else(|| ScriptError::Runtime(format!("contract {contract_id} not found")))?;

        // Check if the hook exists in bytecode
        if !contract.bytecode.methods.contains_key(hook) {
            return Ok(ScriptCallResult {
                return_value: Value::Null,
                events: vec![],
                structured_events: vec![],
                gas_used: 0,
                state_changes: HashMap::new(),
            });
        }

        let ctx = ExecutionContext {
            caller,
            owner: contract.creator,
            epoch: current_epoch,
            energy: contract.energy_at(current_epoch),
            vrf_randomness: self.vrf_randomness,
        };

        let bytecode = contract.bytecode.clone();
        let state = contract.state.clone();

        let result = vm::EvaporVM::execute(&bytecode, hook, vec![], state, &ctx)?;

        let contract = self.contracts.get_mut(&contract_id).unwrap();
        for (key, value) in &result.state_changes {
            contract.state.insert(key.clone(), value.clone());
        }

        Ok(result)
    }

    /// Tick all script contracts: decay energy, evaporate dead ones, fire hooks.
    pub fn tick(&mut self, current_epoch: Epoch) -> ScriptTickResult {
        let mut ticked = 0;
        let mut evaporated = Vec::new();
        let mut events = Vec::new();
        let mut structured_events = Vec::new();

        let ids: Vec<u64> = self.contracts.keys().copied().collect();

        for id in ids {
            let contract = self.contracts.get(&id).unwrap();
            if contract.evaporated {
                continue;
            }

            ticked += 1;
            let current_energy = contract.energy_at(current_epoch);

            if current_energy == 0 {
                // Fire on_evaporate hook before marking as evaporated
                let creator = contract.creator;
                if contract.bytecode.methods.contains_key("on_evaporate") {
                    if let Ok(result) =
                        self.call_lifecycle_hook(id, "on_evaporate", creator, current_epoch)
                    {
                        events.extend(result.events);
                        structured_events.extend(result.structured_events);
                    }
                }

                let contract = self.contracts.get_mut(&id).unwrap();
                contract.evaporated = true;
                evaporated.push(id);
            }
        }

        ScriptTickResult {
            contracts_ticked: ticked,
            contracts_evaporated: evaporated,
            events,
            structured_events,
        }
    }

    /// Get a reference to a deployed contract.
    pub fn get(&self, id: u64) -> Option<&ScriptContract> {
        self.contracts.get(&id)
    }

    /// List all deployed script contracts.
    pub fn list(&self) -> Vec<&ScriptContract> {
        let mut contracts: Vec<_> = self.contracts.values().collect();
        contracts.sort_by_key(|c| c.id);
        contracts
    }

    /// Refresh a script contract's energy.
    pub fn refresh_contract(
        &mut self,
        id: u64,
        energy_deposit: Energy,
        epoch: Epoch,
    ) -> Result<(), ScriptError> {
        let contract = self
            .contracts
            .get_mut(&id)
            .ok_or_else(|| ScriptError::Runtime(format!("contract {id} not found")))?;

        contract.energy = contract.energy_at(epoch) + energy_deposit;
        contract.last_refreshed = epoch;
        if contract.evaporated {
            contract.evaporated = false;
        }
        Ok(())
    }
}

impl Default for ScriptEngine {
    fn default() -> Self {
        Self::new()
    }
}
