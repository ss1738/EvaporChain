use crate::compiler::{EvaporBytecode, Op};
use crate::{
    ContractEvent, ExecutionContext, ExternalCaller, ScriptCallResult, ScriptError, Value,
    MAX_CALL_DEPTH,
};
use std::collections::HashMap;

// ─── Gas Costs ──────────────────────────────────────────────────────────────

const GAS_PUSH: u64 = 1;
const GAS_POP: u64 = 1;
const GAS_LOAD: u64 = 2;
const GAS_STORE: u64 = 2;
const GAS_STATE_LOAD: u64 = 5;
const GAS_STATE_STORE: u64 = 10;
const GAS_ADD: u64 = 3;
const GAS_SUB: u64 = 3;
const GAS_MUL: u64 = 5;
const GAS_DIV: u64 = 5;
const GAS_CMP: u64 = 3;
const GAS_LOGIC: u64 = 3;
const GAS_JUMP: u64 = 2;
const GAS_CALL: u64 = 10;
/// M15 (audit 2026-05-13): size-scaled gas for O(n) builtins.
/// Pre-fix `hash()` ran FNV-1a over up to 1 MiB under flat GAS_CALL=10
/// — a single block could chain-stall the validator for ~13 minutes by
/// looping a 1 MiB-string hash. The base cost stays cheap so calls on
/// short inputs don't get over-charged; the per-32-byte rider is what
/// makes 1 MiB inputs actually expensive. Same shape as
/// `GAS_CONCAT_PER_32B` in the string-concat path at line ~236.
const GAS_HASH_BASE: u64 = 10;
const GAS_HASH_PER_32B: u64 = 1;
/// `to_string` on a structured value walks the whole structure via
/// Rust's `Display` impl. Use the same shape as the hash path so a
/// contract that recursively `to_string`s a near-MAX_STRING_LEN string
/// can't burn O(1 MiB) for `GAS_CALL=10`.
const GAS_TOSTRING_BASE: u64 = 10;
const GAS_TOSTRING_PER_32B: u64 = 1;
const GAS_MAP_GET: u64 = 10;
const GAS_MAP_SET: u64 = 20;
const GAS_REQUIRE: u64 = 5;
const GAS_EMIT: u64 = 8;
const GAS_EMIT_EVENT: u64 = 20;
const GAS_CALL_EXTERNAL: u64 = 100;
const GAS_RETURN: u64 = 1;
const GAS_MOD: u64 = 5;
const GAS_ARRAY_NEW: u64 = 5;
const GAS_ARRAY_GET: u64 = 5;
const GAS_ARRAY_SET: u64 = 10;

/// Maximum stack depth to prevent OOM.
const MAX_STACK_DEPTH: usize = 1024;
/// Maximum loop iterations per method call (gas also limits this, but this is a hard cap).
const MAX_LOOP_ITERATIONS: u64 = 100_000;
/// Maximum string length in bytes to prevent OOM via concatenation.
const MAX_STRING_LEN: usize = 1_048_576; // 1 MiB
/// L8 (audit 2026-05-13): hard cap on `Op::EmitEvent` topic_count.
/// Solidity allows 4 indexed topics; we're generous at 64. Caps the
/// pop loop + memory allocation in the EmitEvent arm. Bytecode
/// specifying a topic_count past this is rejected with
/// `ScriptError::Runtime` rather than panicking on
/// usize-overflow / OOM.
const MAX_EVENT_TOPICS: usize = 64;
/// Maximum entries in a single map to prevent OOM.
const MAX_MAP_ENTRIES: usize = 10_000;
/// Maximum elements in a single array to prevent OOM.
const MAX_ARRAY_SIZE: usize = 10_000;
/// Maximum state keys per contract to prevent unbounded storage growth.
const MAX_STATE_KEYS: usize = 10_000;
/// Hard step limit: maximum number of opcodes executed per method call.
/// Independent of gas — prevents infinite loops even if gas accounting has bugs.
const MAX_STEPS: u64 = 10_000_000;

/// Default gas limit applied when no explicit limit is provided.
/// Prevents unbounded execution — gas metering is ALWAYS enforced.
pub const DEFAULT_GAS_LIMIT: u64 = 10_000_000;

/// Maximum total heap memory a single VM execution can allocate (strings, arrays, maps).
const MAX_MEMORY_BYTES: usize = 4_194_304; // 4 MiB

// ─── VM ─────────────────────────────────────────────────────────────────────

/// M15 (audit 2026-05-13): cheap upper-bound estimate for the byte
/// length of `Display::fmt(v)`. Walks recursive Map / Array values.
/// Used to size-scale the gas cost of `to_string`. Returns
/// approximate bytes; bounded by the VM's MAX_MEMORY_BYTES cap on
/// any single value, so this is O(materialised size) not O(blow-up).
fn estimated_to_string_bytes(v: &Value) -> usize {
    match v {
        Value::Str(s) => s.len(),
        // u64 → up to 20 ASCII digits
        Value::U64(_) => 20,
        // "true" | "false"
        Value::Bool(_) => 5,
        // 32-byte address → "0x" + 64 hex chars + commas/braces fluff
        Value::Address(_) => 70,
        // "null"
        Value::Null => 4,
        Value::Map(m) => {
            // braces, commas, colons, quotes — ~6 bytes of fluff per entry
            let fluff: usize = m.len().saturating_mul(6);
            let entries: usize = m
                .iter()
                .map(|(k, val)| k.len() + estimated_to_string_bytes(val))
                .sum();
            fluff.saturating_add(entries).saturating_add(2)
        }
        Value::Array(a) => {
            let fluff: usize = a.len().saturating_mul(2);
            let entries: usize = a.iter().map(estimated_to_string_bytes).sum();
            fluff.saturating_add(entries).saturating_add(2)
        }
    }
}

/// Stack-based virtual machine for EvaporScript bytecode.
pub struct EvaporVM {
    stack: Vec<Value>,
    locals: HashMap<String, Value>,
    state: HashMap<String, Value>,
    events: Vec<String>,
    structured_events: Vec<ContractEvent>,
    gas_used: u64,
    gas_limit: u64,
    /// Hard step counter: incremented on every opcode, independent of gas.
    step_count: u64,
    /// Running total of heap bytes allocated (strings, arrays, maps).
    memory_used: usize,
    /// M6 (audit 2026-05-02): true while the VM is executing one of
    /// the lifecycle-hook methods (`on_grace`, `on_refresh`,
    /// `on_evaporate`). When set, `Op::CallExternal` is rejected — a
    /// hook making cross-contract calls invites re-entrant lifecycle
    /// invocations which can race-mutate the same contract's state
    /// (e.g. a refresh hook bumping a counter while a callee triggers
    /// another refresh on the same object).
    in_lifecycle_hook: bool,
    /// SCR-N1 (audit 2026-05-15): contract_id whose bytecode this VM
    /// is currently executing. Set by the dispatcher
    /// (`call_with_vm_gas` / `ContractCallRouter::call_external`) so
    /// `Op::CallExternal` can pass the *calling contract's* identity
    /// to the callee — not the original EOA. `0` means "top-level
    /// call from EOA"; the VM then passes `ctx.caller` (the EOA)
    /// unchanged, which matches the previous behaviour and keeps
    /// test fixtures backward-compatible.
    pub(crate) executing_contract_id: u64,
}

impl EvaporVM {
    fn new(state: HashMap<String, Value>, gas_limit: u64) -> Self {
        Self {
            stack: Vec::new(),
            locals: HashMap::new(),
            state,
            events: Vec::new(),
            structured_events: Vec::new(),
            gas_used: 0,
            gas_limit,
            step_count: 0,
            memory_used: 0,
            in_lifecycle_hook: false,
            executing_contract_id: 0,
        }
    }

    /// Returns true if `method` is one of the structurally-special
    /// lifecycle hooks. Hooks run with `Op::CallExternal` disabled
    /// (M6: re-entrancy guard).
    fn is_lifecycle_hook(method: &str) -> bool {
        matches!(method, "on_grace" | "on_refresh" | "on_evaporate")
    }

    fn track_memory(&mut self, bytes: usize) -> Result<(), ScriptError> {
        self.memory_used = self.memory_used.saturating_add(bytes);
        if self.memory_used > MAX_MEMORY_BYTES {
            return Err(ScriptError::Runtime(format!(
                "memory limit exceeded: {} bytes (limit {MAX_MEMORY_BYTES})",
                self.memory_used
            )));
        }
        Ok(())
    }

    fn charge_gas(&mut self, cost: u64) -> Result<(), ScriptError> {
        // Audit G1 (2026-05-15): use checked_add to prevent overflow wrap-around.
        // Without this, gas_used near u64::MAX + a large cost wraps to near-zero,
        // allowing unbounded execution past the gas limit.
        self.gas_used = self.gas_used.checked_add(cost).ok_or(ScriptError::GasLimitExceeded {
            used: u64::MAX,
            limit: self.gas_limit,
        })?;
        if self.gas_used > self.gas_limit {
            return Err(ScriptError::GasLimitExceeded {
                used: self.gas_used,
                limit: self.gas_limit,
            });
        }
        Ok(())
    }

    fn push(&mut self, val: Value) -> Result<(), ScriptError> {
        if self.stack.len() >= MAX_STACK_DEPTH {
            return Err(ScriptError::Runtime(format!(
                "stack overflow: depth exceeded {MAX_STACK_DEPTH}"
            )));
        }
        self.stack.push(val);
        Ok(())
    }

    fn pop(&mut self) -> Result<Value, ScriptError> {
        self.stack
            .pop()
            .ok_or_else(|| ScriptError::Runtime("stack underflow".into()))
    }

