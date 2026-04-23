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
    /// Current call depth (0 = top-level call). Max depth = 8.
    pub call_depth: u8,
}

/// Maximum cross-contract call depth to prevent unbounded recursion.
pub const MAX_CALL_DEPTH: u8 = 8;

/// Callback for cross-contract calls. The ScriptEngine implements this
/// so the VM can invoke other contracts during execution.
pub trait ExternalCaller: Send {
    fn call_external(
        &mut self,
        contract_id: u64,
        method: &str,
        args: Vec<Value>,
        caller: AccountAddress,
        epoch: Epoch,
        call_depth: u8,
        gas_remaining: u64,
    ) -> Result<(Value, Vec<ContractEvent>, u64), ScriptError>;
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

/// Snapshot of contract data for cross-contract call routing.
/// Avoids `&mut` aliasing issues by cloning the registry for the call stack.
struct ContractCallRouter {
    contracts: HashMap<u64, (compiler::EvaporBytecode, HashMap<String, Value>, AccountAddress, Epoch, Energy, HalfLife, Epoch, bool)>,
    vrf_randomness: [u8; 32],
    state_patches: Vec<(u64, HashMap<String, Value>)>,
    collected_events: Vec<ContractEvent>,
}

impl ContractCallRouter {
    fn from_engine(engine: &ScriptEngine) -> Self {
        let mut contracts = HashMap::new();
        for (id, c) in &engine.contracts {
            contracts.insert(*id, (
                c.bytecode.clone(),
                c.state.clone(),
                c.creator,
                c.created_epoch,
                c.energy,
                c.half_life,
                c.last_refreshed,
                c.evaporated,
            ));
        }
        Self {
            contracts,
            vrf_randomness: engine.vrf_randomness,
            state_patches: Vec::new(),
            collected_events: Vec::new(),
        }
    }
}

impl ExternalCaller for ContractCallRouter {
    fn call_external(
        &mut self,
        contract_id: u64,
        method: &str,
        args: Vec<Value>,
        caller: AccountAddress,
        epoch: Epoch,
        call_depth: u8,
        gas_remaining: u64,
    ) -> Result<(Value, Vec<ContractEvent>, u64), ScriptError> {
        if call_depth >= MAX_CALL_DEPTH {
            return Err(ScriptError::Runtime(format!(
                "cross-contract call depth exceeded (max {})", MAX_CALL_DEPTH
            )));
        }

        let (bytecode, state, creator, _created, energy, half_life, last_refreshed, evaporated) =
            self.contracts.get(&contract_id)
                .ok_or_else(|| ScriptError::Runtime(format!("contract {contract_id} not found")))?
                .clone();

        if evaporated {
            return Err(ScriptError::Runtime(format!("contract {contract_id} has evaporated")));
        }

        let current_energy = energy_at_epoch(energy, half_life, epoch.saturating_sub(last_refreshed));
        if current_energy == 0 {
            return Err(ScriptError::Runtime(format!("contract {contract_id} has no energy")));
        }

        let ctx = ExecutionContext {
            caller,
            owner: creator,
            epoch,
            energy: current_energy,
            vrf_randomness: self.vrf_randomness,
            call_depth,
        };

        let result = vm::EvaporVM::execute_full(
            &bytecode, method, args, state, &ctx, gas_remaining, Some(self),
        )?;

        // Collect state changes for later application
        self.state_patches.push((contract_id, result.state_changes));
        self.collected_events.extend(result.structured_events.clone());

        Ok((result.return_value, result.structured_events, result.gas_used))
    }
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
            call_depth: 0,
        };

        let bytecode = contract.bytecode.clone();
        let state = contract.state.clone();

        let mut router = ContractCallRouter::from_engine(self);
        let result = vm::EvaporVM::execute_full(
            &bytecode, method, args, state, &ctx, 0, Some(&mut router),
        )?;

        // Apply state changes from this contract
        let contract = self.contracts.get_mut(&contract_id).unwrap();
        for (key, value) in &result.state_changes {
            contract.state.insert(key.clone(), value.clone());
        }

        // Apply state changes from cross-contract calls
        for (target_id, patches) in router.state_patches {
            if let Some(target) = self.contracts.get_mut(&target_id) {
                for (key, value) in patches {
                    target.state.insert(key, value);
                }
            }
        }

        // Merge events from sub-calls
        let mut all_structured = result.structured_events.clone();
        all_structured.extend(router.collected_events);

        Ok(ScriptCallResult {
            return_value: result.return_value,
            events: result.events,
            structured_events: all_structured,
            gas_used: result.gas_used,
            state_changes: result.state_changes,
        })
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
            call_depth: 0,
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

#[cfg(test)]
mod cross_contract_tests {
    use super::*;

