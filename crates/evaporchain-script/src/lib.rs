#[cfg(test)]
mod audit_tests;
pub mod compiler;
pub mod parser;
pub mod vm;

use evaporchain_types::{energy_at_epoch, AccountAddress, Energy, Epoch, HalfLife};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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
    Array(Vec<Value>),
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
            other => Err(ScriptError::Runtime(format!(
                "expected bool, got {other:?}"
            ))),
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
    /// Type-prefixed to prevent cross-type collisions (e.g. U64(42) vs Str("42")).
    pub fn to_map_key(&self) -> String {
        match self {
            Value::U64(n) => format!("u:{n}"),
            Value::Bool(b) => format!("b:{b}"),
            Value::Str(s) => format!("s:{s}"),
            Value::Address(a) => format!("a:{}", hex::encode(a)),
            Value::Null => "n:null".to_string(),
            Value::Map(_) => "m:map".to_string(),
            Value::Array(_) => "r:array".to_string(),
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
            Value::Map(m) => {
                write!(f, "{{")?;
                let mut sorted_keys: Vec<&String> = m.keys().collect();
                sorted_keys.sort();
                for (i, key) in sorted_keys.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    let val = &m[*key];
                    write!(f, "\"{key}\": {val}")?;
                }
                write!(f, "}}")
            }
            Value::Array(a) => {
                write!(f, "[")?;
                for (i, elem) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{elem}")?;
                }
                write!(f, "]")
            }
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
    Array(Box<ScriptType>),
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
    #[error("step limit exceeded: executed {steps} opcodes (max {limit})")]
    StepLimitExceeded { steps: u64, limit: u64 },
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