    fn execute_method(
        &mut self,
        bytecode: &EvaporBytecode,
        method: &str,
        args: Vec<Value>,
        ctx: &ExecutionContext,
        external: &mut Option<&mut dyn ExternalCaller>,
    ) -> Result<Value, ScriptError> {
        let start_offset = bytecode
            .methods
            .get(method)
            .ok_or_else(|| ScriptError::Runtime(format!("method '{method}' not found")))?;

        // Push args onto stack (left-to-right, so first arg is deepest)
        for arg in args {
            self.push(arg)?;
        }

        let mut ip = *start_offset;
        let mut loop_counter: u64 = 0;

        loop {
            if ip >= bytecode.opcodes.len() {
                return Ok(Value::Null);
            }

            // Hard step limit: prevents infinite loops independent of gas accounting.
            // Audit G2 (2026-05-15): checked_add prevents overflow wrap-around that
            // would reset step_count to near-zero and bypass the instruction limit.
            self.step_count = self.step_count.checked_add(1).ok_or(
                ScriptError::StepLimitExceeded {
                    steps: u64::MAX,
                    limit: MAX_STEPS,
                }
            )?;
            if self.step_count > MAX_STEPS {
                return Err(ScriptError::StepLimitExceeded {
                    steps: self.step_count,
                    limit: MAX_STEPS,
                });
            }

            let op = &bytecode.opcodes[ip];

            match op {
                Op::Push(val) => {
                    self.charge_gas(GAS_PUSH)?;
                    self.push(val.clone())?;
                }

                Op::Pop => {
                    self.charge_gas(GAS_POP)?;
                    self.pop()?;
                }

                Op::Load(name) => {
                    self.charge_gas(GAS_LOAD)?;
                    let val = self.locals.get(name).cloned().unwrap_or(Value::Null);
                    self.push(val)?;
                }

                Op::Store(name) => {
                    self.charge_gas(GAS_STORE)?;
                    let val = self.pop()?;
                    self.locals.insert(name.clone(), val);
                }

                Op::StateLoad(field) => {
                    self.charge_gas(GAS_STATE_LOAD)?;
                    let val = self.state.get(field).cloned().unwrap_or(Value::Null);
                    self.push(val)?;
                }

                Op::StateStore(field) => {
                    self.charge_gas(GAS_STATE_STORE)?;
                    let val = self.pop()?;
                    if !self.state.contains_key(field.as_str())
                        && self.state.len() >= MAX_STATE_KEYS
                    {
                        return Err(ScriptError::Runtime(format!(
                            "contract storage limit exceeded ({MAX_STATE_KEYS} keys)"
                        )));
                    }
                    self.state.insert(field.clone(), val);
                }

                Op::Add => {
                    self.charge_gas(GAS_ADD)?;
                    let b = self.pop()?;
                    let a = self.pop()?;
                    let result = match (&a, &b) {
                        (Value::U64(x), Value::U64(y)) => {
                            Value::U64(x.checked_add(*y).ok_or_else(|| {
                                ScriptError::Runtime("arithmetic overflow: addition".into())
                            })?)
                        }
                        (Value::Str(x), Value::Str(y)) => {
                            let concat_len = x.len() + y.len();
                            self.charge_gas(3 + (concat_len as u64) / 32)?;
                            if concat_len > MAX_STRING_LEN {
                                return Err(ScriptError::Runtime(format!(
                                    "string too large: {} bytes exceeds limit of {MAX_STRING_LEN}",
                                    concat_len
                                )));
                            }
                            self.track_memory(concat_len)?;
                            Value::Str(format!("{x}{y}"))
                        }
                        _ => {
                            return Err(ScriptError::Runtime(format!("cannot add {a:?} and {b:?}")))
                        }
                    };
                    self.push(result)?;
                }

                Op::Sub => {
                    self.charge_gas(GAS_SUB)?;
                    let b = self.pop()?.as_u64()?;
                    let a = self.pop()?.as_u64()?;
                    let result = a.checked_sub(b).ok_or_else(|| {
                        ScriptError::Runtime("arithmetic underflow: subtraction".into())
                    })?;
                    self.push(Value::U64(result))?;
                }

                Op::Mul => {
                    self.charge_gas(GAS_MUL)?;
                    let b = self.pop()?.as_u64()?;
                    let a = self.pop()?.as_u64()?;
                    let result = a.checked_mul(b).ok_or_else(|| {
                        ScriptError::Runtime("arithmetic overflow: multiplication".into())
                    })?;
                    self.push(Value::U64(result))?;
                }

                Op::Div => {
                    self.charge_gas(GAS_DIV)?;
                    let b = self.pop()?.as_u64()?;
                    let a = self.pop()?.as_u64()?;
                    if b == 0 {
                        return Err(ScriptError::Runtime("division by zero".into()));
                    }
                    self.push(Value::U64(a / b))?;
                }

                Op::Mod => {
                    self.charge_gas(GAS_MOD)?;
                    let b = self.pop()?.as_u64()?;
                    let a = self.pop()?.as_u64()?;
                    if b == 0 {
                        return Err(ScriptError::Runtime("modulo by zero".into()));
                    }
                    self.push(Value::U64(a % b))?;
                }

                Op::Eq => {
                    self.charge_gas(GAS_CMP)?;
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(Value::Bool(a == b))?;
                }

                Op::Neq => {
                    self.charge_gas(GAS_CMP)?;
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(Value::Bool(a != b))?;
                }

                Op::Gt => {
                    self.charge_gas(GAS_CMP)?;
                    let b = self.pop()?.as_u64()?;
                    let a = self.pop()?.as_u64()?;
                    self.push(Value::Bool(a > b))?;
                }

                Op::Lt => {
                    self.charge_gas(GAS_CMP)?;
                    let b = self.pop()?.as_u64()?;
                    let a = self.pop()?.as_u64()?;
                    self.push(Value::Bool(a < b))?;
                }

                Op::Gte => {
                    self.charge_gas(GAS_CMP)?;
                    let b = self.pop()?.as_u64()?;
                    let a = self.pop()?.as_u64()?;
                    self.push(Value::Bool(a >= b))?;
                }

                Op::Lte => {
                    self.charge_gas(GAS_CMP)?;
                    let b = self.pop()?.as_u64()?;
                    let a = self.pop()?.as_u64()?;
                    self.push(Value::Bool(a <= b))?;
                }

                Op::And => {
                    self.charge_gas(GAS_LOGIC)?;
                    let b = self.pop()?.as_bool()?;
                    let a = self.pop()?.as_bool()?;
                    self.push(Value::Bool(a && b))?;
                }

                Op::Or => {
                    self.charge_gas(GAS_LOGIC)?;
                    let b = self.pop()?.as_bool()?;
                    let a = self.pop()?.as_bool()?;
                    self.push(Value::Bool(a || b))?;
                }

                Op::Not => {
                    self.charge_gas(GAS_LOGIC)?;
                    let a = self.pop()?.as_bool()?;
                    self.push(Value::Bool(!a))?;
                }

                Op::Neg => {
                    self.charge_gas(GAS_SUB)?;
                    let val = self.pop()?;
                    match val {
                        Value::U64(0) => self.push(Value::U64(0))?,
                        Value::U64(n) => {
                            return Err(ScriptError::Runtime(format!(
                                "cannot negate unsigned integer {n}: EvaporScript uses u64 only"
                            )));
                        }
                        Value::Bool(b) => self.push(Value::Bool(!b))?,
                        other => {
                            return Err(ScriptError::Runtime(format!(
                                "cannot negate value of type {}: expected number or bool",
                                match other {
                                    Value::Str(_) => "string",
                                    Value::Null => "null",
                                    Value::Map(_) => "map",
                                    Value::Array(_) => "array",
                                    Value::Address(_) => "address",
                                    _ => "unknown",
                                }
                            )));
                        }
                    }
                }

                Op::Jump(target) => {
                    self.charge_gas(GAS_JUMP)?;
                    if *target >= bytecode.opcodes.len() {
                        return Err(ScriptError::Runtime(format!(
                            "jump target {} out of bounds (bytecode len {})",
                            target,
                            bytecode.opcodes.len()
                        )));
                    }
                    if *target <= ip {
                        loop_counter += 1;
                        if loop_counter > MAX_LOOP_ITERATIONS {
                            return Err(ScriptError::Runtime(format!(
                                "loop iteration limit exceeded ({MAX_LOOP_ITERATIONS})"
                            )));
                        }
                    }
                    ip = *target;
                    continue;
                }

                Op::JumpIf(target) => {
                    self.charge_gas(GAS_JUMP)?;
                    if *target >= bytecode.opcodes.len() {
                        return Err(ScriptError::Runtime(format!(
                            "jump target {} out of bounds (bytecode len {})",
                            target,
                            bytecode.opcodes.len()
                        )));
                    }
                    let cond = self.pop()?.as_bool()?;
                    if cond {
                        if *target <= ip {
                            loop_counter += 1;
                            if loop_counter > MAX_LOOP_ITERATIONS {
                                return Err(ScriptError::Runtime(format!(
                                    "loop iteration limit exceeded ({MAX_LOOP_ITERATIONS})"
                                )));
                            }
                        }
                        ip = *target;
                        continue;
                    }
                }

                Op::JumpIfFalse(target) => {
                    self.charge_gas(GAS_JUMP)?;
                    if *target >= bytecode.opcodes.len() {
                        return Err(ScriptError::Runtime(format!(
                            "jump target {} out of bounds (bytecode len {})",
                            target,
                            bytecode.opcodes.len()
                        )));
                    }
                    let cond = self.pop()?.as_bool()?;
                    if !cond {
                        if *target <= ip {
                            loop_counter += 1;
                            if loop_counter > MAX_LOOP_ITERATIONS {
                                return Err(ScriptError::Runtime(format!(
                                    "loop iteration limit exceeded ({MAX_LOOP_ITERATIONS})"
                                )));
                            }
                        }
                        ip = *target;
                        continue;
                    }
                }

                Op::Call(name, arg_count) => {
                    self.charge_gas(GAS_CALL)?;
                    let result = self.call_builtin(name, *arg_count, ctx)?;
                    self.push(result)?;
                }

                Op::ArrayNew(count) => {
                    self.charge_gas(GAS_ARRAY_NEW)?;
                    let count = *count;
                    if count > MAX_ARRAY_SIZE {
                        return Err(ScriptError::Runtime(format!(
                            "array size limit exceeded ({MAX_ARRAY_SIZE})"
                        )));
                    }
                    self.charge_gas(count as u64)?;
                    self.track_memory(count * 16)?; // ~16 bytes per Value slot
                    let mut elements = Vec::with_capacity(count);
                    for _ in 0..count {
                        elements.push(self.pop()?);
                    }
                    elements.reverse();
                    self.push(Value::Array(elements))?;
                }

                Op::ArrayGet => {
                    self.charge_gas(GAS_ARRAY_GET)?;
                    let index = self.pop()?.as_u64()? as usize;
                    let array = self.pop()?;
                    match array {
                        Value::Array(arr) => {
                            if index >= arr.len() {
                                return Err(ScriptError::Runtime(format!(
                                    "array index out of bounds: index {index}, length {}",
                                    arr.len()
                                )));
                            }
                            self.push(arr[index].clone())?;
                        }
                        other => {
                            return Err(ScriptError::Runtime(format!(
                                "expected array, got {other:?}"
                            )))
                        }
                    }
                }

                Op::ArraySet(name) => {
                    self.charge_gas(GAS_ARRAY_SET)?;
                    let index = self.pop()?.as_u64()? as usize;
                    let value = self.pop()?;
                    let arr = self.locals.get_mut(name).ok_or_else(|| {
                        ScriptError::Runtime(format!("undefined variable: {name}"))
                    })?;
                    match arr {
                        Value::Array(ref mut vec) => {
                            if index >= vec.len() {
                                return Err(ScriptError::Runtime(format!(
                                    "array index out of bounds: index {index}, length {}",
                                    vec.len()
                                )));
                            }
                            vec[index] = value;
                        }
                        other => {
                            return Err(ScriptError::Runtime(format!(
                                "expected array for '{name}', got {other:?}"
                            )))
                        }
                    }
                }

                Op::MapGet(field) => {
                    // H10 (audit 2026-05-02): considered returning
                    // `Value::Null` for missing keys but reverted —
                    // EvaporScript contracts idiomatically use
                    // `self.map[k] += amount`, which the compiler
                    // lowers to `MapGet; Push amount; Add; MapSet`. A
                    // Null here breaks `Add` with a numeric op2. The
                    // audit finding is filed as a *language-design*
                    // choice (Solidity-like default-zero), not a
                    // VM-level bug. Contracts that need to detect
                    // missing-vs-zero must compare with a sentinel or
                    // use a separate presence map.
                    self.charge_gas(GAS_MAP_GET)?;
                    let key = self.pop()?;
                    let val = match self.state.get(field) {
                        Some(Value::Map(map)) => {
                            let key_str = key.to_map_key();
                            map.get(&key_str).cloned().unwrap_or(Value::U64(0))
                        }
                        Some(Value::Array(arr)) => {
                            let index = key.as_u64()? as usize;
                            if index >= arr.len() {
                                return Err(ScriptError::Runtime(format!(
                                    "array index out of bounds: index {index}, length {}",
                                    arr.len()
                                )));
                            }
                            arr[index].clone()
                        }
                        _ => Value::U64(0),
                    };
                    self.push(val)?;
                }

                Op::MapSet(field) => {
                    self.charge_gas(GAS_MAP_SET)?;
                    let val = self.pop()?;
                    let key = self.pop()?;

                    // M9 (audit 2026-05-02): if the field already holds a
                    // non-Map / non-Array value (e.g. U64 from earlier
                    // initialization), the previous code silently
                    // replaced it with `Value::Map(...)`, losing data
                    // and converting types behind the contract's back.
                    // Reject the set instead — type changes must be
                    // explicit.
                    match self.state.get(field.as_str()) {
                        Some(Value::Map(_)) | Some(Value::Array(_)) | None => {}
                        Some(_) => {
                            return Err(ScriptError::Runtime(format!(
                                "MapSet on field '{field}' which is not a Map or Array — \
                                 refuse to silently coerce; declare the field as a map at deploy"
                            )));
                        }
                    }

                    // Check if this is a new map entry — track memory before borrowing state
                    let is_new_entry = match self.state.get(field.as_str()) {
                        Some(Value::Map(m)) => !m.contains_key(&key.to_map_key()),
                        None => true,
                        _ => false,
                    };
                    if is_new_entry {
                        self.track_memory(64)?; // ~64 bytes per new map entry
                    }

                    let entry = self
                        .state
                        .entry(field.clone())
                        .or_insert_with(|| Value::Map(HashMap::new()));
                    match entry {
                        Value::Map(m) => {
                            let key_str = key.to_map_key();
                            if !m.contains_key(&key_str) && m.len() >= MAX_MAP_ENTRIES {
                                return Err(ScriptError::Runtime(format!(
                                    "map entry limit exceeded ({MAX_MAP_ENTRIES})"
                                )));
                            }
                            m.insert(key_str, val);
                        }
                        Value::Array(arr) => {
                            let index = key.as_u64()? as usize;
                            if index >= arr.len() {
                                return Err(ScriptError::Runtime(format!(
                                    "array index out of bounds: index {index}, length {}",
                                    arr.len()
                                )));
                            }
                            arr[index] = val;
                        }
                        _ => {
                            return Err(ScriptError::Runtime(format!(
                                "state field '{field}' is not a map or array"
                            )))
                        }
                    }
                }

                Op::Require => {
                    self.charge_gas(GAS_REQUIRE)?;
                    let cond = self.pop()?.as_bool()?;
                    let msg = self.pop()?;
                    if !cond {
                        let msg_str = match msg {
                            Value::Str(s) => s,
                            other => format!("{other}"),
                        };
                        return Err(ScriptError::RequireFailed(msg_str));
                    }
                }

                Op::Emit => {
                    self.charge_gas(GAS_EMIT)?;
                    let val = self.pop()?;
                    let msg = match val {
                        Value::Str(s) => s,
                        other => format!("{other}"),
                    };
                    // OPCODE-5 (audit 2026-05-17): track message bytes before
                    // enqueuing so bulk emits can't silently exhaust heap past
                    // MAX_MEMORY_BYTES while only paying flat GAS_EMIT each.
                    self.track_memory(msg.len())?;
                    self.events.push(msg.clone());
                    self.structured_events.push(ContractEvent {
                        name: "Log".into(),
                        topics: vec![],
                        data: vec![Value::Str(msg)],
                    });
                }

                Op::EmitEvent { name, topic_count } => {
                    // L8 (audit 2026-05-13): pre-fix `let total = tc
                    // + 1` was an unchecked usize add and `tc` had
                    // no upper bound. Dead today (the EvaporScript
                    // compiler never emits Op::EmitEvent with a huge
                    // topic_count) but a foot-gun for any future
                    // codepath that constructs bytecode directly —
                    // tc=usize::MAX wraps the addition, and a large
                    // tc burns memory via the Vec::with_capacity.
                    // Cap + checked_add closes both ends.
                    let tc = *topic_count;
                    if tc > MAX_EVENT_TOPICS {
                        return Err(ScriptError::Runtime(format!(
                            "EmitEvent topic_count {tc} exceeds MAX_EVENT_TOPICS ({MAX_EVENT_TOPICS})"
                        )));
                    }
                    let total = tc.checked_add(1).ok_or_else(|| {
                        ScriptError::Runtime(
                            "EmitEvent topic_count overflowed usize".into(),
                        )
                    })?;
                    // Size-scale the gas charge so a near-cap event
                    // doesn't ride for the flat `GAS_EMIT_EVENT = 20`.
                    let scaled = GAS_EMIT_EVENT.saturating_add(total as u64);
                    self.charge_gas(scaled)?;
                    let mut values = Vec::with_capacity(total);
                    for _ in 0..total {
                        values.push(self.pop()?);
                    }
                    values.reverse();
                    let topics = values[..tc].to_vec();
                    let data = values[tc..].to_vec();
                    // OPCODE-5: track name + all value sizes before enqueuing.
                    let event_bytes = name.len()
                        + values
                            .iter()
                            .map(|v| match v {
                                Value::Str(s) => s.len(),
                                _ => 8,
                            })
                            .sum::<usize>();
                    self.track_memory(event_bytes)?;
                    self.events.push(format!("event:{name}"));
                    self.structured_events.push(ContractEvent {
                        name: name.clone(),
                        topics,
                        data,
                    });
                }

                Op::Return => {
                    self.charge_gas(GAS_RETURN)?;
                    return self.pop().or(Ok(Value::Null));
                }

                Op::Halt => {
                    return Ok(Value::Null);
                }

                // ── Temporal Opcodes ──
                Op::EpochNow => {
                    self.charge_gas(GAS_PUSH)?;
                    self.push(Value::U64(ctx.epoch))?;
                }

                Op::BlockNum => {
                    self.charge_gas(GAS_PUSH)?;
                    // Block number is available as epoch (1:1 mapping in EvaporChain).
                    self.push(Value::U64(ctx.epoch))?;
                }

                Op::EnergyOf => {
                    self.charge_gas(GAS_STATE_LOAD)?;
                    // For scripts, energy_of returns the contract's own energy.
                    // Object energy queries are done via the contract engine.
                    self.push(Value::U64(ctx.energy))?;
                }

                Op::RequireEpochRange => {
                    self.charge_gas(GAS_REQUIRE)?;
                    let max_epoch = self.pop()?.as_u64()?;
                    let min_epoch = self.pop()?.as_u64()?;
                    if ctx.epoch < min_epoch || ctx.epoch >= max_epoch {
                        return Err(ScriptError::Runtime(format!(
                            "epoch {} outside required range [{}, {})",
                            ctx.epoch, min_epoch, max_epoch
                        )));
                    }
                }

                Op::ComputeDecay => {
                    self.charge_gas(GAS_MUL)?; // Slightly more expensive
                    let half_life = self.pop()?.as_u64()?;
                    let initial_energy = self.pop()?.as_u64()?;
                    let epochs_elapsed = self.pop()?.as_u64()?;
                    let decayed = evaporchain_types::energy_at_epoch(
                        initial_energy,
                        half_life,
                        epochs_elapsed,
                    );
                    self.push(Value::U64(decayed))?;
                }

                // ── VRF / Randomness Opcodes ──
                Op::VrfRandomness => {
                    self.charge_gas(GAS_PUSH)?;
                    let value = u64::from_le_bytes(ctx.vrf_randomness[..8].try_into().unwrap());
                    self.push(Value::U64(value))?;
                }

                Op::VrfDomainRandomness => {
                    // OPCODE-1 (audit 2026-05-17): size-scaled BLAKE3 charge.
                    // Pre-fix flat GAS_STATE_LOAD=5 ignored the popped `domain`
                    // string length (up to MAX_STRING_LEN = 1 MiB), letting a
                    // contract hash megabytes of input for ~5 gas — same anti-
                    // pattern as M15 (hash() flat-rate). Now uses the M15
                    // size-scaled shape: GAS_HASH_BASE + per-32B byte rider.
                    let domain = self.pop()?.as_str()?.to_string();
                    let chunks_32 = (domain.len() as u64).div_ceil(32);
                    self.charge_gas(GAS_HASH_BASE + GAS_HASH_PER_32B * chunks_32)?;
                    let mut hasher = blake3::Hasher::new();
                    hasher.update(b"EvaporChain_Beacon_Derive");
                    hasher.update(&ctx.vrf_randomness);
                    hasher.update(domain.as_bytes());
                    let derived = hasher.finalize();
                    let value = u64::from_le_bytes(derived.as_bytes()[..8].try_into().unwrap());
                    self.push(Value::U64(value))?;
                }

                Op::RandomRange => {
                    self.charge_gas(GAS_STATE_LOAD)?;
                    let max = self.pop()?.as_u64()?;
                    if max == 0 {
                        return Err(ScriptError::Runtime("random_range: max must be > 0".into()));
                    }
                    // SCR-N6 (audit 2026-05-15): rejection sampling
                    // to eliminate modulo bias for non-power-of-two
                    // `max`. The naive `raw % max` over-weights the
                    // first `u64::MAX % max` outputs — for max=3 the
                    // first two indices are ~2x more likely than the
                    // third. Deterministic (no consensus issue) but
                    // a lottery/raffle contract drawing from N=3
                    // would see biased outcomes. We re-derive
                    // randomness from chained blake3 of
                    // vrf_randomness + an iteration counter until the
                    // sample falls within the unbiased range. Bounded
                    // by `MAX_REJECTION_ITERS = 64` (probability of
                    // 64 consecutive rejections is < 2^-64 for any
                    // reasonable `max`).
                    const MAX_REJECTION_ITERS: u32 = 64;
                    let zone = u64::MAX - (u64::MAX % max);
                    let mut iter: u32 = 0;
                    let value = loop {
                        let mut h = blake3::Hasher::new();
                        h.update(b"EvaporChain_RandomRange_Reject_v1");
                        h.update(&ctx.vrf_randomness);
                        h.update(&iter.to_le_bytes());
                        let derived = h.finalize();
                        let raw = u64::from_le_bytes(
                            derived.as_bytes()[..8].try_into().unwrap(),
                        );
                        if raw < zone || iter >= MAX_REJECTION_ITERS {
                            // Fallback at hard iteration cap: accept
                            // the biased sample. With probability
                            // < 2^-64 over the iter range this branch
                            // is unreachable in practice; the explicit
                            // cap ensures the VM cannot loop forever
                            // on adversarial VRF inputs.
                            break raw % max;
                        }
                        iter += 1;
                    };
                    self.push(Value::U64(value))?;
                }

                Op::CallExternal { arg_count } => {
                    self.charge_gas(GAS_CALL_EXTERNAL)?;
                    // M6 (audit 2026-05-02): refuse cross-contract
                    // calls inside a lifecycle hook. The hook runs
                    // synchronously while the chain is mutating the
                    // owning object's state; allowing CallExternal
                    // here lets a callee (or its own lifecycle) trip
                    // back into this same object before the original
                    // mutation lands.
                    if self.in_lifecycle_hook {
                        return Err(ScriptError::Runtime(
                            "cross-contract calls are forbidden inside lifecycle hooks \
                             (on_grace / on_refresh / on_evaporate)"
                                .into(),
                        ));
                    }
                    if ctx.call_depth >= MAX_CALL_DEPTH {
                        return Err(ScriptError::Runtime(format!(
                            "cross-contract call depth exceeded (max {})",
                            MAX_CALL_DEPTH
                        )));
                    }
                    let ac = *arg_count;
                    // Stack: [contract_id, method, arg0, arg1, ...] (bottom to top)
                    // arg_count includes contract_id and method, so actual args = ac - 2
                    let extra_args = ac.saturating_sub(2);
                    let mut args = Vec::with_capacity(extra_args);
                    for _ in 0..extra_args {
                        args.push(self.pop()?);
                    }
                    args.reverse();
                    let method = self.pop()?.as_str()?.to_string();
                    let contract_id = self.pop()?.as_u64()?;

                    let gas_remaining = self.gas_limit.saturating_sub(self.gas_used);

                    if let Some(ext) = external.as_mut() {
                        // SCR-N1 (audit 2026-05-15): when this VM is
                        // executing inside a known contract (id != 0),
                        // the callee's `caller` MUST be this
                        // contract's identity address — NOT
                        // `ctx.caller` (which is the original EOA).
                        // Otherwise any `require(caller == owner)`
                        // gate on the callee is trivially bypassable
                        // by any user routing through this contract.
                        // Top-level / test-fixture path (id = 0)
                        // preserves the old behaviour for backward
                        // compatibility.
                        let nested_caller = if self.executing_contract_id != 0 {
                            crate::contract_address(self.executing_contract_id)
                        } else {
                            ctx.caller
                        };
                        let (return_val, events, gas_used) = ext.call_external(
                            contract_id,
                            &method,
                            args,
                            nested_caller,
                            ctx.epoch,
                            ctx.call_depth + 1,
                            gas_remaining,
                        )?;
                        // Audit G3 (2026-05-15): checked_add + re-check after
                        // external call returns its gas consumption.  Without this, a
                        // callee returning a large gas_used can wrap the caller's
                        // gas_used to near-zero, bypassing the gas limit.
                        self.gas_used = self.gas_used.checked_add(gas_used).ok_or(
                            ScriptError::GasLimitExceeded {
                                used: u64::MAX,
                                limit: self.gas_limit,
                            }
                        )?;
                        if self.gas_used > self.gas_limit {
                            return Err(ScriptError::GasLimitExceeded {
                                used: self.gas_used,
                                limit: self.gas_limit,
                            });
                        }
                        self.structured_events.extend(events);
                        self.push(return_val)?;
                    } else {
                        return Err(ScriptError::Runtime(
                            "cross-contract calls not available in this context".into(),
                        ));
                    }
                }
            }

            ip += 1;
        }
    }