    #[test]
    fn test_cross_contract_call_basic() {
        let adder_src = r#"
contract Adder {
    state { total: u64 = 0 }
    fn add(x: u64) -> u64 {
        self.total = self.total + x
        return self.total
    }
    fn get_total() -> u64 {
        return self.total
    }
}
"#;
        let caller_src = r#"
contract Caller {
    state { last_result: u64 = 0 }
    fn call_add(target: u64, amount: u64) -> u64 {
        let result: u64 = call_contract(target, "add", amount)
        self.last_result = result
        return result
    }
}
"#;
        let mut engine = ScriptEngine::new();
        let creator = [1u8; 32];

        let adder_id = engine.deploy(adder_src, creator, 10_000, 100, 1).unwrap();
        let caller_id = engine.deploy(caller_src, creator, 10_000, 100, 1).unwrap();

        let result = engine.call(
            caller_id, "call_add",
            vec![Value::U64(adder_id), Value::U64(42)],
            creator, 10,
        ).unwrap();

        assert_eq!(result.return_value, Value::U64(42));

        let adder_total = engine.call(adder_id, "get_total", vec![], creator, 10).unwrap();
        assert_eq!(adder_total.return_value, Value::U64(42));
    }

    #[test]
    fn test_cross_contract_call_depth_limit() {
        let src = r#"
contract Recurse {
    state { v: u64 = 0 }
    fn recurse(self_id: u64) -> u64 {
        return call_contract(self_id, "recurse", self_id)
    }
}
"#;
        let mut engine = ScriptEngine::new();
        let creator = [1u8; 32];
        let id = engine.deploy(src, creator, 10_000, 100, 1).unwrap();

        let result = engine.call(id, "recurse", vec![Value::U64(id)], creator, 10);
        assert!(result.is_err(), "recursive calls must hit depth limit");
        let err = format!("{:?}", result.unwrap_err());
        assert!(err.contains("depth"), "error should mention depth: {err}");
    }

    #[test]
    fn test_cross_contract_call_nonexistent_target() {
        let src = r#"
contract Caller {
    state { v: u64 = 0 }
    fn call_missing() -> u64 {
        return call_contract(999, "get", 0)
    }
}
"#;
        let mut engine = ScriptEngine::new();
        let creator = [1u8; 32];
        let id = engine.deploy(src, creator, 10_000, 100, 1).unwrap();

        let result = engine.call(id, "call_missing", vec![], creator, 10);
        assert!(result.is_err(), "calling nonexistent contract must error");
    }

    #[test]
    fn test_cross_contract_state_isolation() {
        let storage_src = r#"
contract Storage {
    state { value: u64 = 0 }
    fn set(x: u64) -> u64 {
        self.value = x
        return self.value
    }
    fn get() -> u64 {
        return self.value
    }
}
"#;
        let mut engine = ScriptEngine::new();
        let creator = [1u8; 32];

        let s1 = engine.deploy(storage_src, creator, 10_000, 100, 1).unwrap();
        let s2 = engine.deploy(storage_src, creator, 10_000, 100, 1).unwrap();

        engine.call(s1, "set", vec![Value::U64(100)], creator, 10).unwrap();
        engine.call(s2, "set", vec![Value::U64(200)], creator, 10).unwrap();

        let v1 = engine.call(s1, "get", vec![], creator, 10).unwrap();
        let v2 = engine.call(s2, "get", vec![], creator, 10).unwrap();

        assert_eq!(v1.return_value, Value::U64(100));
        assert_eq!(v2.return_value, Value::U64(200));
    }

    #[test]
    fn test_cross_contract_gas_forwarding() {
        let work_src = r#"
contract Worker {
    state { v: u64 = 0 }
    fn work() -> u64 {
        let i: u64 = 0
        while i < 50 {
            i = i + 1
            self.v = self.v + 1
        }
        return self.v
    }
}
"#;
        let caller_src = r#"
contract Boss {
    state { result: u64 = 0 }
    fn delegate(target: u64) -> u64 {
        let r: u64 = call_contract(target, "work")
        self.result = r
        return r
    }
}
"#;
        let mut engine = ScriptEngine::new();
        let creator = [1u8; 32];

        let worker = engine.deploy(work_src, creator, 10_000, 100, 1).unwrap();
        let boss = engine.deploy(caller_src, creator, 10_000, 100, 1).unwrap();

        let result = engine.call(
            boss, "delegate", vec![Value::U64(worker)], creator, 10,
        ).unwrap();
        assert_eq!(result.return_value, Value::U64(50));
        assert!(result.gas_used > 0);
    }
}