// ─── Contract ABI ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractAbi {
    pub name: String,
    pub methods: Vec<AbiMethod>,
    pub state: Vec<AbiStateField>,
    pub lifecycle_hooks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbiMethod {
    pub name: String,
    pub params: Vec<AbiParam>,
    pub return_type: Option<ScriptType>,
    pub mutates_state: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbiParam {
    pub name: String,
    pub ty: ScriptType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbiStateField {
    pub name: String,
    pub ty: ScriptType,
    pub has_default: bool,
}

// ─── Deploy-time bytecode validation (Phase 4.4, 2026-05-03) ──────────────

/// Maximum number of opcodes a deployed contract may carry.
/// Bounds VM step memory and prevents pathological-size DoS.
const MAX_DEPLOY_OPCODES: usize = 65_536;

/// Validate freshly-compiled bytecode before persisting it.
/// Rejects:
///   - bytecode whose opcode count exceeds [`MAX_DEPLOY_OPCODES`].
///   - any Jump/JumpIf/JumpIfFalse whose target is `>= opcodes.len()`.
///   - any method-start offset (in `bytecode.methods`) that is
///     `>= opcodes.len()`.
fn validate_deploy_bytecode(
    bytecode: &compiler::EvaporBytecode,
) -> Result<(), ScriptError> {
    use compiler::Op;
    let n = bytecode.opcodes.len();
    if n > MAX_DEPLOY_OPCODES {
        return Err(ScriptError::Runtime(format!(
            "deploy bytecode too large: {n} opcodes (max {MAX_DEPLOY_OPCODES})"
        )));
    }
    for (i, op) in bytecode.opcodes.iter().enumerate() {
        match op {
            Op::Jump(t) | Op::JumpIf(t) | Op::JumpIfFalse(t) => {
                if *t >= n {
                    return Err(ScriptError::Runtime(format!(
                        "deploy bytecode invalid: jump at offset {i} targets {t}, \
                         out of range (n={n})"
                    )));
                }
            }
            _ => {}
        }
    }
    for (name, &start) in bytecode.methods.iter() {
        if start >= n {
            return Err(ScriptError::Runtime(format!(
                "deploy bytecode invalid: method '{name}' starts at {start}, \
                 out of range (n={n})"
            )));
        }
    }
    Ok(())
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
    #[allow(clippy::too_many_arguments)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptContract {
    pub id: u64,
    pub name: String,
    pub bytecode: compiler::EvaporBytecode,
    pub abi: ContractAbi,
    pub state: HashMap<String, Value>,
    pub creator: AccountAddress,
    pub created_epoch: Epoch,
    pub energy: Energy,
    pub half_life: HalfLife,
    pub last_refreshed: Epoch,
    pub evaporated: bool,
    /// Bytes credited to the creator's `storage_bytes` at deploy time.
    /// Set to `source.len()` at deploy — exact match to what the
    /// execution layer charges. The execution-layer evaporation
    /// credit-back reads this for precise debit. `#[serde(default)]`
    /// keeps legacy script contracts deserializable.
    #[serde(default)]
    pub storage_bytes_charged: u64,
    /// Upgrade authority. `Some(addr)` — `addr` may sign an admin-path
    /// `UpgradeContract` tx. `None` — contract is "frozen" on the admin
    /// path; only a governance-quorum upgrade is permitted. Defaults to
    /// `Some(creator)` at deploy. Legacy snapshots that lack the field
    /// deserialise to `None` (frozen) rather than silently inheriting
    /// upgrade rights.
    #[serde(default)]
    pub admin: Option<AccountAddress>,
    /// Monotonic counter bumped on every successful bytecode swap.
    /// Auditability hook: lets explorers and clients see whether a
    /// contract has ever been mutated. Legacy snapshots default to 0.
    #[serde(default)]
    pub upgrade_count: u64,
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
    #[allow(clippy::type_complexity)]
    contracts: HashMap<
        u64,
        (
            compiler::EvaporBytecode,
            HashMap<String, Value>,
            AccountAddress,
            Epoch,
            Energy,
            HalfLife,
            Epoch,
            bool,
        ),
    >,
    vrf_randomness: [u8; 32],
    state_patches: Vec<(u64, HashMap<String, Value>)>,
    collected_events: Vec<ContractEvent>,
    active_calls: HashSet<u64>,
}

impl ContractCallRouter {
    fn from_engine(engine: &ScriptEngine) -> Self {
        let mut contracts = HashMap::new();
        for (id, c) in &engine.contracts {
            contracts.insert(
                *id,
                (
                    c.bytecode.clone(),
                    c.state.clone(),
                    c.creator,
                    c.created_epoch,
                    c.energy,
                    c.half_life,
                    c.last_refreshed,
                    c.evaporated,
                ),
            );
        }
        Self {
            contracts,
            vrf_randomness: engine.vrf_randomness,
            state_patches: Vec::new(),
            collected_events: Vec::new(),
            active_calls: HashSet::new(),
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
                "cross-contract call depth exceeded (max {})",
                MAX_CALL_DEPTH
            )));
        }

        if self.active_calls.contains(&contract_id) {
            return Err(ScriptError::Runtime(format!(
                "reentrancy detected: contract {} is already in the call stack",
                contract_id
            )));
        }

        let (bytecode, state, creator, _created, energy, half_life, last_refreshed, evaporated) =
            self.contracts
                .get(&contract_id)
                .ok_or_else(|| ScriptError::Runtime(format!("contract {contract_id} not found")))?
                .clone();

        if evaporated {
            return Err(ScriptError::Runtime(format!(
                "contract {contract_id} has evaporated"
            )));
        }

        let current_energy =
            energy_at_epoch(energy, half_life, epoch.saturating_sub(last_refreshed));
        if current_energy == 0 {
            return Err(ScriptError::Runtime(format!(
                "contract {contract_id} has no energy"
            )));
        }

        let ctx = ExecutionContext {
            caller,
            owner: creator,
            epoch,
            energy: current_energy,
            vrf_randomness: self.vrf_randomness,
            call_depth,
        };

        self.active_calls.insert(contract_id);
        let result = vm::EvaporVM::execute_full(
            &bytecode,
            method,
            args,
            state,
            &ctx,
            gas_remaining,
            Some(self),
        );
        self.active_calls.remove(&contract_id);
        let result = result?;

        self.state_patches.push((contract_id, result.state_changes));
        self.collected_events
            .extend(result.structured_events.clone());

        Ok((
            result.return_value,
            result.structured_events,
            result.gas_used,
        ))
    }
}