    fn call_builtin(
        &mut self,
        name: &str,
        arg_count: usize,
        ctx: &ExecutionContext,
    ) -> Result<Value, ScriptError> {
        match name {
            // Zero-arg context accessors
            "caller" => Ok(Value::Address(ctx.caller)),
            "owner" => Ok(Value::Address(ctx.owner)),
            "epoch" => Ok(Value::U64(ctx.epoch)),
            "energy" => Ok(Value::U64(ctx.energy)),

            // One-arg built-ins
            "balance" => {
                if arg_count != 1 {
                    return Err(ScriptError::Runtime("balance() takes 1 argument".into()));
                }
                let _addr = self.pop()?.as_address()?;
                // In a real implementation, this would look up the on-chain balance.
                // For the script VM, we return 0 as a placeholder.
                Ok(Value::U64(0))
            }

            "transfer" => {
                if arg_count != 2 {
                    return Err(ScriptError::Runtime("transfer() takes 2 arguments".into()));
                }
                let _amount = self.pop()?.as_u64()?;
                let _to = self.pop()?.as_address()?;
                // Transfer is a side-effect; in integration, this would be wired
                // to the state DB. For now, emit an event.
                self.events
                    .push(format!("transfer({_amount} to 0x{})", hex::encode(_to)));
                Ok(Value::Null)
            }

            // emit and require are handled as opcodes, but can also be called as functions
            "emit" => {
                if arg_count != 1 {
                    return Err(ScriptError::Runtime("emit() takes 1 argument".into()));
                }
                let val = self.pop()?;
                let msg = match val {
                    Value::Str(s) => s,
                    other => format!("{other}"),
                };
                // OPCODE-5: same guard as Op::Emit.
                self.track_memory(msg.len())?;
                self.events.push(msg);
                Ok(Value::Null)
            }

            // Allen-Decay temporal-interval relation. Wraps the 13-relation
            // interval algebra (Allen 1983) from `evaporchain-allen-decay`
            // into a single VM built-in to keep the opcode count small —
            // contracts branch on the returned u64 code rather than calling
            // 13 separate builtins. Encoding (must match
            // `crate::allen::ALLEN_RELATION_*` constants in this file):
            //   0 Before, 1 Meets, 2 Overlaps, 3 Starts, 4 During,
            //   5 Finishes, 6 Equals, 7 FinishedBy, 8 Contains,
            //   9 StartedBy, 10 OverlappedBy, 11 MetBy, 12 After.
            // Args (top of stack first to last popped): start_a, end_a,
            // start_b, end_b. Stack semantics match other 4-arg builtins.
            // Returns a u64 in 0..=12, or errors if any interval is
            // invalid (start >= end).
            "allen_relation" => {
                if arg_count != 4 {
                    return Err(ScriptError::Runtime(
                        "allen_relation() takes 4 arguments: start_a, end_a, start_b, end_b".into(),
                    ));
                }
                let end_b = self.pop()?.as_u64()?;
                let start_b = self.pop()?.as_u64()?;
                let end_a = self.pop()?.as_u64()?;
                let start_a = self.pop()?.as_u64()?;
                let interval_a =
                    evaporchain_allen_decay::Interval::new(start_a, end_a).map_err(|e| {
                        ScriptError::Runtime(format!("allen_relation: invalid interval A: {e}"))
                    })?;
                let interval_b =
                    evaporchain_allen_decay::Interval::new(start_b, end_b).map_err(|e| {
                        ScriptError::Runtime(format!("allen_relation: invalid interval B: {e}"))
                    })?;
                let rel = evaporchain_allen_decay::compute_relation(interval_a, interval_b);
                let code = match rel {
                    evaporchain_allen_decay::AllenRelation::Before => 0u64,
                    evaporchain_allen_decay::AllenRelation::Meets => 1,
                    evaporchain_allen_decay::AllenRelation::Overlaps => 2,
                    evaporchain_allen_decay::AllenRelation::Starts => 3,
                    evaporchain_allen_decay::AllenRelation::During => 4,
                    evaporchain_allen_decay::AllenRelation::Finishes => 5,
                    evaporchain_allen_decay::AllenRelation::Equals => 6,
                    evaporchain_allen_decay::AllenRelation::FinishedBy => 7,
                    evaporchain_allen_decay::AllenRelation::Contains => 8,
                    evaporchain_allen_decay::AllenRelation::StartedBy => 9,
                    evaporchain_allen_decay::AllenRelation::OverlappedBy => 10,
                    evaporchain_allen_decay::AllenRelation::MetBy => 11,
                    evaporchain_allen_decay::AllenRelation::After => 12,
                };
                Ok(Value::U64(code))
            }

            "emit_event" => {
                if arg_count < 2 {
                    return Err(ScriptError::Runtime(
                        "emit_event() takes at least 2 arguments: name, data, [topics...]".into(),
                    ));
                }
                let name = self.pop()?.as_str()?.to_string();
                let data_val = self.pop()?;
                let mut topics = Vec::new();
                for _ in 0..(arg_count - 2) {
                    topics.push(self.pop()?);
                }
                topics.reverse();
                // OPCODE-5: track name + data + topic sizes before enqueuing.
                let event_bytes = name.len()
                    + match &data_val {
                        Value::Str(s) => s.len(),
                        _ => 8,
                    }
                    + topics
                        .iter()
                        .map(|v| match v {
                            Value::Str(s) => s.len(),
                            _ => 8,
                        })
                        .sum::<usize>();
                self.track_memory(event_bytes)?;
                self.events.push(format!("event:{name}"));
                self.structured_events.push(ContractEvent {
                    name,
                    topics,
                    data: vec![data_val],
                });
                Ok(Value::Null)
            }

            "require" => {
                if arg_count != 2 {
                    return Err(ScriptError::Runtime("require() takes 2 arguments".into()));
                }
                let msg = self.pop()?;
                let cond = self.pop()?.as_bool()?;
                if !cond {
                    let msg_str = match msg {
                        Value::Str(s) => s,
                        other => format!("{other}"),
                    };
                    return Err(ScriptError::RequireFailed(msg_str));
                }
                Ok(Value::Null)
            }

            // ── Math built-ins ──
            "min" => {
                if arg_count != 2 {
                    return Err(ScriptError::Runtime("min() takes 2 arguments".into()));
                }
                let b = self.pop()?.as_u64()?;
                let a = self.pop()?.as_u64()?;
                Ok(Value::U64(a.min(b)))
            }

            "max" => {
                if arg_count != 2 {
                    return Err(ScriptError::Runtime("max() takes 2 arguments".into()));
                }
                let b = self.pop()?.as_u64()?;
                let a = self.pop()?.as_u64()?;
                Ok(Value::U64(a.max(b)))
            }

            // ── Hashing ──
            "hash" => {
                if arg_count != 1 {
                    return Err(ScriptError::Runtime("hash() takes 1 argument".into()));
                }
                let val = self.pop()?;
                let input = match &val {
                    Value::Str(s) => s.as_bytes().to_vec(),
                    Value::U64(n) => n.to_le_bytes().to_vec(),
                    Value::Address(a) => a.to_vec(),
                    _ => format!("{val:?}").into_bytes(),
                };
                // M15 (audit 2026-05-13): size-scaled gas. The
                // baseline `GAS_CALL=10` for `Op::Call("hash", _)` was
                // already paid by the caller at the dispatch site;
                // this is the per-byte rider that makes a 1 MiB hash
                // actually cost more than a 4-byte hash. Without it a
                // contract could chain-stall the validator ~13 min
                // per block by looping `hash(megabyte_str)` while
                // gas_used stayed near 10⁷ × GAS_CALL = 10⁸ — under
                // the DEFAULT_GAS_LIMIT.
                let extra =
                    (input.len() as u64).div_ceil(32).saturating_mul(GAS_HASH_PER_32B);
                self.charge_gas(GAS_HASH_BASE.saturating_add(extra))?;
                // Use a simple hash → u64 for in-VM use
                let hash = {
                    let mut h: u64 = 0xcbf29ce484222325; // FNV-1a offset basis
                    for byte in &input {
                        h ^= *byte as u64;
                        h = h.wrapping_mul(0x100000001b3); // FNV prime
                    }
                    h
                };
                Ok(Value::U64(hash))
            }

            // ── String/collection utilities ──
            "len" => {
                if arg_count != 1 {
                    return Err(ScriptError::Runtime("len() takes 1 argument".into()));
                }
                let val = self.pop()?;
                let length = match &val {
                    Value::Str(s) => s.len() as u64,
                    Value::Map(m) => m.len() as u64,
                    Value::Array(a) => a.len() as u64,
                    _ => {
                        return Err(ScriptError::Runtime(format!(
                            "len() not supported for {val:?}"
                        )))
                    }
                };
                Ok(Value::U64(length))
            }

            "to_string" => {
                if arg_count != 1 {
                    return Err(ScriptError::Runtime("to_string() takes 1 argument".into()));
                }
                let val = self.pop()?;
                // M15 (audit 2026-05-13): same shape as `hash` — the
                // Display impl walks recursive Map / Array values
                // linearly. Charge per 32-byte slice of the input's
                // estimated byte footprint so a 1 MiB string can't be
                // converted for the same gas as a 4-byte one. The
                // estimate is computed BEFORE the format! so we don't
                // pay to_string()-builds-the-string and only then
                // charge for it.
                let est = estimated_to_string_bytes(&val);
                let extra = (est as u64).div_ceil(32).saturating_mul(GAS_TOSTRING_PER_32B);
                self.charge_gas(GAS_TOSTRING_BASE.saturating_add(extra))?;
                let s = match val {
                    Value::Str(s) => s,
                    other => format!("{other}"),
                };
                Ok(Value::Str(s))
            }

            // LOTTERY-1 (audit 2026-05-17): expose VRF beacon as callable
            // functions so contracts can bind randomness to on-chain state
            // rather than trusting operator-supplied proofs. Mirrors the
            // Op::VrfDomainRandomness / Op::RandomRange opcode semantics.
            "vrf_domain_randomness" => {
                if arg_count != 1 {
                    return Err(ScriptError::Runtime(
                        "vrf_domain_randomness() takes 1 argument (domain string)".into(),
                    ));
                }
                let domain = self.pop()?.as_str()?.to_string();
                let mut hasher = blake3::Hasher::new();
                hasher.update(b"EvaporChain_Beacon_Derive");
                hasher.update(&ctx.vrf_randomness);
                hasher.update(domain.as_bytes());
                let derived = hasher.finalize();
                let value = u64::from_le_bytes(derived.as_bytes()[..8].try_into().unwrap());
                Ok(Value::U64(value))
            }

            "random_range" => {
                if arg_count != 1 {
                    return Err(ScriptError::Runtime(
                        "random_range() takes 1 argument (max exclusive)".into(),
                    ));
                }
                let max = self.pop()?.as_u64()?;
                if max == 0 {
                    return Err(ScriptError::Runtime(
                        "random_range: max must be > 0".into(),
                    ));
                }
                const MAX_REJECTION_ITERS: u32 = 64;
                let zone = u64::MAX - (u64::MAX % max);
                let mut iter: u32 = 0;
                let value = loop {
                    let mut h = blake3::Hasher::new();
                    h.update(b"EvaporChain_RandomRange_Reject_v1");
                    h.update(&ctx.vrf_randomness);
                    h.update(&iter.to_le_bytes());
                    let derived = h.finalize();
                    let raw =
                        u64::from_le_bytes(derived.as_bytes()[..8].try_into().unwrap());
                    if raw < zone || iter >= MAX_REJECTION_ITERS {
                        break raw % max;
                    }
                    iter += 1;
                };
                Ok(Value::U64(value))
            }

            other => Err(ScriptError::Runtime(format!(
                "unknown built-in function: {other}"
            ))),
        }
    }

