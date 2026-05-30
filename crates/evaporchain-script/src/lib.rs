#[cfg(test)]
mod audit_tests;
pub mod compiler;
pub mod parser;
pub mod totality;
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
    ///
    /// SCR-N4 (audit 2026-05-15): Map and Array variants previously
    /// collapsed to the constant strings `"m:map"` / `"r:array"`, so
    /// any two Maps (or any two Arrays) used as map keys would alias
    /// to the same slot — silent collision, lost writes, wrong
    /// reads. Now hashed via BLAKE3 of the canonical `Display`
    /// representation (Map keys are already sorted in `Display`,
    /// making the form deterministic across runs).
    pub fn to_map_key(&self) -> String {
        match self {
            Value::U64(n) => format!("u:{n}"),
            Value::Bool(b) => format!("b:{b}"),
            Value::Str(s) => format!("s:{s}"),
            Value::Address(a) => format!("a:{}", hex::encode(a)),
            Value::Null => "n:null".to_string(),
            Value::Map(_) => format!(
                "m:{}",
                hex::encode(blake3::hash(self.to_string().as_bytes()).as_bytes())
            ),
            Value::Array(_) => format!(
                "r:{}",
                hex::encode(blake3::hash(self.to_string().as_bytes()).as_bytes())
            ),
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
fn validate_deploy_bytecode(bytecode: &compiler::EvaporBytecode) -> Result<(), ScriptError> {
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

/// SCR-N1 (audit 2026-05-15): derive a deterministic identity address
/// for a script contract from its u64 `contract_id`. Used by the VM
/// when invoking cross-contract calls so the callee sees its caller as
/// the *calling contract*, not the original EOA — without this, any
/// contract-level `require(caller == owner)` gate is bypassable by
/// routing through an intermediate contract.
///
/// DST `EVAPORCHAIN_V1_SCRIPT_CONTRACT_ID\0` distinguishes this hash
/// from every other AccountAddress derivation on the chain so a
/// contract's identity can't collide with any EOA-style address.
pub const CONTRACT_ID_DST: &[u8] = b"EVAPORCHAIN_V1_SCRIPT_CONTRACT_ID\0";

/// Derive the 32-byte identity address for a script contract by hashing
/// its u64 id under the contract-id DST.
pub fn contract_address(contract_id: u64) -> AccountAddress {
    let mut preimage = Vec::with_capacity(CONTRACT_ID_DST.len() + 8);
    preimage.extend_from_slice(CONTRACT_ID_DST);
    preimage.extend_from_slice(&contract_id.to_le_bytes());
    *blake3::hash(&preimage).as_bytes()
}

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
        // SCR-N1: tag the inner VM with `executing_contract_id =
        // contract_id` so any further nested call from this contract
        // passes `contract_address(contract_id)` as the callee's
        // caller — not `caller` (which is the parent's identity).
        let result = vm::EvaporVM::execute_full_with_self(
            &bytecode,
            method,
            args,
            state,
            &ctx,
            gas_remaining,
            Some(self),
            contract_id,
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

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "ScriptEngine deploys EvaporScript contracts with
    /// deploy-time bytecode validation: every Jump/JumpIf target must
    /// lie within the opcode list, opcode count is capped, and every
    /// method-start offset must be in range. A malformed bytecode
    /// rejects at deploy with no contract persisted."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let mut engine = ScriptEngine::new();
        let creator: AccountAddress = [1u8; 32];

        // Honest deploy: tiny contract with one method.
        let source = "\
contract HelloPress {
    state {
        counter: u64 = 0
    }

    fn bump(by: u64) {
        self.counter = self.counter + by
    }
}
";
        let id = engine.deploy(source, creator, 1_000, 100, 0).unwrap();
        assert_eq!(id, 1);

        // Repeat deploy → fresh id, both contracts coexist.
        let id2 = engine.deploy(source, creator, 1_000, 100, 0).unwrap();
        assert_eq!(id2, 2);
        assert_ne!(id, id2);

        // Malformed source → ScriptError, no contract persisted.
        let bad = "contract Broken { state { x: u64 method } }";
        assert!(engine.deploy(bad, creator, 1_000, 100, 0).is_err());

        // Out-of-range jump in hand-crafted bytecode rejects at the
        // validator gate (deploy-time hardening).
        use compiler::{EvaporBytecode, Op};
        use std::collections::HashMap;
        let mut methods = HashMap::new();
        methods.insert("oops".to_string(), 0usize);
        let bytecode = EvaporBytecode {
            opcodes: vec![Op::Jump(999)], // off-the-end target
            methods,
            state_schema: StateSchema { fields: vec![] },
            name: "Bad".into(),
        };
        assert!(validate_deploy_bytecode(&bytecode).is_err());
    }
}

/// Engine managing all deployed script contracts.
#[derive(Clone)]
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

    /// Call a method on a deployed script contract using the VM's
    /// `DEFAULT_GAS_LIMIT` (10M). Backward-compat shim; production
    /// callers SHOULD use [`Self::call_with_vm_gas`] and pass an
    /// explicit budget tied to the tx-level gas the sender paid for.
    /// Otherwise the validator can be forced to do up to 10M units of
    /// VM work for a 50k-gas tx — a 200× economic asymmetry that an
    /// adversarial contract author can weaponise via pathological
    /// loops (bounded by `MAX_STEPS = 10M` but not by what was paid).
    pub fn call(
        &mut self,
        contract_id: u64,
        method: &str,
        args: Vec<Value>,
        caller: AccountAddress,
        current_epoch: Epoch,
    ) -> Result<ScriptCallResult, ScriptError> {
        self.call_with_vm_gas(
            contract_id,
            method,
            args,
            caller,
            current_epoch,
            vm::DEFAULT_GAS_LIMIT,
        )
    }

    /// Call a method on a deployed script contract with an explicit
    /// VM gas budget. Production tx-handling code derives `vm_gas_limit`
    /// from the tx-level gas (e.g. `GAS_CALL_SCRIPT * SCRIPT_VM_GAS_TX_RATIO`)
    /// so the VM cannot do more work than the sender paid for.
    /// Closes the AUDIT_2026_05_06.md H-08 economic-asymmetry concern.
    pub fn call_with_vm_gas(
        &mut self,
        contract_id: u64,
        method: &str,
        args: Vec<Value>,
        caller: AccountAddress,
        current_epoch: Epoch,
        vm_gas_limit: u64,
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
        // SCR-N1: top-level dispatch tags the VM with the contract_id
        // so cross-contract calls from this contract correctly
        // propagate the calling contract's identity (not the EOA).
        let result = vm::EvaporVM::execute_full_with_self(
            &bytecode,
            method,
            args,
            state,
            &ctx,
            vm_gas_limit,
            Some(&mut router),
            contract_id,
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

    /// SCR-N1 (audit 2026-05-15) regression: when contract A calls
    /// contract B, B's `caller` built-in MUST report A's identity
    /// address — NOT the original EOA that called A. Pre-fix, B saw
    /// `caller == EOA`, so any `require(caller == owner)` gate B
    /// has was bypassable by any user routing through A.
    #[test]
    fn scr_n1_cross_contract_caller_is_callee_identity_not_eoa() {
        let target_src = r#"
contract Target {
    state { last_caller: address }
    fn record() -> u64 {
        self.last_caller = caller()
        return 1
    }
}
"#;
        let proxy_src = r#"
contract Proxy {
    state {}
    fn pass_through(target: u64) -> u64 {
        return call_contract(target, "record", 0)
    }
}
"#;
        let mut engine = ScriptEngine::new();
        engine.vrf_randomness = [0u8; 32];

        let creator: AccountAddress = [0xCC; 32];
        let eoa: AccountAddress = [0xEE; 32]; // some random EOA

        let target_id = engine.deploy(target_src, creator, 1, 1000, 100).unwrap();
        let proxy_id = engine.deploy(proxy_src, creator, 1, 1000, 100).unwrap();

        // EOA calls Proxy.pass_through(target_id) → Proxy calls
        // Target.record(). Target.last_caller must be the proxy's
        // identity address, not the EOA.
        engine
            .call(
                proxy_id,
                "pass_through",
                vec![Value::U64(target_id)],
                eoa,
                10,
            )
            .unwrap();

        let target = engine.contracts.get(&target_id).unwrap();
        let last_caller = match target.state.get("last_caller").unwrap() {
            Value::Address(a) => *a,
            other => panic!("expected Address, got {other:?}"),
        };
        let expected_proxy_address = contract_address(proxy_id);
        assert_eq!(
            last_caller,
            expected_proxy_address,
            "SCR-N1: callee must see calling contract's identity \
             ({:x?}…), not the EOA ({:x?}…)",
            &expected_proxy_address[..4],
            &eoa[..4]
        );
        assert_ne!(
            last_caller, eoa,
            "SCR-N1: callee must NOT see the original EOA as caller"
        );
    }

    /// SCR-N1: `contract_address` is deterministic + collision-resistant
    /// across distinct contract ids.
    #[test]
    fn scr_n1_contract_address_distinct_per_id() {
        let a = contract_address(1);
        let b = contract_address(2);
        let c = contract_address(1);
        assert_eq!(a, c, "contract_address must be deterministic");
        assert_ne!(a, b, "distinct ids must map to distinct addresses");
        // DST prefix in the preimage means contract addresses can't
        // collide with a domainless `blake3(id_bytes)` derivation.
        let domainless: AccountAddress = *blake3::hash(&1u64.to_le_bytes()).as_bytes();
        assert_ne!(a, domainless, "DST must be mixed into the address");
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
        let mut m1: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
        m1.insert("zebra".into(), Value::U64(1));
        m1.insert("alpha".into(), Value::U64(2));
        m1.insert("middle".into(), Value::U64(3));

        let mut m2: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
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

    /// AUDIT_2026_05_06.md H-08 close — `call_with_vm_gas` enforces the
    /// caller-supplied budget. A pathological loop that would burn many
    /// VM steps under `call()` (DEFAULT_GAS_LIMIT = 10M) MUST fail
    /// under `call_with_vm_gas(.., tight)`. This pins the economic-
    /// asymmetry close: a validator can no longer be tricked into
    /// doing 10M units of VM work for a 50k-gas tx — the new
    /// production budget is `GAS_CALL_SCRIPT * SCRIPT_VM_GAS_TX_RATIO`
    /// = 1_000_000, a 10× tighter cap than the pre-fix default.
    #[test]
    fn call_with_vm_gas_enforces_caller_supplied_budget() {
        // Pathological contract — runs a while loop bounded by the
        // VM's MAX_LOOP_ITERATIONS (100_000) ceiling. Under a generous
        // VM gas budget the call completes; under a tight budget it
        // hits gas exhaustion before completing.
        let src = r#"
contract Burn {
    state { counter: u64 = 0 }
    fn run() -> u64 {
        while (self.counter < 99000) {
            self.counter = self.counter + 1
        }
        return self.counter
    }
}
"#;
        let mut engine = ScriptEngine::new();
        let creator = [1u8; 32];
        let id = engine.deploy(src, creator, 10_000, 100, 1).unwrap();

        // Tight budget — 50 VM-gas units. Cannot complete even one
        // loop iteration. Pre-fix the default 10M would have been
        // applied silently.
        let tight = engine.call_with_vm_gas(id, "run", vec![], creator, 10, 50);
        assert!(
            tight.is_err(),
            "tight VM gas budget must reject the pathological loop; \
             got Ok which means the bound wasn't enforced"
        );
        let err_msg = format!("{:?}", tight.unwrap_err());
        assert!(
            err_msg.contains("gas") || err_msg.contains("Gas"),
            "error must indicate gas exhaustion; got: {err_msg}"
        );

        // Reset the counter for the second call (state was already
        // mutated up to the gas-exhaustion point on the first attempt).
        // Using a fresh engine to keep the test crisp.
        let mut engine2 = ScriptEngine::new();
        let id2 = engine2.deploy(src, creator, 10_000, 100, 1).unwrap();

        // Generous budget — 5M VM-gas units. Loop should complete.
        let generous = engine2.call_with_vm_gas(id2, "run", vec![], creator, 10, 5_000_000);
        assert!(
            generous.is_ok(),
            "generous VM gas budget must let the loop complete; got {:?}",
            generous.err()
        );
        assert_eq!(generous.unwrap().return_value, Value::U64(99000));
    }

    /// Backward-compat: the no-budget `call()` shim still works (it
    /// passes `DEFAULT_GAS_LIMIT = 10M`). Existing tests that use
    /// `call()` continue to pass without changes — this commit doesn't
    /// break the call-site API.
    #[test]
    fn call_default_shim_still_uses_default_gas_limit() {
        let src = r#"
contract Tiny {
    state {}
    fn ping() -> u64 {
        return 42
    }
}
"#;
        let mut engine = ScriptEngine::new();
        let creator = [1u8; 32];
        let id = engine.deploy(src, creator, 10_000, 100, 1).unwrap();

        let result = engine.call(id, "ping", vec![], creator, 10);
        assert!(
            result.is_ok(),
            "default call must succeed; got {:?}",
            result.err()
        );
        assert_eq!(result.unwrap().return_value, Value::U64(42));
    }

    /// T1.20 — Value::Display for Map variant. Previously
    /// uncovered (lines 80-83).
    #[test]
    fn t1_20_value_display_map() {
        use std::collections::HashMap;
        let mut m = HashMap::new();
        m.insert("b".to_string(), Value::U64(2));
        m.insert("a".to_string(), Value::U64(1));
        let v = Value::Map(m);
        let s = format!("{v}");
        // Keys are sorted alphabetically.
        assert!(s.contains("a"));
        assert!(s.contains("b"));
        assert!(s.starts_with('{'));
        assert!(s.ends_with('}'));
    }

    /// T1.20 — Value::to_map_key for every variant (covers all
    /// arms in the to_map_key match, lines 63-71).
    #[test]
    fn t1_20_value_to_map_key_all_variants() {
        assert_eq!(Value::U64(42).to_map_key(), "u:42");
        assert_eq!(Value::Bool(true).to_map_key(), "b:true");
        assert_eq!(Value::Str("foo".into()).to_map_key(), "s:foo");
        let mut a = [0u8; 32];
        a[0] = 0xAB;
        assert!(Value::Address(a).to_map_key().starts_with("a:"));
        assert_eq!(Value::Null.to_map_key(), "n:null");
        // SCR-N4 (audit 2026-05-15): Map / Array variants now hash
        // their canonical Display form rather than collapsing to a
        // constant string. The key starts with the type tag and a
        // 64-char hex digest.
        let m_key = Value::Map(Default::default()).to_map_key();
        assert!(m_key.starts_with("m:") && m_key.len() == 2 + 64);
        let r_key = Value::Array(vec![]).to_map_key();
        assert!(r_key.starts_with("r:") && r_key.len() == 2 + 64);
    }

    /// SCR-N4 (audit 2026-05-15) regression: distinct Maps must
    /// produce distinct map keys (previously both collapsed to
    /// `"m:map"`).
    #[test]
    fn scr_n4_distinct_maps_produce_distinct_keys() {
        let mut a = HashMap::new();
        a.insert("k".to_string(), Value::U64(1));
        let mut b = HashMap::new();
        b.insert("k".to_string(), Value::U64(2));
        let ka = Value::Map(a).to_map_key();
        let kb = Value::Map(b).to_map_key();
        assert_ne!(
            ka, kb,
            "SCR-N4: Map(k=1) and Map(k=2) must produce different keys"
        );
    }

    /// SCR-N4: distinct Arrays must produce distinct map keys.
    #[test]
    fn scr_n4_distinct_arrays_produce_distinct_keys() {
        let a = Value::Array(vec![Value::U64(1)]);
        let b = Value::Array(vec![Value::U64(2)]);
        assert_ne!(
            a.to_map_key(),
            b.to_map_key(),
            "SCR-N4: Array([1]) and Array([2]) must produce different keys"
        );
    }
}

#[cfg(test)]
mod decay_access_pass_pilot {
    //! Reference-contract pilot: deploys
    //! `contracts/evaporscript/decay_access_pass.es` via `ScriptEngine`
    //! and exercises the decay-credential pattern on-chain — validity
    //! tracks the contract's energy lifecycle (the bare `energy`
    //! builtin), issuer-gated issue/revoke, holder-gated exercise.
    use super::*;

    const SRC: &str = include_str!("../../../contracts/evaporscript/decay_access_pass.es");

    fn issuer() -> AccountAddress {
        [0x11; 32]
    }
    fn holder() -> AccountAddress {
        [0x22; 32]
    }
    fn stranger() -> AccountAddress {
        [0x33; 32]
    }

    fn issue_args() -> Vec<Value> {
        vec![Value::Address(holder()), Value::U64(250_000)]
    }

    #[test]
    fn pass_is_totality_clean() {
        // Must pass the script_vm_mode=total deploy gate (no `while`).
        let ast = parser::parse(SRC).expect("parses");
        totality::check_total_contract(&ast).expect("totality-clean");
    }

    #[test]
    fn validity_tracks_energy_decay() {
        let mut engine = ScriptEngine::new();
        // strength 1_000_000, half-life 100, deployed at epoch 0.
        let id = engine.deploy(SRC, issuer(), 1_000_000, 100, 0).unwrap();
        engine.call(id, "issue", issue_args(), issuer(), 0).unwrap();

        // epoch 0: energy 1_000_000 >= floor 250_000 → valid.
        assert_eq!(
            engine.call(id, "is_valid", vec![], issuer(), 0).unwrap().return_value,
            Value::Bool(true)
        );
        // epoch 200 (two half-lives): energy == 250_000 == floor → valid.
        assert_eq!(
            engine.call(id, "is_valid", vec![], issuer(), 200).unwrap().return_value,
            Value::Bool(true)
        );
        // epoch 260: energy 175_000 < floor → invalid (decayed out).
        assert_eq!(
            engine.call(id, "is_valid", vec![], issuer(), 260).unwrap().return_value,
            Value::Bool(false)
        );
    }

    #[test]
    fn require_valid_is_holder_gated() {
        let mut engine = ScriptEngine::new();
        let id = engine.deploy(SRC, issuer(), 1_000_000, 100, 0).unwrap();
        engine.call(id, "issue", issue_args(), issuer(), 0).unwrap();
        // holder may exercise while valid.
        assert_eq!(
            engine.call(id, "require_valid", vec![], holder(), 0).unwrap().return_value,
            Value::Bool(true)
        );
        // a non-holder is rejected.
        assert!(engine.call(id, "require_valid", vec![], stranger(), 0).is_err());
    }

    #[test]
    fn revoke_is_terminal_and_issuer_only() {
        let mut engine = ScriptEngine::new();
        let id = engine.deploy(SRC, issuer(), 1_000_000, 100, 0).unwrap();
        engine.call(id, "issue", issue_args(), issuer(), 0).unwrap();
        // non-issuer cannot revoke.
        assert!(engine.call(id, "revoke", vec![], stranger(), 0).is_err());
        // issuer revokes → invalid even at full energy.
        engine.call(id, "revoke", vec![], issuer(), 0).unwrap();
        assert_eq!(
            engine.call(id, "is_valid", vec![], issuer(), 0).unwrap().return_value,
            Value::Bool(false)
        );
        // double revoke rejected (terminal).
        assert!(engine.call(id, "revoke", vec![], issuer(), 0).is_err());
    }

    #[test]
    fn issue_is_once_and_issuer_only() {
        let mut engine = ScriptEngine::new();
        let id = engine.deploy(SRC, issuer(), 1_000_000, 100, 0).unwrap();
        // non-issuer cannot issue.
        assert!(engine.call(id, "issue", issue_args(), stranger(), 0).is_err());
        engine.call(id, "issue", issue_args(), issuer(), 0).unwrap();
        // issuing twice rejected.
        assert!(engine.call(id, "issue", issue_args(), issuer(), 0).is_err());
    }
}

#[cfg(test)]
mod mortal_dao_pilot {
    //! Reference-contract pilot: deploys
    //! `contracts/evaporscript/mortal_dao.es` via `ScriptEngine` and
    //! exercises the four-decay-primitive composition.
    //!
    //!   decay-credential → membership staleness gate
    //!   decay-rate-limit → per-member proposal cap
    //!   decay-reputation → vote weight grows with participation
    //!   decay-quorum     → quorum tracks a running peak
    //!
    //! Together: a governance contract whose behaviour is impossible
    //! to express cleanly on a chain without per-contract energy decay.
    use super::*;

    const SRC: &str = include_str!("../../../contracts/evaporscript/mortal_dao.es");

    fn founder() -> AccountAddress {
        [0x11; 32]
    }
    fn alice() -> AccountAddress {
        [0x21; 32]
    }
    fn bob() -> AccountAddress {
        [0x22; 32]
    }
    fn stranger() -> AccountAddress {
        [0x33; 32]
    }

    fn deploy_with_members() -> (ScriptEngine, u64) {
        let mut engine = ScriptEngine::new();
        // strength 1_000_000, half-life 100 (slow decay, so the
        // contract's own energy doesn't expire mid-test).
        let id = engine.deploy(SRC, founder(), 1_000_000, 100, 0).unwrap();
        engine
            .call(
                id,
                "add_member",
                vec![Value::Address(alice())],
                founder(),
                0,
            )
            .unwrap();
        engine
            .call(id, "add_member", vec![Value::Address(bob())], founder(), 0)
            .unwrap();
        (engine, id)
    }

    #[test]
    fn dao_is_totality_clean() {
        // Must pass the V1 totality gate (no `while`).
        let ast = parser::parse(SRC).expect("parses");
        totality::check_total_contract(&ast).expect("totality-clean");
    }

    #[test]
    fn add_member_owner_only_and_no_duplicates() {
        let mut engine = ScriptEngine::new();
        let id = engine.deploy(SRC, founder(), 1_000_000, 100, 0).unwrap();
        // Non-owner cannot add.
        assert!(engine
            .call(
                id,
                "add_member",
                vec![Value::Address(alice())],
                stranger(),
                0
            )
            .is_err());
        // Owner adds successfully.
        engine
            .call(
                id,
                "add_member",
                vec![Value::Address(alice())],
                founder(),
                0,
            )
            .unwrap();
        // Double-add rejected.
        assert!(engine
            .call(
                id,
                "add_member",
                vec![Value::Address(alice())],
                founder(),
                0
            )
            .is_err());
        assert_eq!(
            engine
                .call(id, "member_count_now", vec![], founder(), 0)
                .unwrap()
                .return_value,
            Value::U64(1)
        );
    }

    #[test]
    fn open_proposal_blocked_by_stale_membership() {
        // decay-credential: alice joined at epoch 0, members[alice] = 1
        // (epoch + 1). freshness_window = 500, so the gate
        // `members[addr] + freshness > epoch` requires 1 + 500 > epoch,
        // i.e. fresh through epoch 500, stale at epoch 501.
        let (mut engine, id) = deploy_with_members();
        assert!(engine
            .call(
                id,
                "open_proposal",
                vec![Value::Str("p".to_string())],
                alice(),
                501
            )
            .is_err());
        engine
            .call(id, "refresh_membership", vec![], alice(), 501)
            .unwrap();
        // Refresh sets members[alice] = 502; fresh again
        // (502 + 500 == 1002 > 501).
        engine
            .call(
                id,
                "open_proposal",
                vec![Value::Str("p".to_string())],
                alice(),
                501,
            )
            .unwrap();
    }

    #[test]
    fn proposal_rate_limit_caps_at_three() {
        // decay-rate-limit: proposal_cap = 3. Alice opens + closes
        // three proposals; the fourth open is rejected until she
        // refreshes.
        let (mut engine, id) = deploy_with_members();
        for round in 0..3u64 {
            let e_open = round * 60;
            let e_close = e_open + 50;
            engine
                .call(
                    id,
                    "open_proposal",
                    vec![Value::Str("p".to_string())],
                    alice(),
                    e_open,
                )
                .unwrap();
            engine.call(id, "vote_for", vec![], alice(), e_open).unwrap();
            engine
                .call(id, "close_proposal", vec![], alice(), e_close)
                .unwrap();
        }
        // 4th open at epoch 180 — rate-limit fires (cap=3).
        assert!(engine
            .call(
                id,
                "open_proposal",
                vec![Value::Str("p".to_string())],
                alice(),
                180
            )
            .is_err());
        // Refresh resets the counter (and the freshness clock).
        engine
            .call(id, "refresh_membership", vec![], alice(), 180)
            .unwrap();
        engine
            .call(
                id,
                "open_proposal",
                vec![Value::Str("p".to_string())],
                alice(),
                180,
            )
            .unwrap();
    }

    #[test]
    fn vote_weight_grows_with_participation() {
        // decay-reputation: weight = participations + 1.
        let (mut engine, id) = deploy_with_members();
        // Initial weight = 1 (participations 0 + 1).
        assert_eq!(
            engine
                .call(
                    id,
                    "weight_of",
                    vec![Value::Address(alice())],
                    founder(),
                    0
                )
                .unwrap()
                .return_value,
            Value::U64(1)
        );
        // P1: alice votes for, weight = 1.
        engine
            .call(
                id,
                "open_proposal",
                vec![Value::Str("p1".to_string())],
                alice(),
                0,
            )
            .unwrap();
        engine.call(id, "vote_for", vec![], alice(), 0).unwrap();
        assert_eq!(
            engine
                .call(id, "for_count", vec![], founder(), 0)
                .unwrap()
                .return_value,
            Value::U64(1)
        );
        engine
            .call(id, "close_proposal", vec![], alice(), 50)
            .unwrap();
        // Post-P1: alice's weight should now be 2.
        assert_eq!(
            engine
                .call(
                    id,
                    "weight_of",
                    vec![Value::Address(alice())],
                    founder(),
                    50
                )
                .unwrap()
                .return_value,
            Value::U64(2)
        );
        // P2: same vote, but weight = 2.
        engine
            .call(
                id,
                "open_proposal",
                vec![Value::Str("p2".to_string())],
                alice(),
                60,
            )
            .unwrap();
        engine.call(id, "vote_for", vec![], alice(), 60).unwrap();
        assert_eq!(
            engine
                .call(id, "for_count", vec![], founder(), 60)
                .unwrap()
                .return_value,
            Value::U64(2)
        );
    }

    #[test]
    fn quorum_gates_against_running_peak() {
        // decay-quorum: observed_peak rises with engagement;
        // weight_collected * 2 >= observed_peak gates close.
        let (mut engine, id) = deploy_with_members();
        // P1: both vote, weight collected = 2 (1 + 1).
        engine
            .call(
                id,
                "open_proposal",
                vec![Value::Str("p1".to_string())],
                alice(),
                0,
            )
            .unwrap();
        engine.call(id, "vote_for", vec![], alice(), 0).unwrap();
        engine.call(id, "vote_for", vec![], bob(), 0).unwrap();
        engine
            .call(id, "close_proposal", vec![], alice(), 50)
            .unwrap();
        assert_eq!(
            engine
                .call(id, "peak", vec![], founder(), 50)
                .unwrap()
                .return_value,
            Value::U64(2)
        );
        assert_eq!(
            engine
                .call(id, "carried_total", vec![], founder(), 50)
                .unwrap()
                .return_value,
            Value::U64(1)
        );
        // P2: alice alone, weight = 2 (participations=1 + 1).
        // weight_collected = 2, prev_peak = 2 → peak unchanged at 2,
        // quorum 2*2=4 >= 2 passes.
        engine
            .call(
                id,
                "open_proposal",
                vec![Value::Str("p2".to_string())],
                alice(),
                60,
            )
            .unwrap();
        engine.call(id, "vote_for", vec![], alice(), 60).unwrap();
        engine
            .call(id, "close_proposal", vec![], alice(), 110)
            .unwrap();
        assert_eq!(
            engine
                .call(id, "carried_total", vec![], founder(), 110)
                .unwrap()
                .return_value,
            Value::U64(2)
        );
    }

    #[test]
    fn close_requires_voting_window_elapsed() {
        let (mut engine, id) = deploy_with_members();
        engine
            .call(
                id,
                "open_proposal",
                vec![Value::Str("p".to_string())],
                alice(),
                0,
            )
            .unwrap();
        engine.call(id, "vote_for", vec![], alice(), 0).unwrap();
        // voting_window = 50; epoch 49 < 0 + 50 → reject.
        assert!(engine
            .call(id, "close_proposal", vec![], alice(), 49)
            .is_err());
        // epoch 50 = exactly the window → accept.
        engine
            .call(id, "close_proposal", vec![], alice(), 50)
            .unwrap();
    }

    #[test]
    fn double_vote_rejected_on_same_proposal() {
        let (mut engine, id) = deploy_with_members();
        engine
            .call(
                id,
                "open_proposal",
                vec![Value::Str("p".to_string())],
                alice(),
                0,
            )
            .unwrap();
        engine.call(id, "vote_for", vec![], alice(), 0).unwrap();
        // Second vote on same proposal rejected.
        assert!(engine.call(id, "vote_for", vec![], alice(), 0).is_err());
        // Bob can still vote (different member).
        engine.call(id, "vote_against", vec![], bob(), 0).unwrap();
        // But Bob can't double-vote either.
        assert!(engine
            .call(id, "vote_against", vec![], bob(), 0)
            .is_err());
    }

    #[test]
    fn stranger_cannot_vote_or_propose() {
        let (mut engine, id) = deploy_with_members();
        // Open a proposal so the active slot exists.
        engine
            .call(
                id,
                "open_proposal",
                vec![Value::Str("p".to_string())],
                alice(),
                0,
            )
            .unwrap();
        // Non-member is rejected at every gate.
        assert!(engine
            .call(id, "vote_for", vec![], stranger(), 0)
            .is_err());
        assert!(engine
            .call(id, "refresh_membership", vec![], stranger(), 0)
            .is_err());
        // After close, stranger still can't open one.
        engine.call(id, "vote_for", vec![], alice(), 0).unwrap();
        engine
            .call(id, "close_proposal", vec![], alice(), 50)
            .unwrap();
        assert!(engine
            .call(
                id,
                "open_proposal",
                vec![Value::Str("hi".to_string())],
                stranger(),
                50
            )
            .is_err());
    }
}

#[cfg(test)]
mod bell_oracle_pilot {
    //! Reference-contract pilot: deploys
    //! `contracts/evaporscript/bell_oracle.es` via `ScriptEngine`
    //! and exercises the chain's only-on-EvaporChain primitive — the
    //! per-block Bell-CHSH S-value beacon. The contract structurally
    //! REJECTS sub-threshold readings (S ≤ 2.0 = classical, no
    //! Bell-inequality violation); only above-floor readings are
    //! stored, so the contract's accepted state is provably
    //! quantum-derived.
    use super::*;

    const SRC: &str = include_str!("../../../contracts/evaporscript/bell_oracle.es");

    fn operator() -> AccountAddress {
        [0x11; 32]
    }
    fn stranger() -> AccountAddress {
        [0x33; 32]
    }

    fn deploy_and_arm() -> (ScriptEngine, u64) {
        let mut engine = ScriptEngine::new();
        // strength 1_000_000, half-life 100, deployed at epoch 0.
        let id = engine.deploy(SRC, operator(), 1_000_000, 100, 0).unwrap();
        // Arm with max_age = 10 epochs (matches the catalogue default).
        engine
            .call(id, "arm", vec![Value::U64(10)], operator(), 0)
            .unwrap();
        (engine, id)
    }

    #[test]
    fn bell_oracle_is_totality_clean() {
        let ast = parser::parse(SRC).expect("parses");
        totality::check_total_contract(&ast).expect("totality-clean");
    }

    #[test]
    fn arm_is_owner_only_and_once() {
        let mut engine = ScriptEngine::new();
        let id = engine.deploy(SRC, operator(), 1_000_000, 100, 0).unwrap();
        // Stranger cannot arm.
        assert!(engine
            .call(id, "arm", vec![Value::U64(10)], stranger(), 0)
            .is_err());
        // Owner arms once successfully.
        engine
            .call(id, "arm", vec![Value::U64(10)], operator(), 0)
            .unwrap();
        // Second arm rejected (sealed).
        assert!(engine
            .call(id, "arm", vec![Value::U64(10)], operator(), 0)
            .is_err());
        // Arm with zero max_age rejected upfront.
        let id2 = engine.deploy(SRC, operator(), 1_000_000, 100, 0).unwrap();
        assert!(engine
            .call(id2, "arm", vec![Value::U64(0)], operator(), 0)
            .is_err());
    }

    #[test]
    fn submit_requires_arm() {
        let mut engine = ScriptEngine::new();
        let id = engine.deploy(SRC, operator(), 1_000_000, 100, 0).unwrap();
        // Submitting before arming reverts.
        assert!(engine
            .call(
                id,
                "submit_reading",
                vec![Value::U64(2828), Value::U64(1)],
                operator(),
                0
            )
            .is_err());
    }

    #[test]
    fn submit_is_owner_only() {
        let (mut engine, id) = deploy_and_arm();
        // Stranger cannot submit.
        assert!(engine
            .call(
                id,
                "submit_reading",
                vec![Value::U64(2828), Value::U64(1)],
                stranger(),
                0
            )
            .is_err());
        // Owner submits successfully.
        engine
            .call(
                id,
                "submit_reading",
                vec![Value::U64(2828), Value::U64(1)],
                operator(),
                0,
            )
            .unwrap();
        assert_eq!(
            engine
                .call(id, "accepted_total", vec![], operator(), 0)
                .unwrap()
                .return_value,
            Value::U64(1)
        );
    }

    #[test]
    fn reading_at_or_below_floor_rejected() {
        // The local-realism floor is exactly 2000 milli (S = 2.0).
        // Both classical (< 2000) and exactly-at-the-floor (= 2000)
        // readings are STRUCTURALLY REJECTED — the gate is strict.
        let (mut engine, id) = deploy_and_arm();
        // 1500 milli (S = 1.5, classical) — rejected but recorded.
        engine
            .call(
                id,
                "submit_reading",
                vec![Value::U64(1500), Value::U64(1)],
                operator(),
                0,
            )
            .unwrap();
        assert_eq!(
            engine
                .call(id, "rejected_below_floor", vec![], operator(), 0)
                .unwrap()
                .return_value,
            Value::U64(1)
        );
        // 2000 milli (S = 2.0, exactly at the classical maximum) —
        // also rejected (strict `>` check).
        engine
            .call(
                id,
                "submit_reading",
                vec![Value::U64(2000), Value::U64(2)],
                operator(),
                0,
            )
            .unwrap();
        assert_eq!(
            engine
                .call(id, "rejected_below_floor", vec![], operator(), 0)
                .unwrap()
                .return_value,
            Value::U64(2)
        );
        // Nothing accepted yet.
        assert_eq!(
            engine
                .call(id, "accepted_total", vec![], operator(), 0)
                .unwrap()
                .return_value,
            Value::U64(0)
        );
        // is_certified_now is FALSE — no accepted reading.
        assert_eq!(
            engine
                .call(id, "is_certified_now", vec![], operator(), 0)
                .unwrap()
                .return_value,
            Value::Bool(false)
        );
    }

    #[test]
    fn above_floor_reading_accepted_and_certifies() {
        let (mut engine, id) = deploy_and_arm();
        // 2828 milli ≈ S = 2√2, the Tsirelson bound — maximally
        // quantum. Should be accepted.
        engine
            .call(
                id,
                "submit_reading",
                vec![Value::U64(2828), Value::U64(7)],
                operator(),
                0,
            )
            .unwrap();
        // is_certified_now flips true.
        assert_eq!(
            engine
                .call(id, "is_certified_now", vec![], operator(), 0)
                .unwrap()
                .return_value,
            Value::Bool(true)
        );
        // Stored values match.
        assert_eq!(
            engine
                .call(id, "latest_s_milli_view", vec![], operator(), 0)
                .unwrap()
                .return_value,
            Value::U64(2828)
        );
        assert_eq!(
            engine
                .call(id, "last_height", vec![], operator(), 0)
                .unwrap()
                .return_value,
            Value::U64(7)
        );
    }

    #[test]
    fn stale_height_rejected() {
        let (mut engine, id) = deploy_and_arm();
        // Accept a reading at height 10.
        engine
            .call(
                id,
                "submit_reading",
                vec![Value::U64(2500), Value::U64(10)],
                operator(),
                0,
            )
            .unwrap();
        // Submitting at the same height bumps stale counter.
        engine
            .call(
                id,
                "submit_reading",
                vec![Value::U64(2500), Value::U64(10)],
                operator(),
                0,
            )
            .unwrap();
        // And submitting at a lower height also.
        engine
            .call(
                id,
                "submit_reading",
                vec![Value::U64(2700), Value::U64(5)],
                operator(),
                0,
            )
            .unwrap();
        assert_eq!(
            engine
                .call(id, "rejected_stale_height", vec![], operator(), 0)
                .unwrap()
                .return_value,
            Value::U64(2)
        );
        // Strict-greater height accepted.
        engine
            .call(
                id,
                "submit_reading",
                vec![Value::U64(2700), Value::U64(11)],
                operator(),
                0,
            )
            .unwrap();
        assert_eq!(
            engine
                .call(id, "accepted_total", vec![], operator(), 0)
                .unwrap()
                .return_value,
            Value::U64(2)
        );
    }

    #[test]
    fn freshness_window_expires() {
        // max_age = 10 epochs. Reading recorded at epoch 0 is fresh
        // through epoch 10 and stale at epoch 11.
        let (mut engine, id) = deploy_and_arm();
        engine
            .call(
                id,
                "submit_reading",
                vec![Value::U64(2500), Value::U64(1)],
                operator(),
                0,
            )
            .unwrap();
        // is_fresh at epoch 10 still true.
        assert_eq!(
            engine
                .call(id, "is_fresh", vec![], operator(), 10)
                .unwrap()
                .return_value,
            Value::Bool(true)
        );
        // At epoch 11, stale.
        assert_eq!(
            engine
                .call(id, "is_fresh", vec![], operator(), 11)
                .unwrap()
                .return_value,
            Value::Bool(false)
        );
        // is_certified_now also flips to false past the window.
        assert_eq!(
            engine
                .call(id, "is_certified_now", vec![], operator(), 11)
                .unwrap()
                .return_value,
            Value::Bool(false)
        );
    }

    #[test]
    fn views_have_correct_initial_state() {
        let (mut engine, id) = deploy_and_arm();
        // No reading yet — latest_s_milli_view reverts.
        assert!(engine
            .call(id, "latest_s_milli_view", vec![], operator(), 0)
            .is_err());
        // Counters all zero.
        assert_eq!(
            engine
                .call(id, "accepted_total", vec![], operator(), 0)
                .unwrap()
                .return_value,
            Value::U64(0)
        );
        assert_eq!(
            engine
                .call(id, "rejected_below_floor", vec![], operator(), 0)
                .unwrap()
                .return_value,
            Value::U64(0)
        );
        // Floor + max_age match deployed values.
        assert_eq!(
            engine
                .call(id, "floor", vec![], operator(), 0)
                .unwrap()
                .return_value,
            Value::U64(2000)
        );
        assert_eq!(
            engine
                .call(id, "max_age", vec![], operator(), 0)
                .unwrap()
                .return_value,
            Value::U64(10)
        );
        assert_eq!(
            engine
                .call(id, "is_armed", vec![], operator(), 0)
                .unwrap()
                .return_value,
            Value::Bool(true)
        );
    }
}

#[cfg(test)]
mod refresh_market_pilot {
    //! Reference-contract pilot: deploys
    //! `contracts/evaporscript/refresh_market.es` via `ScriptEngine`
    //! and exercises the chain's primary economic activity —
    //! quadratic-in-utilisation rent + eviction-on-stale-refresh.
    use super::*;

    const SRC: &str = include_str!("../../../contracts/evaporscript/refresh_market.es");

    fn operator() -> AccountAddress {
        [0x11; 32]
    }
    fn alice() -> AccountAddress {
        [0x21; 32]
    }
    fn bob() -> AccountAddress {
        [0x22; 32]
    }
    fn stranger() -> AccountAddress {
        [0x33; 32]
    }

    /// Deploy with capacity=10, base_rent=100, eviction_window=5.
    fn deploy_and_arm() -> (ScriptEngine, u64) {
        let mut engine = ScriptEngine::new();
        let id = engine.deploy(SRC, operator(), 1_000_000, 100, 0).unwrap();
        engine
            .call(
                id,
                "arm",
                vec![Value::U64(10), Value::U64(100), Value::U64(5)],
                operator(),
                0,
            )
            .unwrap();
        (engine, id)
    }

    #[test]
    fn refresh_market_is_totality_clean() {
        let ast = parser::parse(SRC).expect("parses");
        totality::check_total_contract(&ast).expect("totality-clean");
    }

    #[test]
    fn arm_owner_only_validates_inputs_one_shot() {
        let mut engine = ScriptEngine::new();
        let id = engine.deploy(SRC, operator(), 1_000_000, 100, 0).unwrap();
        // Stranger cannot arm.
        assert!(engine
            .call(
                id,
                "arm",
                vec![Value::U64(10), Value::U64(100), Value::U64(5)],
                stranger(),
                0
            )
            .is_err());
        // Capacity 0 rejected.
        assert!(engine
            .call(
                id,
                "arm",
                vec![Value::U64(0), Value::U64(100), Value::U64(5)],
                operator(),
                0
            )
            .is_err());
        // Eviction 0 rejected.
        assert!(engine
            .call(
                id,
                "arm",
                vec![Value::U64(10), Value::U64(100), Value::U64(0)],
                operator(),
                0
            )
            .is_err());
        // OK now.
        engine
            .call(
                id,
                "arm",
                vec![Value::U64(10), Value::U64(100), Value::U64(5)],
                operator(),
                0,
            )
            .unwrap();
        // Second arm rejected (sealed).
        assert!(engine
            .call(
                id,
                "arm",
                vec![Value::U64(20), Value::U64(50), Value::U64(3)],
                operator(),
                0
            )
            .is_err());
    }

    #[test]
    fn claim_blocked_until_armed() {
        let mut engine = ScriptEngine::new();
        let id = engine.deploy(SRC, operator(), 1_000_000, 100, 0).unwrap();
        // Pre-arm claim reverts.
        assert!(engine
            .call(id, "claim_slot", vec![], alice(), 0)
            .is_err());
    }

    #[test]
    fn claim_increments_used_marks_holder_blocks_double_claim() {
        let (mut engine, id) = deploy_and_arm();
        engine
            .call(id, "claim_slot", vec![], alice(), 0)
            .unwrap();
        // used_now bumped.
        assert_eq!(
            engine
                .call(id, "used", vec![], operator(), 0)
                .unwrap()
                .return_value,
            Value::U64(1)
        );
        // is_holder reflects.
        assert_eq!(
            engine
                .call(id, "is_holder", vec![Value::Address(alice())], operator(), 0)
                .unwrap()
                .return_value,
            Value::Bool(true)
        );
        // Bob isn't a holder yet.
        assert_eq!(
            engine
                .call(id, "is_holder", vec![Value::Address(bob())], operator(), 0)
                .unwrap()
                .return_value,
            Value::Bool(false)
        );
        // Double claim by alice rejected.
        assert!(engine
            .call(id, "claim_slot", vec![], alice(), 0)
            .is_err());
    }

    #[test]
    fn claim_rejected_at_capacity() {
        // capacity = 10. Fill it with 10 distinct callers, then 11th
        // is rejected.
        let (mut engine, id) = deploy_and_arm();
        for i in 0..10u8 {
            let mut addr: AccountAddress = [0u8; 32];
            addr[0] = 0xA0 | i;
            engine.call(id, "claim_slot", vec![], addr, 0).unwrap();
        }
        assert_eq!(
            engine
                .call(id, "used", vec![], operator(), 0)
                .unwrap()
                .return_value,
            Value::U64(10)
        );
        assert_eq!(
            engine
                .call(id, "slots_remaining", vec![], operator(), 0)
                .unwrap()
                .return_value,
            Value::U64(0)
        );
        // 11th distinct caller rejected.
        let mut overflow: AccountAddress = [0u8; 32];
        overflow[0] = 0xB0;
        assert!(engine
            .call(id, "claim_slot", vec![], overflow, 0)
            .is_err());
    }

    #[test]
    fn release_decrements_used_and_clears_holder() {
        let (mut engine, id) = deploy_and_arm();
        engine
            .call(id, "claim_slot", vec![], alice(), 0)
            .unwrap();
        engine
            .call(id, "release_slot", vec![], alice(), 0)
            .unwrap();
        assert_eq!(
            engine
                .call(id, "used", vec![], operator(), 0)
                .unwrap()
                .return_value,
            Value::U64(0)
        );
        assert_eq!(
            engine
                .call(id, "is_holder", vec![Value::Address(alice())], operator(), 0)
                .unwrap()
                .return_value,
            Value::Bool(false)
        );
        // Alice can re-claim after release.
        engine
            .call(id, "claim_slot", vec![], alice(), 1)
            .unwrap();
    }

    #[test]
    fn evict_only_after_eviction_window_elapsed() {
        // eviction_window = 5. Alice claims at epoch 0; her
        // last_refresh = 0 + 1 = 1. Evictable iff
        // epoch >= 1 + 5 = 6.
        let (mut engine, id) = deploy_and_arm();
        engine
            .call(id, "claim_slot", vec![], alice(), 0)
            .unwrap();
        // Epoch 5: NOT evictable yet (5 < 6).
        assert_eq!(
            engine
                .call(id, "is_evictable", vec![Value::Address(alice())], operator(), 5)
                .unwrap()
                .return_value,
            Value::Bool(false)
        );
        assert!(engine
            .call(id, "evict", vec![Value::Address(alice())], bob(), 5)
            .is_err());
        // Epoch 6: evictable (6 >= 6). Anyone can evict.
        assert_eq!(
            engine
                .call(id, "is_evictable", vec![Value::Address(alice())], operator(), 6)
                .unwrap()
                .return_value,
            Value::Bool(true)
        );
        engine
            .call(id, "evict", vec![Value::Address(alice())], bob(), 6)
            .unwrap();
        // After eviction: slot freed, counter bumped, alice no longer holds.
        assert_eq!(
            engine
                .call(id, "used", vec![], operator(), 6)
                .unwrap()
                .return_value,
            Value::U64(0)
        );
        assert_eq!(
            engine
                .call(id, "evictions_total", vec![], operator(), 6)
                .unwrap()
                .return_value,
            Value::U64(1)
        );
        assert_eq!(
            engine
                .call(id, "is_holder", vec![Value::Address(alice())], operator(), 6)
                .unwrap()
                .return_value,
            Value::Bool(false)
        );
    }

    #[test]
    fn refresh_resets_eviction_clock() {
        let (mut engine, id) = deploy_and_arm();
        engine
            .call(id, "claim_slot", vec![], alice(), 0)
            .unwrap();
        // At epoch 4 alice refreshes. last_refresh becomes 4+1=5,
        // so she's safe until epoch 10.
        engine
            .call(id, "refresh_slot", vec![], alice(), 4)
            .unwrap();
        // At epoch 9: still not evictable (9 < 5+5=10).
        assert_eq!(
            engine
                .call(id, "is_evictable", vec![Value::Address(alice())], operator(), 9)
                .unwrap()
                .return_value,
            Value::Bool(false)
        );
        // At epoch 10: now evictable.
        assert_eq!(
            engine
                .call(id, "is_evictable", vec![Value::Address(alice())], operator(), 10)
                .unwrap()
                .return_value,
            Value::Bool(true)
        );
    }

    #[test]
    fn current_rate_is_quadratic_in_used() {
        // base = 100, capacity = 10. Formula:
        //   rate = 100 * (used + 1)^2 / 100  =  (used + 1)^2
        let (mut engine, id) = deploy_and_arm();
        // used=0 → rate = 1
        assert_eq!(
            engine
                .call(id, "current_rate", vec![], operator(), 0)
                .unwrap()
                .return_value,
            Value::U64(1)
        );
        // rate_at_used(5) = 36
        assert_eq!(
            engine
                .call(id, "rate_at_used", vec![Value::U64(5)], operator(), 0)
                .unwrap()
                .return_value,
            Value::U64(36)
        );
        // rate_at_used(9) = 100 (cap-1, last claimable)
        assert_eq!(
            engine
                .call(id, "rate_at_used", vec![Value::U64(9)], operator(), 0)
                .unwrap()
                .return_value,
            Value::U64(100)
        );
        // After alice claims, used=1 → rate = 4
        engine
            .call(id, "claim_slot", vec![], alice(), 0)
            .unwrap();
        assert_eq!(
            engine
                .call(id, "current_rate", vec![], operator(), 0)
                .unwrap()
                .return_value,
            Value::U64(4)
        );
    }

    #[test]
    fn pre_arm_views_return_safe_defaults() {
        // Without arming, current_rate should return 0 (not panic on
        // capacity^2 = 0). slots_remaining likewise.
        let mut engine = ScriptEngine::new();
        let id = engine.deploy(SRC, operator(), 1_000_000, 100, 0).unwrap();
        assert_eq!(
            engine
                .call(id, "current_rate", vec![], operator(), 0)
                .unwrap()
                .return_value,
            Value::U64(0)
        );
        assert_eq!(
            engine
                .call(id, "slots_remaining", vec![], operator(), 0)
                .unwrap()
                .return_value,
            Value::U64(0)
        );
        assert_eq!(
            engine
                .call(id, "is_armed", vec![], operator(), 0)
                .unwrap()
                .return_value,
            Value::Bool(false)
        );
    }
}

#[cfg(test)]
mod witnessfit_pilot {
    //! Reference-contract pilot: deploys
    //! `contracts/evaporscript/witnessfit.es` via `ScriptEngine` and
    //! exercises the "graceful fade" streak doctrine — check-ins
    //! inside the half-life window grow the streak; outside it the
    //! current streak resets to 1 but max_streak is preserved; the
    //! boost stays available while current streak ≥ threshold_bp of
    //! max.
    use super::*;

    const SRC: &str = include_str!("../../../contracts/evaporscript/witnessfit.es");

    fn wearer() -> AccountAddress {
        [0x11; 32]
    }
    fn stranger() -> AccountAddress {
        [0x33; 32]
    }

    /// Deploy with energy=1_000_000, half_life=100 (the chain's
    /// decay half-life, NOT the streak's; the contract uses its own
    /// `state.half_life` field for the streak window).
    fn deploy() -> (ScriptEngine, u64) {
        let mut engine = ScriptEngine::new();
        let id = engine.deploy(SRC, wearer(), 1_000_000, 100, 0).unwrap();
        (engine, id)
    }

    #[test]
    fn witnessfit_is_totality_clean() {
        let ast = parser::parse(SRC).expect("parses");
        totality::check_total_contract(&ast).expect("totality-clean");
    }

    #[test]
    fn check_in_owner_only() {
        let (mut engine, id) = deploy();
        // Stranger cannot check in.
        assert!(engine.call(id, "check_in", vec![], stranger(), 0).is_err());
        // Wearer checks in.
        engine
            .call(id, "check_in", vec![], wearer(), 0)
            .unwrap();
        assert_eq!(
            engine
                .call(id, "current_streak", vec![], wearer(), 0)
                .unwrap()
                .return_value,
            Value::U64(1)
        );
    }

    #[test]
    fn double_checkin_same_epoch_rejected() {
        let (mut engine, id) = deploy();
        engine.call(id, "check_in", vec![], wearer(), 0).unwrap();
        // Same epoch → rejected.
        assert!(engine.call(id, "check_in", vec![], wearer(), 0).is_err());
        // But epoch+1 OK.
        engine.call(id, "check_in", vec![], wearer(), 1).unwrap();
        assert_eq!(
            engine
                .call(id, "current_streak", vec![], wearer(), 1)
                .unwrap()
                .return_value,
            Value::U64(2)
        );
    }

    #[test]
    fn streak_grows_within_window() {
        // half_life default = 7. Check in every 5 epochs (within window).
        let (mut engine, id) = deploy();
        for day in [0u64, 5, 10, 15, 20] {
            engine.call(id, "check_in", vec![], wearer(), day).unwrap();
        }
        assert_eq!(
            engine
                .call(id, "current_streak", vec![], wearer(), 20)
                .unwrap()
                .return_value,
            Value::U64(5)
        );
        assert_eq!(
            engine
                .call(id, "peak", vec![], wearer(), 20)
                .unwrap()
                .return_value,
            Value::U64(5)
        );
    }

    #[test]
    fn streak_resets_outside_window_but_peak_preserved() {
        // half_life = 7. Build a streak of 3, then skip past the window.
        let (mut engine, id) = deploy();
        engine.call(id, "check_in", vec![], wearer(), 0).unwrap();
        engine.call(id, "check_in", vec![], wearer(), 3).unwrap();
        engine.call(id, "check_in", vec![], wearer(), 6).unwrap();
        // Peak = 3.
        assert_eq!(
            engine
                .call(id, "peak", vec![], wearer(), 6)
                .unwrap()
                .return_value,
            Value::U64(3)
        );
        // Skip 30 epochs → way past 6+7=13 window.
        engine.call(id, "check_in", vec![], wearer(), 30).unwrap();
        // Streak reset to 1.
        assert_eq!(
            engine
                .call(id, "current_streak", vec![], wearer(), 30)
                .unwrap()
                .return_value,
            Value::U64(1)
        );
        // But peak preserved.
        assert_eq!(
            engine
                .call(id, "peak", vec![], wearer(), 30)
                .unwrap()
                .return_value,
            Value::U64(3)
        );
    }

    #[test]
    fn current_streak_decays_to_zero_outside_window_without_checkin() {
        // Build streak, then read the view at an epoch past the window.
        let (mut engine, id) = deploy();
        engine.call(id, "check_in", vec![], wearer(), 0).unwrap();
        engine.call(id, "check_in", vec![], wearer(), 5).unwrap();
        // At epoch 12 (inside 5+7=12 window — boundary inclusive): still 2.
        assert_eq!(
            engine
                .call(id, "current_streak", vec![], wearer(), 12)
                .unwrap()
                .return_value,
            Value::U64(2)
        );
        // At epoch 13 (past window): 0.
        assert_eq!(
            engine
                .call(id, "current_streak", vec![], wearer(), 13)
                .unwrap()
                .return_value,
            Value::U64(0)
        );
    }

    #[test]
    fn has_boost_at_threshold_and_below() {
        // boost_threshold_bp default = 5000 (50%). Build streak of 4 →
        // peak=4; need streak ≥ 2 (50% of 4) for boost.
        let (mut engine, id) = deploy();
        for d in [0u64, 3, 6, 9] {
            engine.call(id, "check_in", vec![], wearer(), d).unwrap();
        }
        // Streak = 4, peak = 4. 4*10000 >= 5000*4 = 40000 >= 20000 ✓ boost.
        assert_eq!(
            engine
                .call(id, "has_boost", vec![], wearer(), 9)
                .unwrap()
                .return_value,
            Value::Bool(true)
        );
        // Now skip the window: streak resets next check-in.
        engine.call(id, "check_in", vec![], wearer(), 50).unwrap();
        // streak=1, peak=4 → 1*10000=10000 vs 5000*4=20000 → 10000 < 20000 → NO boost.
        assert_eq!(
            engine
                .call(id, "has_boost", vec![], wearer(), 50)
                .unwrap()
                .return_value,
            Value::Bool(false)
        );
        // Build back up to streak=2: 2*10000=20000 vs 20000 → ≥ ✓ boost.
        engine.call(id, "check_in", vec![], wearer(), 51).unwrap();
        assert_eq!(
            engine
                .call(id, "has_boost", vec![], wearer(), 51)
                .unwrap()
                .return_value,
            Value::Bool(true)
        );
    }

    #[test]
    fn window_remaining_counts_down() {
        let (mut engine, id) = deploy();
        engine.call(id, "check_in", vec![], wearer(), 0).unwrap();
        // half_life = 7 → window expires at epoch 7.
        // At epoch 0: remaining = 0 + 7 - 0 = 7.
        assert_eq!(
            engine
                .call(id, "window_remaining", vec![], wearer(), 0)
                .unwrap()
                .return_value,
            Value::U64(7)
        );
        // At epoch 5: remaining = 2.
        assert_eq!(
            engine
                .call(id, "window_remaining", vec![], wearer(), 5)
                .unwrap()
                .return_value,
            Value::U64(2)
        );
        // At epoch 7 (boundary): 0.
        assert_eq!(
            engine
                .call(id, "window_remaining", vec![], wearer(), 7)
                .unwrap()
                .return_value,
            Value::U64(0)
        );
        // Past window: 0.
        assert_eq!(
            engine
                .call(id, "window_remaining", vec![], wearer(), 8)
                .unwrap()
                .return_value,
            Value::U64(0)
        );
    }

    #[test]
    fn reset_peak_starts_a_new_chapter() {
        let (mut engine, id) = deploy();
        // Build peak of 5.
        for d in [0u64, 1, 2, 3, 4] {
            engine.call(id, "check_in", vec![], wearer(), d).unwrap();
        }
        assert_eq!(
            engine
                .call(id, "peak", vec![], wearer(), 4)
                .unwrap()
                .return_value,
            Value::U64(5)
        );
        // Reset peak → new peak = current streak (5).
        engine.call(id, "reset_peak", vec![], wearer(), 4).unwrap();
        assert_eq!(
            engine
                .call(id, "peak", vec![], wearer(), 4)
                .unwrap()
                .return_value,
            Value::U64(5)
        );
        // Stranger cannot reset.
        assert!(engine
            .call(id, "reset_peak", vec![], stranger(), 4)
            .is_err());
    }

    #[test]
    fn pre_checkin_views_return_zeros() {
        let (mut engine, id) = deploy();
        assert_eq!(
            engine
                .call(id, "current_streak", vec![], wearer(), 100)
                .unwrap()
                .return_value,
            Value::U64(0)
        );
        assert_eq!(
            engine
                .call(id, "peak", vec![], wearer(), 100)
                .unwrap()
                .return_value,
            Value::U64(0)
        );
        assert_eq!(
            engine
                .call(id, "has_boost", vec![], wearer(), 100)
                .unwrap()
                .return_value,
            Value::Bool(false)
        );
        assert_eq!(
            engine
                .call(id, "window_remaining", vec![], wearer(), 100)
                .unwrap()
                .return_value,
            Value::U64(0)
        );
    }
}

#[cfg(test)]
mod mayfly_pilot {
    //! Reference-contract pilot: deploys
    //! `contracts/evaporscript/mayfly.es` via `ScriptEngine` and
    //! exercises the doctrine-purest NFT — hatch + transfer +
    //! metadata read, with the contract's own energy as the
    //! lifespan.
    use super::*;

    const SRC: &str = include_str!("../../../contracts/evaporscript/mayfly.es");

    fn minter() -> AccountAddress {
        [0x11; 32]
    }
    fn alice() -> AccountAddress {
        [0x21; 32]
    }
    fn bob() -> AccountAddress {
        [0x22; 32]
    }
    fn stranger() -> AccountAddress {
        [0x33; 32]
    }

    #[test]
    fn mayfly_is_totality_clean() {
        let ast = parser::parse(SRC).expect("parses");
        totality::check_total_contract(&ast).expect("totality-clean");
    }

    #[test]
    fn hatch_owner_only_one_shot() {
        let mut engine = ScriptEngine::new();
        // strength 1000, half-life 10 — the mayfly catalogue defaults.
        let id = engine.deploy(SRC, minter(), 1000, 10, 0).unwrap();
        // Stranger cannot hatch.
        assert!(engine
            .call(
                id,
                "hatch",
                vec![Value::Str("brief life".to_string())],
                stranger(),
                0
            )
            .is_err());
        // Minter hatches successfully.
        engine
            .call(
                id,
                "hatch",
                vec![Value::Str("brief life".to_string())],
                minter(),
                0,
            )
            .unwrap();
        // Second hatch rejected.
        assert!(engine
            .call(
                id,
                "hatch",
                vec![Value::Str("again".to_string())],
                minter(),
                0
            )
            .is_err());
        // is_hatched flips true.
        assert_eq!(
            engine
                .call(id, "is_hatched", vec![], minter(), 0)
                .unwrap()
                .return_value,
            Value::Bool(true)
        );
    }

    #[test]
    fn metadata_read_open_after_hatch() {
        let mut engine = ScriptEngine::new();
        let id = engine.deploy(SRC, minter(), 1000, 10, 0).unwrap();
        // Read before hatch reverts.
        assert!(engine
            .call(id, "read_metadata", vec![], stranger(), 0)
            .is_err());
        engine
            .call(
                id,
                "hatch",
                vec![Value::Str("nymph→imago".to_string())],
                minter(),
                0,
            )
            .unwrap();
        // Anyone (incl. stranger) can read after hatch.
        assert_eq!(
            engine
                .call(id, "read_metadata", vec![], stranger(), 0)
                .unwrap()
                .return_value,
            Value::Str("nymph→imago".to_string())
        );
    }

    #[test]
    fn transfer_only_by_current_holder() {
        let mut engine = ScriptEngine::new();
        let id = engine.deploy(SRC, minter(), 1000, 10, 0).unwrap();
        engine
            .call(
                id,
                "hatch",
                vec![Value::Str("nymph".to_string())],
                minter(),
                0,
            )
            .unwrap();
        // Minter is the initial holder.
        assert_eq!(
            engine
                .call(id, "is_holder", vec![Value::Address(minter())], stranger(), 0)
                .unwrap()
                .return_value,
            Value::Bool(true)
        );
        // Stranger cannot transfer (not the holder).
        assert!(engine
            .call(
                id,
                "transfer",
                vec![Value::Address(alice())],
                stranger(),
                0
            )
            .is_err());
        // Minter transfers to alice.
        engine
            .call(
                id,
                "transfer",
                vec![Value::Address(alice())],
                minter(),
                0,
            )
            .unwrap();
        // is_holder updates.
        assert_eq!(
            engine
                .call(id, "is_holder", vec![Value::Address(alice())], stranger(), 0)
                .unwrap()
                .return_value,
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .call(id, "is_holder", vec![Value::Address(minter())], stranger(), 0)
                .unwrap()
                .return_value,
            Value::Bool(false)
        );
        // Minter (former holder) can no longer transfer.
        assert!(engine
            .call(
                id,
                "transfer",
                vec![Value::Address(bob())],
                minter(),
                0
            )
            .is_err());
        // Alice can now transfer to bob.
        engine
            .call(
                id,
                "transfer",
                vec![Value::Address(bob())],
                alice(),
                0,
            )
            .unwrap();
        assert_eq!(
            engine
                .call(id, "transfers_total", vec![], stranger(), 0)
                .unwrap()
                .return_value,
            Value::U64(2)
        );
    }

    #[test]
    fn transfer_requires_hatch_first() {
        let mut engine = ScriptEngine::new();
        let id = engine.deploy(SRC, minter(), 1000, 10, 0).unwrap();
        // Pre-hatch transfer reverts.
        assert!(engine
            .call(
                id,
                "transfer",
                vec![Value::Address(alice())],
                minter(),
                0
            )
            .is_err());
    }

    #[test]
    fn age_epochs_counts_from_hatch_epoch() {
        let mut engine = ScriptEngine::new();
        let id = engine.deploy(SRC, minter(), 1000, 10, 0).unwrap();
        // Pre-hatch age is 0 (sentinel handled by sealed bool).
        assert_eq!(
            engine
                .call(id, "age_epochs", vec![], stranger(), 50)
                .unwrap()
                .return_value,
            Value::U64(0)
        );
        // Hatch at epoch 7.
        engine
            .call(
                id,
                "hatch",
                vec![Value::Str("mayfly".to_string())],
                minter(),
                7,
            )
            .unwrap();
        // Same-epoch age: 0.
        assert_eq!(
            engine
                .call(id, "age_epochs", vec![], stranger(), 7)
                .unwrap()
                .return_value,
            Value::U64(0)
        );
        // 12 epochs later: 12.
        assert_eq!(
            engine
                .call(id, "age_epochs", vec![], stranger(), 19)
                .unwrap()
                .return_value,
            Value::U64(12)
        );
        // born() returns the hatch epoch.
        assert_eq!(
            engine
                .call(id, "born", vec![], stranger(), 100)
                .unwrap()
                .return_value,
            Value::U64(7)
        );
    }

    #[test]
    fn hatch_at_epoch_zero_works_via_sealed_sentinel() {
        // Regression for the witnessfit sentinel bug — born_epoch=0
        // must NOT be indistinguishable from "never hatched."
        let mut engine = ScriptEngine::new();
        let id = engine.deploy(SRC, minter(), 1000, 10, 0).unwrap();
        engine
            .call(
                id,
                "hatch",
                vec![Value::Str("epoch zero".to_string())],
                minter(),
                0,
            )
            .unwrap();
        // is_hatched + read_metadata still work despite born_epoch=0.
        assert_eq!(
            engine
                .call(id, "is_hatched", vec![], stranger(), 0)
                .unwrap()
                .return_value,
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .call(id, "read_metadata", vec![], stranger(), 0)
                .unwrap()
                .return_value,
            Value::Str("epoch zero".to_string())
        );
        // Re-hatch still rejected (sealed gate works).
        assert!(engine
            .call(
                id,
                "hatch",
                vec![Value::Str("again".to_string())],
                minter(),
                0
            )
            .is_err());
    }
}