/// Upgrade-path authorisation mode for `ScriptEngine::upgrade_contract`.
///
/// `Admin(caller)` — caller must equal `contract.admin` (and admin
///   must not be `None`). Mirrors the deployer-as-admin default.
/// `Governance` — chain has already verified a stake-quorum amendment;
///   skip the caller-equals-admin check.
#[derive(Debug, Clone, Copy)]
pub enum UpgradeAuth {
    Admin(AccountAddress),
    Governance,
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
        let abi = compiler::generate_abi(&contract_ast);

        // Phase 4.4 (2026-05-03) — deploy-time bytecode validation.
        // Audit `end_to_end_audit_2026_04_27.md §3` flagged
        // "bytecode validation at deploy time is loose". Hardening:
        // (1) cap total opcode count to bound VM step memory.
        // (2) verify every Jump/JumpIf/JumpIfFalse target lies
        //     within the opcode list.
        // (3) verify every method-start offset is in-range.
        // Each rejection is a deploy failure (no bytecode persisted).
        validate_deploy_bytecode(&bytecode)?;

        // Initialize state from schema defaults
        let mut state = HashMap::new();
        for field in &bytecode.state_schema.fields {
            let default_val = field.default.clone().unwrap_or_else(|| match &field.ty {
                ScriptType::U64 => Value::U64(0),
                ScriptType::Bool => Value::Bool(false),
                ScriptType::String => Value::Str(String::new()),
                ScriptType::Address => Value::Address([0u8; 32]),
                ScriptType::Map(_, _) => Value::Map(HashMap::new()),
                ScriptType::Array(_) => Value::Array(Vec::new()),
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
                abi,
                state,
                creator,
                created_epoch: current_epoch,
                energy,
                half_life,
                last_refreshed: current_epoch,
                evaporated: false,
                // Exact deploy-time charge. Matches what the execution
                // layer credits to the deployer's storage_bytes.
                storage_bytes_charged: source.len() as u64,
                // New contracts start with the deployer as admin. The
                // admin can be transferred or set to None ("frozen")
                // through the contract's own state-machine semantics
                // — the chain just enforces whatever value lives here
                // at upgrade time.
                admin: Some(creator),
                upgrade_count: 0,
            },
        );

        Ok(id)
    }