    /// Execute a method on compiled bytecode with the default gas limit.
    pub fn execute(
        bytecode: &EvaporBytecode,
        method: &str,
        args: Vec<Value>,
        state: HashMap<String, Value>,
        ctx: &ExecutionContext,
    ) -> Result<ScriptCallResult, ScriptError> {
        Self::execute_full(bytecode, method, args, state, ctx, DEFAULT_GAS_LIMIT, None)
    }

    /// Execute with an explicit gas limit. The limit is always enforced;
    /// passing 0 will cause immediate gas exhaustion on the first opcode.
    pub fn execute_with_gas_limit(
        bytecode: &EvaporBytecode,
        method: &str,
        args: Vec<Value>,
        state: HashMap<String, Value>,
        ctx: &ExecutionContext,
        gas_limit: u64,
    ) -> Result<ScriptCallResult, ScriptError> {
        Self::execute_full(bytecode, method, args, state, ctx, gas_limit, None)
    }

    /// Execute with full options including cross-contract call support.
    ///
    /// SCR-N1: callers wanting the cross-contract `caller` identity to
    /// be propagated correctly should use [`Self::execute_full_with_self`]
    /// and pass the contract_id this bytecode belongs to. This entry
    /// point keeps `executing_contract_id = 0` for backward
    /// compatibility — top-level test fixtures and lifecycle hooks
    /// continue to pass `ctx.caller` unchanged to nested calls.
    pub fn execute_full(
        bytecode: &EvaporBytecode,
        method: &str,
        args: Vec<Value>,
        state: HashMap<String, Value>,
        ctx: &ExecutionContext,
        gas_limit: u64,
        external: Option<&mut dyn ExternalCaller>,
    ) -> Result<ScriptCallResult, ScriptError> {
        Self::execute_full_with_self(bytecode, method, args, state, ctx, gas_limit, external, 0)
    }

    /// SCR-N1 (audit 2026-05-15): identical to [`execute_full`] but
    /// records `executing_contract_id` on the VM so `Op::CallExternal`
    /// can pass the *calling contract's* identity to the callee. Used
    /// by `ScriptEngine::call_with_vm_gas` (top-level dispatch) and
    /// `ContractCallRouter::call_external` (nested dispatch).
    #[allow(clippy::too_many_arguments)]
    pub fn execute_full_with_self(
        bytecode: &EvaporBytecode,
        method: &str,
        args: Vec<Value>,
        state: HashMap<String, Value>,
        ctx: &ExecutionContext,
        gas_limit: u64,
        mut external: Option<&mut dyn ExternalCaller>,
        executing_contract_id: u64,
    ) -> Result<ScriptCallResult, ScriptError> {
        let mut vm = Self::new(state, gas_limit);
        vm.executing_contract_id = executing_contract_id;
        // M6 (audit 2026-05-02): mark hook entry so `Op::CallExternal`
        // will refuse to dispatch — closes the lifecycle re-entrancy
        // window where `on_grace`/`on_refresh`/`on_evaporate` could
        // make cross-contract calls that recursed back into another
        // hook on the same object.
        vm.in_lifecycle_hook = Self::is_lifecycle_hook(method);
        let return_value = vm.execute_method(bytecode, method, args, ctx, &mut external)?;

        Ok(ScriptCallResult {
            return_value,
            events: vm.events,
            structured_events: vm.structured_events,
            gas_used: vm.gas_used,
            state_changes: vm.state,
        })
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{compiler, parser};

    fn test_ctx() -> ExecutionContext {
        ExecutionContext {
            caller: [1u8; 32],
            owner: [2u8; 32],
            epoch: 100,
            energy: 5000,
            vrf_randomness: [42u8; 32],
            call_depth: 0,
        }
    }

    fn compile_src(src: &str) -> EvaporBytecode {
        let ast = parser::parse(src).unwrap();
        compiler::compile(&ast).unwrap()
    }

    fn empty_state() -> HashMap<String, Value> {
        HashMap::new()
    }

    /// Build a minimal bytecode with a single method from raw opcodes.
    fn make_bytecode(method: &str, opcodes: Vec<Op>) -> EvaporBytecode {
        let mut methods = HashMap::new();
        methods.insert(method.to_string(), 0);
        EvaporBytecode {
            methods,
            opcodes,
            state_schema: crate::StateSchema { fields: vec![] },
            name: "Test".to_string(),
        }
    }

    #[test]
    fn test_vm_push_add_return() {
        let src = r#"
contract Math {
    state { x: u64 = 0 }
    fn add(a: u64, b: u64) -> u64 {
        return a + b
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();
        let result = EvaporVM::execute(
            &bytecode,
            "add",
            vec![Value::U64(30), Value::U64(12)],
            empty_state(),
            &ctx,
        )
        .unwrap();

        assert_eq!(result.return_value, Value::U64(42));
        assert!(result.gas_used > 0);
    }

    #[test]
    fn test_vm_variable_load_store() {
        let src = r#"
contract Vars {
    state { x: u64 = 0 }
    fn compute(a: u64, b: u64) -> u64 {
        let sum = a + b
        let doubled = sum * 2
        return doubled
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();
        let result = EvaporVM::execute(
            &bytecode,
            "compute",
            vec![Value::U64(5), Value::U64(3)],
            empty_state(),
            &ctx,
        )
        .unwrap();

        assert_eq!(result.return_value, Value::U64(16));
    }

    #[test]
    fn test_vm_require_passes() {
        let src = r#"
contract Auth {
    state { x: u64 = 0 }
    fn check(val: u64) -> u64 {
        require(val > 0, "must be positive")
        return val
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();
        let result = EvaporVM::execute(
            &bytecode,
            "check",
            vec![Value::U64(42)],
            empty_state(),
            &ctx,
        )
        .unwrap();

        assert_eq!(result.return_value, Value::U64(42));
    }

    #[test]
    fn test_vm_require_fails_and_reverts() {
        let src = r#"
contract Auth {
    state { x: u64 = 0 }
    fn check(val: u64) -> u64 {
        require(val > 0, "must be positive")
        return val
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();
        let result =
            EvaporVM::execute(&bytecode, "check", vec![Value::U64(0)], empty_state(), &ctx);

        match result {
            Err(ScriptError::RequireFailed(msg)) => {
                assert_eq!(msg, "must be positive");
            }
            other => panic!("expected RequireFailed, got {other:?}"),
        }
    }

    #[test]
    fn test_vm_map_get_set() {
        let src = r#"
contract Store {
    state {
        data: map[address -> u64]
    }
    fn set(key: address, val: u64) {
        self.data[key] = val
    }
    fn get(key: address) -> u64 {
        return self.data[key]
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();

        // Set a value
        let mut state = HashMap::new();
        state.insert("data".into(), Value::Map(HashMap::new()));

        let result = EvaporVM::execute(
            &bytecode,
            "set",
            vec![Value::Address([5u8; 32]), Value::U64(999)],
            state,
            &ctx,
        )
        .unwrap();

        // Now get it back
        let result2 = EvaporVM::execute(
            &bytecode,
            "get",
            vec![Value::Address([5u8; 32])],
            result.state_changes,
            &ctx,
        )
        .unwrap();

        assert_eq!(result2.return_value, Value::U64(999));
    }

    #[test]
    fn test_vm_if_else_branching() {
        let src = r#"
contract Branch {
    state { x: u64 = 0 }
    fn classify(val: u64) -> u64 {
        if val > 100 {
            return 2
        } else {
            if val > 50 {
                return 1
            } else {
                return 0
            }
        }
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();

        let r1 = EvaporVM::execute(
            &bytecode,
            "classify",
            vec![Value::U64(150)],
            empty_state(),
            &ctx,
        )
        .unwrap();
        assert_eq!(r1.return_value, Value::U64(2));

        let r2 = EvaporVM::execute(
            &bytecode,
            "classify",
            vec![Value::U64(75)],
            empty_state(),
            &ctx,
        )
        .unwrap();
        assert_eq!(r2.return_value, Value::U64(1));

        let r3 = EvaporVM::execute(
            &bytecode,
            "classify",
            vec![Value::U64(25)],
            empty_state(),
            &ctx,
        )
        .unwrap();
        assert_eq!(r3.return_value, Value::U64(0));
    }

    #[test]
    fn test_vm_gas_limit_exceeded() {
        let src = r#"
contract Expensive {
    state { x: u64 = 0 }
    fn work(a: u64, b: u64) -> u64 {
        let c = a + b
        let d = c * a
        let e = d + b
        let f = e * c
        return f
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();

        // Very low gas limit
        let result = EvaporVM::execute_with_gas_limit(
            &bytecode,
            "work",
            vec![Value::U64(5), Value::U64(3)],
            empty_state(),
            &ctx,
            5, // impossibly low
        );

        match result {
            Err(ScriptError::GasLimitExceeded { used, limit }) => {
                assert!(used > limit);
                assert_eq!(limit, 5);
            }
            other => panic!("expected GasLimitExceeded, got {other:?}"),
        }
    }

    #[test]
    fn test_vm_emit_events() {
        let src = r#"
contract Events {
    state { x: u64 = 0 }
    fn fire() {
        emit("hello world")
        emit("second event")
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();
        let result = EvaporVM::execute(&bytecode, "fire", vec![], empty_state(), &ctx).unwrap();

        assert_eq!(result.events.len(), 2);
        assert_eq!(result.events[0], "hello world");
        assert_eq!(result.events[1], "second event");
    }

    #[test]
    fn test_vm_allen_relation_builtin() {
        // Allen-Decay built-in (research-buildable item #9). Returns the
        // 13-relation interval-algebra code as u64. Code 0 = Before:
        // [0, 5) is Before [10, 20). Code 6 = Equals: [3, 7) Equals
        // [3, 7). Code 12 = After: [50, 60) is After [10, 20).
        let src = r#"
contract Allen {
    state { x: u64 = 0 }
    fn before() -> u64 {
        return allen_relation(0, 5, 10, 20)
    }
    fn equals() -> u64 {
        return allen_relation(3, 7, 3, 7)
    }
    fn after_rel() -> u64 {
        return allen_relation(50, 60, 10, 20)
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();

        let r1 = EvaporVM::execute(&bytecode, "before", vec![], empty_state(), &ctx).unwrap();
        assert_eq!(r1.return_value, Value::U64(0), "Before encodes as 0");

        let r2 = EvaporVM::execute(&bytecode, "equals", vec![], empty_state(), &ctx).unwrap();
        assert_eq!(r2.return_value, Value::U64(6), "Equals encodes as 6");

        let r3 = EvaporVM::execute(&bytecode, "after_rel", vec![], empty_state(), &ctx).unwrap();
        assert_eq!(r3.return_value, Value::U64(12), "After encodes as 12");
    }

    #[test]
    fn test_vm_allen_relation_all_thirteen_codes() {
        // Exhaustive test for the Allen-Decay VM built-in: every one of
        // the 13 interval relations must round-trip through EvaporScript
        // to the correct numeric code. The reference inputs match
        // `evaporchain_allen_decay::compute::tests` exactly.
        //
        // Codes: 0 Before, 1 Meets, 2 Overlaps, 3 Starts, 4 During,
        // 5 Finishes, 6 Equals, 7 FinishedBy, 8 Contains, 9 StartedBy,
        // 10 OverlappedBy, 11 MetBy, 12 After.
        let src = r#"
contract AllenAll {
    state { x: u64 = 0 }
    fn rel(a_s: u64, a_e: u64, b_s: u64, b_e: u64) -> u64 {
        return allen_relation(a_s, a_e, b_s, b_e)
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();

        // (input_a_start, input_a_end, input_b_start, input_b_end, expected_code)
        let cases: [(u64, u64, u64, u64, u64); 13] = [
            (1, 3, 5, 7, 0),  // Before
            (1, 5, 5, 7, 1),  // Meets
            (1, 5, 3, 7, 2),  // Overlaps
            (1, 3, 1, 7, 3),  // Starts
            (2, 5, 1, 7, 4),  // During
            (3, 7, 1, 7, 5),  // Finishes
            (1, 5, 1, 5, 6),  // Equals
            (1, 7, 3, 7, 7),  // FinishedBy
            (1, 7, 3, 5, 8),  // Contains
            (1, 7, 1, 3, 9),  // StartedBy
            (3, 7, 1, 5, 10), // OverlappedBy
            (5, 7, 1, 5, 11), // MetBy
            (5, 7, 1, 3, 12), // After
        ];

        for (i, (a_s, a_e, b_s, b_e, expected)) in cases.iter().enumerate() {
            let args = vec![
                Value::U64(*a_s),
                Value::U64(*a_e),
                Value::U64(*b_s),
                Value::U64(*b_e),
            ];
            let r = EvaporVM::execute(&bytecode, "rel", args, empty_state(), &ctx).unwrap();
            assert_eq!(
                r.return_value,
                Value::U64(*expected),
                "case #{i}: relation(({a_s},{a_e}), ({b_s},{b_e})) — \
                 expected code {expected}"
            );
        }
    }

    #[test]
    fn test_vm_allen_relation_invalid_interval_rejected() {
        // start_a >= end_a must be rejected by the underlying Interval::new.
        let src = r#"
contract AllenBad {
    state { x: u64 = 0 }
    fn bad() -> u64 {
        return allen_relation(10, 5, 0, 1)
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();
        let err = EvaporVM::execute(&bytecode, "bad", vec![], empty_state(), &ctx);
        assert!(err.is_err(), "invalid interval must error");
    }

    #[test]
    fn test_vm_state_persistence() {
        let src = r#"
contract Counter {
    state { count: u64 = 0 }
    fn increment(n: u64) {
        self.count += n
    }
    fn get() -> u64 {
        return self.count
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();

        let mut state = HashMap::new();
        state.insert("count".into(), Value::U64(0));

        // Increment
        let r1 =
            EvaporVM::execute(&bytecode, "increment", vec![Value::U64(5)], state, &ctx).unwrap();

        // Get — use state from previous execution
        let r2 = EvaporVM::execute(&bytecode, "get", vec![], r1.state_changes, &ctx).unwrap();
        assert_eq!(r2.return_value, Value::U64(5));
    }

    #[test]
    fn test_vm_caller_and_owner_builtins() {
        let src = r#"
contract Context {
    state { x: u64 = 0 }
    fn is_owner() -> bool {
        return caller == owner
    }
}
"#;
        let bytecode = compile_src(src);

        // caller != owner
        let ctx1 = ExecutionContext {
            caller: [1u8; 32],
            owner: [2u8; 32],
            epoch: 0,
            energy: 0,
            vrf_randomness: [0u8; 32],
            call_depth: 0,
        };
        let r1 = EvaporVM::execute(&bytecode, "is_owner", vec![], empty_state(), &ctx1).unwrap();
        assert_eq!(r1.return_value, Value::Bool(false));

        // caller == owner
        let ctx2 = ExecutionContext {
            caller: [1u8; 32],
            owner: [1u8; 32],
            epoch: 0,
            energy: 0,
            vrf_randomness: [0u8; 32],
            call_depth: 0,
        };
        let r2 = EvaporVM::execute(&bytecode, "is_owner", vec![], empty_state(), &ctx2).unwrap();
        assert_eq!(r2.return_value, Value::Bool(true));
    }

    #[test]
    fn test_vm_division_by_zero() {
        let src = r#"
contract Math {
    state { x: u64 = 0 }
    fn divide(a: u64, b: u64) -> u64 {
        return a / b
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();
        let result = EvaporVM::execute(
            &bytecode,
            "divide",
            vec![Value::U64(10), Value::U64(0)],
            empty_state(),
            &ctx,
        );

        assert!(result.is_err());
        assert!(format!("{}", result.unwrap_err()).contains("division by zero"));
    }

    #[test]
    fn test_vm_gas_metering_accurate() {
        let src = r#"
contract GasTest {
    state { x: u64 = 0 }
    fn simple(a: u64) -> u64 {
        return a
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();
        let result = EvaporVM::execute(
            &bytecode,
            "simple",
            vec![Value::U64(42)],
            empty_state(),
            &ctx,
        )
        .unwrap();

        // Store("a")=2 + Load("a")=2 + Return=1 = 5
        // Then the implicit Push(Null)+Return after the explicit return aren't reached
        assert_eq!(result.gas_used, 5);
    }

    // ─── Full Pipeline Tests ────────────────────────────────────────────────

    #[test]
    fn test_full_pipeline_loyalty_points() {
        let src = r#"
contract LoyaltyPoints {
    state {
        name: string = "ShopPoints"
        points: map[address -> u64]
        total_issued: u64 = 0
    }

    fn issue(to: address, amount: u64) {
        require(caller == owner, "only owner")
        self.points[to] += amount
        self.total_issued += amount
    }

    fn spend(amount: u64) {
        require(self.points[caller] >= amount, "insufficient points")
        self.points[caller] -= amount
    }

    fn balance(addr: address) -> u64 {
        return self.points[addr]
    }

    on_evaporate() {
        emit("loyalty program expired")
    }
}
"#;
        let bytecode = compile_src(src);
        let owner = [2u8; 32];
        let user = [3u8; 32];

        let ctx_owner = ExecutionContext {
            caller: owner,
            owner,
            epoch: 100,
            energy: 5000,
            vrf_randomness: [0u8; 32],
            call_depth: 0,
        };

        // Initialize state
        let mut state = HashMap::new();
        state.insert("name".into(), Value::Str("ShopPoints".into()));
        state.insert("points".into(), Value::Map(HashMap::new()));
        state.insert("total_issued".into(), Value::U64(0));

        // Issue 100 points to user
        let r1 = EvaporVM::execute(
            &bytecode,
            "issue",
            vec![Value::Address(user), Value::U64(100)],
            state,
            &ctx_owner,
        )
        .unwrap();

        // Check balance
        let r2 = EvaporVM::execute(
            &bytecode,
            "balance",
            vec![Value::Address(user)],
            r1.state_changes.clone(),
            &ctx_owner,
        )
        .unwrap();
        assert_eq!(r2.return_value, Value::U64(100));

        // Check total_issued
        assert_eq!(r1.state_changes.get("total_issued"), Some(&Value::U64(100)));
    }

    #[test]
    fn test_full_pipeline_issue_and_spend() {
        let src = r#"
contract LoyaltyPoints {
    state {
        points: map[address -> u64]
        total_issued: u64 = 0
    }

    fn issue(to: address, amount: u64) {
        require(caller == owner, "only owner")
        self.points[to] += amount
        self.total_issued += amount
    }

    fn spend(amount: u64) {
        require(self.points[caller] >= amount, "insufficient points")
        self.points[caller] -= amount
    }

    fn balance(addr: address) -> u64 {
        return self.points[addr]
    }
}
"#;
        let bytecode = compile_src(src);
        let owner = [2u8; 32];
        let user = [3u8; 32];

        let ctx_owner = ExecutionContext {
            caller: owner,
            owner,
            epoch: 100,
            energy: 5000,
            vrf_randomness: [0u8; 32],
            call_depth: 0,
        };
        let ctx_user = ExecutionContext {
            caller: user,
            owner,
            epoch: 100,
            energy: 5000,
            vrf_randomness: [0u8; 32],
            call_depth: 0,
        };

        let mut state = HashMap::new();
        state.insert("points".into(), Value::Map(HashMap::new()));
        state.insert("total_issued".into(), Value::U64(0));

        // Issue 200 points
        let r1 = EvaporVM::execute(
            &bytecode,
            "issue",
            vec![Value::Address(user), Value::U64(200)],
            state,
            &ctx_owner,
        )
        .unwrap();

        // Spend 75 as user
        let r2 = EvaporVM::execute(
            &bytecode,
            "spend",
            vec![Value::U64(75)],
            r1.state_changes,
            &ctx_user,
        )
        .unwrap();

        // Check balance = 125
        let r3 = EvaporVM::execute(
            &bytecode,
            "balance",
            vec![Value::Address(user)],
            r2.state_changes,
            &ctx_user,
        )
        .unwrap();
        assert_eq!(r3.return_value, Value::U64(125));
    }

    #[test]
    fn test_full_pipeline_on_evaporate_hook() {
        let src = r#"
contract Expiring {
    state { x: u64 = 0 }
    on_evaporate() {
        emit("contract expired")
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();
        let result =
            EvaporVM::execute(&bytecode, "on_evaporate", vec![], empty_state(), &ctx).unwrap();

        assert_eq!(result.events, vec!["contract expired"]);
    }

    #[test]
    fn test_full_pipeline_on_grace_hook() {
        let src = r#"
contract GraceAware {
    state { x: u64 = 0 }
    on_grace() {
        emit("entering grace period")
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();
        let result = EvaporVM::execute(&bytecode, "on_grace", vec![], empty_state(), &ctx).unwrap();

        assert_eq!(result.events, vec!["entering grace period"]);
    }

    // ═══════════════════════════════════════════════════════════════
    // Phase 4: New Feature Tests
    // ═══════════════════════════════════════════════════════════════

    #[test]
    fn test_vm_checked_add_overflow() {
        let ops = vec![
            Op::Push(Value::U64(u64::MAX)),
            Op::Push(Value::U64(1)),
            Op::Add,
            Op::Return,
        ];
        let bytecode = make_bytecode("run", ops);
        let result = EvaporVM::execute(&bytecode, "run", vec![], empty_state(), &test_ctx());
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            ScriptError::Runtime(msg) => assert!(msg.contains("overflow"), "got: {msg}"),
            _ => panic!("expected Runtime error, got {err:?}"),
        }
    }

    #[test]
    fn test_vm_checked_sub_underflow() {
        let ops = vec![
            Op::Push(Value::U64(5)),
            Op::Push(Value::U64(10)),
            Op::Sub,
            Op::Return,
        ];
        let bytecode = make_bytecode("run", ops);
        let result = EvaporVM::execute(&bytecode, "run", vec![], empty_state(), &test_ctx());
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            ScriptError::Runtime(msg) => assert!(msg.contains("underflow"), "got: {msg}"),
            _ => panic!("expected Runtime error, got {err:?}"),
        }
    }

    #[test]
    fn test_vm_checked_mul_overflow() {
        let ops = vec![
            Op::Push(Value::U64(u64::MAX)),
            Op::Push(Value::U64(2)),
            Op::Mul,
            Op::Return,
        ];
        let bytecode = make_bytecode("run", ops);
        let result = EvaporVM::execute(&bytecode, "run", vec![], empty_state(), &test_ctx());
        assert!(result.is_err());
    }

    #[test]
    fn test_vm_modulo() {
        let ops = vec![
            Op::Push(Value::U64(17)),
            Op::Push(Value::U64(5)),
            Op::Mod,
            Op::Return,
        ];
        let bytecode = make_bytecode("run", ops);
        let result =
            EvaporVM::execute(&bytecode, "run", vec![], empty_state(), &test_ctx()).unwrap();
        assert_eq!(result.return_value, Value::U64(2));
    }

    #[test]
    fn test_vm_modulo_by_zero() {
        let ops = vec![
            Op::Push(Value::U64(10)),
            Op::Push(Value::U64(0)),
            Op::Mod,
            Op::Return,
        ];
        let bytecode = make_bytecode("run", ops);
        let result = EvaporVM::execute(&bytecode, "run", vec![], empty_state(), &test_ctx());
        assert!(result.is_err());
    }

    #[test]
    fn test_vm_builtin_min_max() {
        // min(10, 20) = 10
        let ops = vec![
            Op::Push(Value::U64(10)),
            Op::Push(Value::U64(20)),
            Op::Call("min".into(), 2),
            Op::Return,
        ];
        let bytecode = make_bytecode("run", ops);
        let r = EvaporVM::execute(&bytecode, "run", vec![], empty_state(), &test_ctx()).unwrap();
        assert_eq!(r.return_value, Value::U64(10));

        // max(10, 20) = 20
        let ops2 = vec![
            Op::Push(Value::U64(10)),
            Op::Push(Value::U64(20)),
            Op::Call("max".into(), 2),
            Op::Return,
        ];
        let bytecode2 = make_bytecode("run", ops2);
        let r2 = EvaporVM::execute(&bytecode2, "run", vec![], empty_state(), &test_ctx()).unwrap();
        assert_eq!(r2.return_value, Value::U64(20));
    }

    #[test]
    fn test_vm_builtin_hash() {
        let ops = vec![
            Op::Push(Value::Str("hello".into())),
            Op::Call("hash".into(), 1),
            Op::Return,
        ];
        let bytecode = make_bytecode("run", ops);
        let r = EvaporVM::execute(&bytecode, "run", vec![], empty_state(), &test_ctx()).unwrap();
        match r.return_value {
            Value::U64(h) => assert!(h != 0, "hash should be non-zero"),
            other => panic!("expected U64, got {other:?}"),
        }
    }

    #[test]
    fn test_vm_builtin_len() {
        let ops = vec![
            Op::Push(Value::Str("hello".into())),
            Op::Call("len".into(), 1),
            Op::Return,
        ];
        let bytecode = make_bytecode("run", ops);
        let r = EvaporVM::execute(&bytecode, "run", vec![], empty_state(), &test_ctx()).unwrap();
        assert_eq!(r.return_value, Value::U64(5));
    }

    #[test]
    fn test_vm_builtin_to_string() {
        let ops = vec![
            Op::Push(Value::U64(42)),
            Op::Call("to_string".into(), 1),
            Op::Return,
        ];
        let bytecode = make_bytecode("run", ops);
        let r = EvaporVM::execute(&bytecode, "run", vec![], empty_state(), &test_ctx()).unwrap();
        assert_eq!(r.return_value, Value::Str("42".into()));
    }

    #[test]
    fn test_vm_stack_overflow() {
        // Push 1025 values to exceed MAX_STACK_DEPTH
        let mut ops: Vec<Op> = (0..1025).map(|_| Op::Push(Value::U64(1))).collect();
        ops.push(Op::Return);
        let bytecode = make_bytecode("run", ops);
        let result = EvaporVM::execute(&bytecode, "run", vec![], empty_state(), &test_ctx());
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            ScriptError::Runtime(msg) => assert!(msg.contains("stack overflow"), "got: {msg}"),
            _ => panic!("expected Runtime error, got {err:?}"),
        }
    }

    #[test]
    fn test_full_pipeline_while_loop() {
        let src = r#"
contract Counter {
    state { total: u64 = 0 }

    fn sum_to(n: u64) -> u64 {
        let i: u64 = 0
        let acc: u64 = 0
        while i < n {
            i += 1
            acc += i
        }
        self.total = acc
        return acc
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();
        let mut state = HashMap::new();
        state.insert("total".into(), Value::U64(0));

        let result =
            EvaporVM::execute(&bytecode, "sum_to", vec![Value::U64(10)], state, &ctx).unwrap();
        // sum 1..=10 = 55
        assert_eq!(result.return_value, Value::U64(55));
        assert_eq!(result.state_changes.get("total"), Some(&Value::U64(55)));
    }

    #[test]
    fn test_full_pipeline_while_with_gas_limit() {
        let src = r#"
contract Looper {
    state { x: u64 = 0 }

    fn loop_forever() {
        let i: u64 = 0
        while i < 1000000 {
            i += 1
        }
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();
        let state = HashMap::new();

        // With a tight gas limit, the loop should run out of gas
        let result =
            EvaporVM::execute_with_gas_limit(&bytecode, "loop_forever", vec![], state, &ctx, 500);
        assert!(result.is_err());
    }

    #[test]
    fn test_full_pipeline_modulo_operator() {
        let src = r#"
contract Math {
    state { x: u64 = 0 }

    fn is_even(n: u64) -> bool {
        return n % 2 == 0
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();

        let r1 = EvaporVM::execute(
            &bytecode,
            "is_even",
            vec![Value::U64(4)],
            HashMap::new(),
            &ctx,
        )
        .unwrap();
        assert_eq!(r1.return_value, Value::Bool(true));

        let r2 = EvaporVM::execute(
            &bytecode,
            "is_even",
            vec![Value::U64(7)],
            HashMap::new(),
            &ctx,
        )
        .unwrap();
        assert_eq!(r2.return_value, Value::Bool(false));
    }

    #[test]
    fn test_full_pipeline_checked_arithmetic_in_contract() {
        // A contract that tries to withdraw more than balance should fail
        let src = r#"
contract Vault {
    state { balance: u64 = 100 }

    fn withdraw(amount: u64) {
        self.balance -= amount
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();
        let mut state = HashMap::new();
        state.insert("balance".into(), Value::U64(100));

        // Withdraw 50 — should succeed
        let r1 = EvaporVM::execute(
            &bytecode,
            "withdraw",
            vec![Value::U64(50)],
            state.clone(),
            &ctx,
        )
        .unwrap();
        assert_eq!(r1.state_changes.get("balance"), Some(&Value::U64(50)));

        // Withdraw 200 from 100 — should fail with underflow
        let r2 = EvaporVM::execute(&bytecode, "withdraw", vec![Value::U64(200)], state, &ctx);
        assert!(r2.is_err(), "should fail: 100 - 200 underflows");
    }

    // ═══════════════════════════════════════════════════════════════
    // Array Tests
    // ═══════════════════════════════════════════════════════════════

    #[test]
    fn test_array_literal_and_access() {
        let src = r#"
contract Arrays {
    state { x: u64 = 0 }
    fn get_second() -> u64 {
        let arr = [10, 20, 30]
        return arr[1]
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();
        let result =
            EvaporVM::execute(&bytecode, "get_second", vec![], empty_state(), &ctx).unwrap();
        assert_eq!(result.return_value, Value::U64(20));
    }

    #[test]
    fn test_array_set_element() {
        let src = r#"
contract Arrays {
    state { x: u64 = 0 }
    fn modify() -> u64 {
        let arr = [1, 2, 3]
        arr[0] = 99
        return arr[0]
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();
        let result = EvaporVM::execute(&bytecode, "modify", vec![], empty_state(), &ctx).unwrap();
        assert_eq!(result.return_value, Value::U64(99));
    }

    #[test]
    fn test_array_compound_assign() {
        let src = r#"
contract Arrays {
    state { x: u64 = 0 }
    fn add_to_element() -> u64 {
        let arr = [10, 20, 30]
        arr[1] += 5
        return arr[1]
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();
        let result =
            EvaporVM::execute(&bytecode, "add_to_element", vec![], empty_state(), &ctx).unwrap();
        assert_eq!(result.return_value, Value::U64(25));
    }

    #[test]
    fn test_array_len_builtin() {
        let src = r#"
contract Arrays {
    state { x: u64 = 0 }
    fn count() -> u64 {
        let arr = [1, 2, 3, 4, 5]
        return len(arr)
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();
        let result = EvaporVM::execute(&bytecode, "count", vec![], empty_state(), &ctx).unwrap();
        assert_eq!(result.return_value, Value::U64(5));
    }

    #[test]
    fn test_array_out_of_bounds() {
        let src = r#"
contract Arrays {
    state { x: u64 = 0 }
    fn oob() -> u64 {
        let arr = [1, 2, 3]
        return arr[5]
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();
        let result = EvaporVM::execute(&bytecode, "oob", vec![], empty_state(), &ctx);
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("out of bounds"), "got: {err}");
    }

    #[test]
    fn test_array_empty_literal() {
        let src = r#"
contract Arrays {
    state { x: u64 = 0 }
    fn empty_len() -> u64 {
        let arr = []
        return len(arr)
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();
        let result =
            EvaporVM::execute(&bytecode, "empty_len", vec![], empty_state(), &ctx).unwrap();
        assert_eq!(result.return_value, Value::U64(0));
    }

    #[test]
    fn test_state_array_access() {
        let src = r#"
contract WithStateArray {
    state {
        items: array[u64]
    }
    fn get_item(idx: u64) -> u64 {
        return self.items[idx]
    }
    fn set_item(idx: u64, val: u64) {
        self.items[idx] = val
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();

        let mut state = HashMap::new();
        state.insert(
            "items".into(),
            Value::Array(vec![Value::U64(100), Value::U64(200), Value::U64(300)]),
        );

        let r1 = EvaporVM::execute(
            &bytecode,
            "get_item",
            vec![Value::U64(1)],
            state.clone(),
            &ctx,
        )
        .unwrap();
        assert_eq!(r1.return_value, Value::U64(200));

        let r2 = EvaporVM::execute(
            &bytecode,
            "set_item",
            vec![Value::U64(0), Value::U64(999)],
            state,
            &ctx,
        )
        .unwrap();
        let updated_items = r2.state_changes.get("items").unwrap();
        match updated_items {
            Value::Array(arr) => assert_eq!(arr[0], Value::U64(999)),
            other => panic!("expected array, got {other:?}"),
        }
    }

    // ── L8 (audit 2026-05-13): Op::EmitEvent safety ──

    /// `topic_count > MAX_EVENT_TOPICS` is rejected at runtime
    /// (cleanly errors rather than allocating a giant Vec).
    #[test]
    fn audit_l8_emit_event_rejects_oversize_topic_count() {
        let ops = vec![
            // Push something so pop is well-formed if we got past the cap check.
            Op::Push(Value::U64(0)),
            Op::EmitEvent {
                name: "Spam".into(),
                topic_count: MAX_EVENT_TOPICS + 1,
            },
            Op::Return,
        ];
        let bytecode = make_bytecode("run", ops);
        let result = EvaporVM::execute(&bytecode, "run", vec![], empty_state(), &test_ctx());
        assert!(result.is_err());
        let err = format!("{:?}", result.unwrap_err());
        assert!(
            err.contains("MAX_EVENT_TOPICS"),
            "expected cap-exceeded error, got: {err}"
        );
    }

    /// usize::MAX as topic_count must surface as the checked-add error
    /// (not panic on overflow). Belt-and-braces — the cap check above
    /// already catches values >= cap+1; this proves the second guard
    /// for the cap-equals-usize::MAX hypothetical (impossible today
    /// since the cap < usize::MAX, but the checked_add stays as
    /// defense-in-depth).
    #[test]
    fn audit_l8_emit_event_topic_count_overflow_rejected() {
        let ops = vec![
            Op::EmitEvent {
                name: "Spam".into(),
                topic_count: usize::MAX,
            },
        ];
        let bytecode = make_bytecode("run", ops);
        let result = EvaporVM::execute(&bytecode, "run", vec![], empty_state(), &test_ctx());
        assert!(result.is_err());
    }

    /// Normal-size EmitEvent still works (sanity).
    #[test]
    fn audit_l8_emit_event_normal_topic_count_works() {
        let ops = vec![
            Op::Push(Value::U64(1)), // topic[0]
            Op::Push(Value::U64(2)), // topic[1]
            Op::Push(Value::Str("payload".into())), // data
            Op::EmitEvent {
                name: "Ok".into(),
                topic_count: 2,
            },
            Op::Return,
        ];
        let bytecode = make_bytecode("run", ops);
        let r = EvaporVM::execute(&bytecode, "run", vec![], empty_state(), &test_ctx()).unwrap();
        assert_eq!(r.structured_events.len(), 1);
        assert_eq!(r.structured_events[0].name, "Ok");
        assert_eq!(r.structured_events[0].topics.len(), 2);
        assert_eq!(r.structured_events[0].data.len(), 1);
    }

    // ── M15 (audit 2026-05-13): size-scaled gas for hash() + to_string() ──

    /// 1 MiB string hashed in a tight loop must run out of gas long
    /// before the audit's ~13-min chain-stall budget. Pre-fix: GAS_CALL
    /// = 10 charged once per hash → 10⁶ hashes fit under
    /// DEFAULT_GAS_LIMIT = 10⁷. Post-fix: every hash charges
    /// GAS_HASH_BASE + ceil(1 MiB / 32) * GAS_HASH_PER_32B ≈ 32 778
    /// gas, so DEFAULT_GAS_LIMIT caps the loop at ~305 iterations.
    #[test]
    fn audit_m15_hash_megabyte_input_runs_out_of_gas_in_loop() {
        let big: String = "a".repeat(1_048_576); // 1 MiB
        let mut ops: Vec<Op> = Vec::new();
        // Loop body: push the megabyte string, hash it, pop the result.
        for _ in 0..10_000 {
            ops.push(Op::Push(Value::Str(big.clone())));
            ops.push(Op::Call("hash".into(), 1));
            ops.push(Op::Pop);
        }
        ops.push(Op::Push(Value::U64(0)));
        ops.push(Op::Return);
        let bytecode = make_bytecode("spam", ops);
        let result =
            EvaporVM::execute(&bytecode, "spam", vec![], empty_state(), &test_ctx());
        assert!(result.is_err(), "expected gas exhaustion");
        let err = format!("{:?}", result.unwrap_err());
        assert!(
            err.contains("gas") || err.contains("Gas") || err.contains("memory"),
            "expected gas/memory error, got: {err}"
        );
    }

    /// Small inputs still hash cheaply — the per-32B rider only adds
    /// 1 gas for a sub-32-byte payload.
    #[test]
    fn audit_m15_hash_short_input_still_cheap() {
        let ops = vec![
            Op::Push(Value::Str("hi".into())),
            Op::Call("hash".into(), 1),
            Op::Return,
        ];
        let bytecode = make_bytecode("run", ops);
        let r = EvaporVM::execute(&bytecode, "run", vec![], empty_state(), &test_ctx())
            .unwrap();
        match r.return_value {
            Value::U64(_) => {}
            other => panic!("expected U64 hash, got {other:?}"),
        }
        // Sanity: total gas burned is dominated by the per-32B rider
        // for a 2-byte input → ~3 (push+call dispatch) + GAS_HASH_BASE
        // (10) + 1 (rider for 2 bytes ≤ 32) + GAS_RETURN. Should fit
        // comfortably under 50 gas.
        assert!(r.gas_used < 200, "got {} gas — expected ≪200", r.gas_used);
    }

    /// `to_string` on a 1 MiB string must charge ~the same envelope
    /// as `hash` so a tight `to_string(big_str)` loop also tops out.
    #[test]
    fn audit_m15_tostring_megabyte_input_runs_out_of_gas_in_loop() {
        let big: String = "x".repeat(1_048_576);
        let mut ops: Vec<Op> = Vec::new();
        for _ in 0..10_000 {
            ops.push(Op::Push(Value::Str(big.clone())));
            ops.push(Op::Call("to_string".into(), 1));
            ops.push(Op::Pop);
        }
        ops.push(Op::Push(Value::U64(0)));
        ops.push(Op::Return);
        let bytecode = make_bytecode("spam", ops);
        let result =
            EvaporVM::execute(&bytecode, "spam", vec![], empty_state(), &test_ctx());
        assert!(result.is_err(), "expected gas exhaustion");
    }

    /// The size-estimator must walk recursive Map / Array values.
    /// A nested Map with 32 KiB of nested data should price out
    /// substantially higher than a 4-byte Map.
    #[test]
    fn audit_m15_tostring_estimator_walks_recursive_value() {
        let big_str = "z".repeat(32 * 1024);
        let small = Value::Map({
            let mut m = HashMap::new();
            m.insert("k".to_string(), Value::U64(1));
            m
        });
        let big = Value::Map({
            let mut m = HashMap::new();
            m.insert("k".to_string(), Value::Str(big_str));
            m
        });
        let small_est = estimated_to_string_bytes(&small);
        let big_est = estimated_to_string_bytes(&big);
        assert!(
            big_est > small_est * 100,
            "estimator should scale with nested string size (small={small_est}, big={big_est})"
        );
    }

    // SCR-N6: RandomRange must never return a value >= max, and must reject
    // max=0. Tests across non-power-of-two sizes to confirm rejection sampling
    // eliminates modulo bias (naive `raw % max` would over-weight low residues).
    #[test]
    fn scr_n6_random_range_output_always_in_bounds() {
        let non_pow2_sizes: &[u64] = &[3, 5, 6, 7, 10, 13, 100, 1000, u64::MAX];
        for &max in non_pow2_sizes {
            // Sweep 256 different VRF seeds; verify every output < max.
            for seed in 0u8..=255 {
                let mut ctx = test_ctx();
                ctx.vrf_randomness = [seed; 32];
                let ops = vec![
                    Op::Push(Value::U64(max)),
                    Op::RandomRange,
                    Op::Return,
                ];
                let bytecode = make_bytecode("run", ops);
                let r = EvaporVM::execute(&bytecode, "run", vec![], empty_state(), &ctx)
                    .expect("RandomRange should succeed");
                let v = match r.return_value {
                    Value::U64(v) => v,
                    other => panic!("expected U64, got {other:?}"),
                };
                assert!(v < max, "SCR-N6: RandomRange({max}) returned {v} >= {max} for seed={seed}");
            }
        }
    }

    #[test]
    fn scr_n6_random_range_max_one_always_zero() {
        for seed in [0u8, 1, 42, 255] {
            let mut ctx = test_ctx();
            ctx.vrf_randomness = [seed; 32];
            let ops = vec![Op::Push(Value::U64(1)), Op::RandomRange, Op::Return];
            let bytecode = make_bytecode("run", ops);
            let r = EvaporVM::execute(&bytecode, "run", vec![], empty_state(), &ctx).unwrap();
            assert_eq!(r.return_value, Value::U64(0), "RandomRange(1) must always be 0");
        }
    }

    #[test]
    fn scr_n6_random_range_zero_max_rejected() {
        let ops = vec![Op::Push(Value::U64(0)), Op::RandomRange, Op::Return];
        let bytecode = make_bytecode("run", ops);
        let err = EvaporVM::execute(&bytecode, "run", vec![], empty_state(), &test_ctx());
        assert!(err.is_err(), "SCR-N6: RandomRange(0) must be a runtime error");
    }

    // ── OPCODE-5 (audit 2026-05-17): Op::Emit bulk-memory guard ──

    /// Pre-fix: 64 emits of 1 MiB strings each charged only GAS_EMIT (8 gas)
    /// but enqueued ~64 MiB of heap. Now each emit tracks msg.len() against
    /// MAX_MEMORY_BYTES (4 MiB), so the 5th 1-MiB emit must hit the limit.
    #[test]
    fn opcode5_bulk_emit_hits_memory_limit() {
        let big: String = "e".repeat(1_048_576); // 1 MiB
        let mut ops: Vec<Op> = Vec::new();
        for _ in 0..10 {
            ops.push(Op::Push(Value::Str(big.clone())));
            ops.push(Op::Emit);
        }
        ops.push(Op::Push(Value::U64(0)));
        ops.push(Op::Return);
        let bytecode = make_bytecode("blast", ops);
        let result = EvaporVM::execute(&bytecode, "blast", vec![], empty_state(), &test_ctx());
        assert!(result.is_err(), "bulk emit should hit memory limit");
        let err = format!("{:?}", result.unwrap_err());
        assert!(
            err.contains("memory"),
            "expected memory error, got: {err}"
        );
    }

    /// Small emits are unaffected: a handful of short messages should succeed.
    #[test]
    fn opcode5_small_emits_still_work() {
        let ops = vec![
            Op::Push(Value::Str("hello".into())),
            Op::Emit,
            Op::Push(Value::Str("world".into())),
            Op::Emit,
            Op::Push(Value::U64(0)),
            Op::Return,
        ];
        let bytecode = make_bytecode("ok", ops);
        let r = EvaporVM::execute(&bytecode, "ok", vec![], empty_state(), &test_ctx())
            .expect("small emits should succeed");
        assert_eq!(r.events.len(), 2);
    }
}