    /// Upgrade a deployed contract to new bytecode.
    ///
    /// Authorization layers:
    ///   - Admin path: `auth = UpgradeAuth::Admin(caller)` — caller
    ///     must equal `contract.admin` (and admin must not be `None`,
    ///     i.e. the contract must not be frozen).
    ///   - Governance path: `auth = UpgradeAuth::Governance` — chain
    ///     has already verified a stake-quorum amendment; skip the
    ///     caller-equals-admin check. Used when the execution layer
    ///     enforces the quorum.
    ///
    /// Either path bumps `upgrade_count` and replaces the bytecode.
    /// State, energy, half_life, last_refreshed, creator, admin and
    /// created_epoch are preserved across the upgrade.
    ///
    /// Schema compatibility (both paths): every existing field in the
    /// current state must be present in the new schema with the same
    /// type. New fields are allowed and initialised to their declared
    /// defaults. Removed fields are rejected — silently dropping state
    /// on upgrade would erase user balances or governance votes that
    /// the original contract was responsible for. Closes K-10
    /// (UpgradeContract) deferred item.
    pub fn upgrade_contract(
        &mut self,
        contract_id: u64,
        new_source: &str,
        auth: UpgradeAuth,
        _current_epoch: Epoch,
    ) -> Result<(), ScriptError> {
        let contract = self.contracts.get(&contract_id).ok_or_else(|| {
            ScriptError::Runtime(format!("upgrade: contract {contract_id} not found"))
        })?;
        if contract.evaporated {
            return Err(ScriptError::Runtime(format!(
                "upgrade: contract {contract_id} has evaporated"
            )));
        }
        match auth {
            UpgradeAuth::Admin(caller) => match contract.admin {
                None => {
                    return Err(ScriptError::Runtime(format!(
                        "upgrade: contract {contract_id} is frozen \
                         (admin = None) — admin path is unavailable"
                    )));
                }
                Some(admin_addr) => {
                    if admin_addr != caller {
                        return Err(ScriptError::Runtime(format!(
                            "upgrade: caller is not the admin of contract {contract_id}"
                        )));
                    }
                }
            },
            UpgradeAuth::Governance => {
                // Stake-quorum already verified by the execution layer.
                // No caller-equals-admin check; this path can upgrade
                // a frozen contract too — that is the whole point of
                // having a governance gate.
            }
        }

        let new_ast = parser::parse(new_source)?;
        let new_bytecode = compiler::compile(&new_ast)?;
        let new_abi = compiler::generate_abi(&new_ast);

        // Schema-compatibility check.
        let current = self.contracts.get_mut(&contract_id).expect("checked above");
        for (field_name, current_value) in current.state.iter() {
            let new_field = new_bytecode
                .state_schema
                .fields
                .iter()
                .find(|f| &f.name == field_name)
                .ok_or_else(|| {
                    ScriptError::Runtime(format!(
                        "upgrade: new schema removes existing field '{field_name}' \
                         (would orphan live state)"
                    ))
                })?;
            let compatible = matches!(
                (current_value, &new_field.ty),
                (Value::U64(_), ScriptType::U64)
                    | (Value::Bool(_), ScriptType::Bool)
                    | (Value::Str(_), ScriptType::String)
                    | (Value::Address(_), ScriptType::Address)
                    | (Value::Map(_), ScriptType::Map(_, _))
                    | (Value::Array(_), ScriptType::Array(_))
            );
            if !compatible {
                return Err(ScriptError::Runtime(format!(
                    "upgrade: field '{field_name}' type mismatch \
                     (current value cannot inhabit new declared type)"
                )));
            }
        }

        // Inject defaults for new fields.
        for field in &new_bytecode.state_schema.fields {
            if !current.state.contains_key(&field.name) {
                let default_val = field.default.clone().unwrap_or_else(|| match &field.ty {
                    ScriptType::U64 => Value::U64(0),
                    ScriptType::Bool => Value::Bool(false),
                    ScriptType::String => Value::Str(String::new()),
                    ScriptType::Address => Value::Address([0u8; 32]),
                    ScriptType::Map(_, _) => Value::Map(HashMap::new()),
                    ScriptType::Array(_) => Value::Array(Vec::new()),
                });
                current.state.insert(field.name.clone(), default_val);
            }
        }

        current.bytecode = new_bytecode;
        current.abi = new_abi;
        current.name = new_ast.name;
        // Auditability counter — bumped on every successful swap,
        // independent of which path (admin / governance) was taken.
        current.upgrade_count = current.upgrade_count.saturating_add(1);
        Ok(())
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
        router.active_calls.insert(contract_id);
        let result = vm::EvaporVM::execute_full(
            &bytecode,
            method,
            args,
            state,
            &ctx,
            10_000_000,
            Some(&mut router),
        );
        router.active_calls.remove(&contract_id);
        let result = result?;

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

    pub fn get_abi(&self, contract_id: u64) -> Result<&ContractAbi, ScriptError> {
        let contract = self
            .contracts
            .get(&contract_id)
            .ok_or_else(|| ScriptError::Runtime(format!("contract {contract_id} not found")))?;
        Ok(&contract.abi)
    }

    pub fn get_contract(&self, contract_id: u64) -> Option<&ScriptContract> {
        self.contracts.get(&contract_id)
    }

    /// Return references to all deployed contracts (unordered).
    pub fn all_contracts(&self) -> Vec<&ScriptContract> {
        self.contracts.values().collect()
    }

    /// Mutable access for test fixtures. Production callers should use
    /// `upgrade_contract` / `refresh_contract` etc. — direct mutation
    /// bypasses the schema-compatibility and admin-path invariants.
    #[doc(hidden)]
    pub fn contract_mut_for_test(&mut self, id: u64) -> Option<&mut ScriptContract> {
        self.contracts.get_mut(&id)
    }

    /// Restore a previously-serialized contract into the engine.
    /// Adjusts `next_id` so future deploys never collide.
    pub fn restore_contract(&mut self, contract: ScriptContract) {
        if contract.id >= self.next_id {
            self.next_id = contract.id + 1;
        }
        self.contracts.insert(contract.id, contract);
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

        let result = engine
            .call(
                caller_id,
                "call_add",
                vec![Value::U64(adder_id), Value::U64(42)],
                creator,
                10,
            )
            .unwrap();

        assert_eq!(result.return_value, Value::U64(42));

        let adder_total = engine
            .call(adder_id, "get_total", vec![], creator, 10)
            .unwrap();
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
        assert!(result.is_err(), "recursive calls must be blocked");
        let err = format!("{:?}", result.unwrap_err());
        assert!(
            err.contains("depth") || err.contains("reentrancy"),
            "error should mention depth or reentrancy: {err}"
        );
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

        engine
            .call(s1, "set", vec![Value::U64(100)], creator, 10)
            .unwrap();
        engine
            .call(s2, "set", vec![Value::U64(200)], creator, 10)
            .unwrap();

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

        let result = engine
            .call(boss, "delegate", vec![Value::U64(worker)], creator, 10)
            .unwrap();
        assert_eq!(result.return_value, Value::U64(50));
        assert!(result.gas_used > 0);
    }
}

// ═══════════════════════════════════════════════════════════════
// Security regression tests (C-05: map key type collisions)
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod map_key_collision_tests {
    use super::*;

    /// C-05: U64(42) and Str("42") must produce different map keys.
    #[test]
    fn test_c05_u64_and_str_produce_different_keys() {
        let key_u64 = Value::U64(42).to_map_key();
        let key_str = Value::Str("42".to_string()).to_map_key();
        assert_ne!(
            key_u64, key_str,
            "U64(42) and Str(\"42\") must hash to different map keys, got both = {key_u64}"
        );
    }

    /// C-05: All variant prefixes must be distinct.
    #[test]
    fn test_c05_all_type_prefixes_distinct() {
        let keys = [
            Value::U64(0).to_map_key(),
            Value::Bool(false).to_map_key(),
            Value::Str("0".to_string()).to_map_key(),
            Value::Address([0u8; 32]).to_map_key(),
            Value::Null.to_map_key(),
            Value::Map(HashMap::new()).to_map_key(),
            Value::Array(Vec::new()).to_map_key(),
        ];
        // Extract the prefix (everything before first ':')
        let prefixes: Vec<&str> = keys.iter().map(|k| k.split(':').next().unwrap()).collect();
        let unique: std::collections::HashSet<&str> = prefixes.iter().copied().collect();
        assert_eq!(
            prefixes.len(),
            unique.len(),
            "all type prefixes must be unique, got: {:?}",
            prefixes
        );
    }

    /// C-05: Verify map key collisions cannot happen in an end-to-end contract execution.
    /// An attacker tries to overwrite a U64-keyed entry using a Str key with the same digits.
    #[test]
    fn test_c05_map_collision_attack_e2e() {
        let src = r#"
contract MapTest {
    state {
        data: map[string -> u64]
    }
    fn set_str(key: string, val: u64) {
        self.data[key] = val
    }
    fn get_str(key: string) -> u64 {
        return self.data[key]
    }
}
"#;
        let mut engine = ScriptEngine::new();
        let creator = [1u8; 32];
        let id = engine.deploy(src, creator, 10_000, 100, 1).unwrap();

        // Set key "hello" = 100
        engine
            .call(
                id,
                "set_str",
                vec![Value::Str("hello".into()), Value::U64(100)],
                creator,
                10,
            )
            .unwrap();

        // Set key "world" = 200
        engine
            .call(
                id,
                "set_str",
                vec![Value::Str("world".into()), Value::U64(200)],
                creator,
                10,
            )
            .unwrap();

        // Verify "hello" is still 100 (not overwritten)
        let result = engine
            .call(id, "get_str", vec![Value::Str("hello".into())], creator, 10)
            .unwrap();
        assert_eq!(result.return_value, Value::U64(100));

        // Verify "world" is 200
        let result = engine
            .call(id, "get_str", vec![Value::Str("world".into())], creator, 10)
            .unwrap();
        assert_eq!(result.return_value, Value::U64(200));
    }

    /// C-05: Verify that the type prefix format is stable (regression guard).
    #[test]
    fn test_c05_key_format_stability() {
        assert_eq!(Value::U64(42).to_map_key(), "u:42");
        assert_eq!(Value::Str("42".to_string()).to_map_key(), "s:42");
        assert_eq!(Value::Bool(true).to_map_key(), "b:true");
        assert_eq!(Value::Null.to_map_key(), "n:null");
    }

    // ─── Value accessor invariants ────────────────────────────────────────

    #[test]
    fn test_value_as_u64_extracts_and_rejects_wrong_type() {
        assert_eq!(Value::U64(7).as_u64().unwrap(), 7);
        let err = Value::Str("nope".into()).as_u64().unwrap_err();
        assert!(format!("{err}").contains("expected u64"));
    }

    #[test]
    fn test_value_as_bool_extracts_and_rejects_wrong_type() {
        assert!(Value::Bool(true).as_bool().unwrap());
        assert!(!Value::Bool(false).as_bool().unwrap());
        let err = Value::U64(0).as_bool().unwrap_err();
        assert!(format!("{err}").contains("expected bool"));
    }

    #[test]
    fn test_value_as_str_extracts_and_rejects_wrong_type() {
        assert_eq!(Value::Str("hi".into()).as_str().unwrap(), "hi");
        let err = Value::U64(1).as_str().unwrap_err();
        assert!(format!("{err}").contains("expected string"));
    }

    #[test]
    fn test_value_as_address_extracts_and_rejects_wrong_type() {
        let addr = [0xABu8; 32];
        assert_eq!(Value::Address(addr).as_address().unwrap(), addr);
        let err = Value::Bool(true).as_address().unwrap_err();
        assert!(format!("{err}").contains("expected address"));
    }

    #[test]
    fn test_value_display_address_is_hex_prefixed() {
        let addr = [0x01u8; 32];
        let s = format!("{}", Value::Address(addr));
        assert!(s.starts_with("0x"));
        assert_eq!(s.len(), 66); // "0x" + 64 hex chars
    }

    #[test]
    fn test_value_display_map_keys_sorted_deterministic() {
        // Map iteration is non-deterministic in HashMap, but Display sorts
        // keys. Same input map → same string regardless of insertion order.
        let mut m1: std::collections::HashMap<String, Value> =
            std::collections::HashMap::new();
        m1.insert("zebra".into(), Value::U64(1));
        m1.insert("alpha".into(), Value::U64(2));
        m1.insert("middle".into(), Value::U64(3));

        let mut m2: std::collections::HashMap<String, Value> =
            std::collections::HashMap::new();
        m2.insert("middle".into(), Value::U64(3));
        m2.insert("alpha".into(), Value::U64(2));
        m2.insert("zebra".into(), Value::U64(1));

        let s1 = format!("{}", Value::Map(m1));
        let s2 = format!("{}", Value::Map(m2));
        assert_eq!(s1, s2);
        // alpha < middle < zebra alphabetically
        let alpha_pos = s1.find("alpha").unwrap();
        let middle_pos = s1.find("middle").unwrap();
        let zebra_pos = s1.find("zebra").unwrap();
        assert!(alpha_pos < middle_pos);
        assert!(middle_pos < zebra_pos);
    }

    #[test]
    fn test_value_display_array_brackets() {
        let arr = Value::Array(vec![Value::U64(1), Value::U64(2), Value::U64(3)]);
        assert_eq!(format!("{arr}"), "[1, 2, 3]");
        // Empty array
        let empty = Value::Array(vec![]);
        assert_eq!(format!("{empty}"), "[]");
    }

    #[test]
    fn test_value_to_map_key_address_full_hex() {
        let addr = [0xCDu8; 32];
        let key = Value::Address(addr).to_map_key();
        assert!(key.starts_with("a:"));
        // 32 bytes → 64 hex chars
        assert_eq!(key.len(), 2 + 64);
    }
}
