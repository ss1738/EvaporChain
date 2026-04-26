use crate::compiler::{EvaporBytecode, Op};
use crate::{ContractEvent, ExecutionContext, ExternalCaller, ScriptCallResult, ScriptError, Value, MAX_CALL_DEPTH};
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
        }
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
        self.gas_used += cost;
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
            self.step_count += 1;
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
                    let val = self
                        .locals
                        .get(name)
                        .cloned()
                        .unwrap_or(Value::Null);
                    self.push(val)?;
                }

                Op::Store(name) => {
                    self.charge_gas(GAS_STORE)?;
                    let val = self.pop()?;
                    self.locals.insert(name.clone(), val);
                }

                Op::StateLoad(field) => {
                    self.charge_gas(GAS_STATE_LOAD)?;
                    let val = self
                        .state
                        .get(field)
                        .cloned()
                        .unwrap_or(Value::Null);
                    self.push(val)?;
                }

                Op::StateStore(field) => {
                    self.charge_gas(GAS_STATE_STORE)?;
                    let val = self.pop()?;
                    if !self.state.contains_key(&*field)
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
                        (Value::U64(x), Value::U64(y)) => Value::U64(
                            x.checked_add(*y).ok_or_else(|| {
                                ScriptError::Runtime("arithmetic overflow: addition".into())
                            })?,
                        ),
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
                            return Err(ScriptError::Runtime(format!(
                                "cannot add {a:?} and {b:?}"
                            )))
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
                            target, bytecode.opcodes.len()
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
                            target, bytecode.opcodes.len()
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
                            target, bytecode.opcodes.len()
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

                    // Check if this is a new map entry — track memory before borrowing state
                    let is_new_entry = match self.state.get(&*field) {
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
                    self.events.push(msg.clone());
                    self.structured_events.push(ContractEvent {
                        name: "Log".into(),
                        topics: vec![],
                        data: vec![Value::Str(msg)],
                    });
                }

                Op::EmitEvent { name, topic_count } => {
                    self.charge_gas(GAS_EMIT_EVENT)?;
                    let tc = *topic_count;
                    let total = tc + 1; // topics + 1 data value
                    let mut values = Vec::with_capacity(total);
                    for _ in 0..total {
                        values.push(self.pop()?);
                    }
                    values.reverse();
                    let topics = values[..tc].to_vec();
                    let data = values[tc..].to_vec();
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
                    let decayed =
                        evaporchain_types::energy_at_epoch(initial_energy, half_life, epochs_elapsed);
                    self.push(Value::U64(decayed))?;
                }

                // ── VRF / Randomness Opcodes ──

                Op::VrfRandomness => {
                    self.charge_gas(GAS_PUSH)?;
                    let value = u64::from_le_bytes(
                        ctx.vrf_randomness[..8].try_into().unwrap(),
                    );
                    self.push(Value::U64(value))?;
                }

                Op::VrfDomainRandomness => {
                    self.charge_gas(GAS_STATE_LOAD)?; // Costs a bit more (hashing)
                    let domain = self.pop()?.as_str()?.to_string();
                    let mut hasher = blake3::Hasher::new();
                    hasher.update(b"EvaporChain_Beacon_Derive");
                    hasher.update(&ctx.vrf_randomness);
                    hasher.update(domain.as_bytes());
                    let derived = hasher.finalize();
                    let value = u64::from_le_bytes(
                        derived.as_bytes()[..8].try_into().unwrap(),
                    );
                    self.push(Value::U64(value))?;
                }

                Op::RandomRange => {
                    self.charge_gas(GAS_STATE_LOAD)?;
                    let max = self.pop()?.as_u64()?;
                    if max == 0 {
                        return Err(ScriptError::Runtime(
                            "random_range: max must be > 0".into(),
                        ));
                    }
                    let raw = u64::from_le_bytes(
                        ctx.vrf_randomness[..8].try_into().unwrap(),
                    );
                    self.push(Value::U64(raw % max))?;
                }

                Op::CallExternal { arg_count } => {
                    self.charge_gas(GAS_CALL_EXTERNAL)?;
                    if ctx.call_depth >= MAX_CALL_DEPTH {
                        return Err(ScriptError::Runtime(format!(
                            "cross-contract call depth exceeded (max {})", MAX_CALL_DEPTH
                        )));
                    }
                    let ac = *arg_count;
                    // Stack: [contract_id, method, arg0, arg1, ...] (bottom to top)
                    // arg_count includes contract_id and method, so actual args = ac - 2
                    let extra_args = if ac >= 2 { ac - 2 } else { 0 };
                    let mut args = Vec::with_capacity(extra_args);
                    for _ in 0..extra_args {
                        args.push(self.pop()?);
                    }
                    args.reverse();
                    let method = self.pop()?.as_str()?.to_string();
                    let contract_id = self.pop()?.as_u64()?;

                    let gas_remaining = self.gas_limit.saturating_sub(self.gas_used);

                    if let Some(ext) = external.as_mut() {
                        let (return_val, events, gas_used) = ext.call_external(
                            contract_id,
                            &method,
                            args,
                            ctx.caller,
                            ctx.epoch,
                            ctx.call_depth + 1,
                            gas_remaining,
                        )?;
                        self.gas_used += gas_used;
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
                    return Err(ScriptError::Runtime(
                        "balance() takes 1 argument".into(),
                    ));
                }
                let _addr = self.pop()?.as_address()?;
                // In a real implementation, this would look up the on-chain balance.
                // For the script VM, we return 0 as a placeholder.
                Ok(Value::U64(0))
            }

            "transfer" => {
                if arg_count != 2 {
                    return Err(ScriptError::Runtime(
                        "transfer() takes 2 arguments".into(),
                    ));
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
                    return Err(ScriptError::Runtime(
                        "emit() takes 1 argument".into(),
                    ));
                }
                let val = self.pop()?;
                let msg = match val {
                    Value::Str(s) => s,
                    other => format!("{other}"),
                };
                self.events.push(msg);
                Ok(Value::Null)
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
                    return Err(ScriptError::Runtime(
                        "require() takes 2 arguments".into(),
                    ));
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
                    return Err(ScriptError::Runtime(
                        "to_string() takes 1 argument".into(),
                    ));
                }
                let val = self.pop()?;
                let s = match val {
                    Value::Str(s) => s,
                    other => format!("{other}"),
                };
                Ok(Value::Str(s))
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
    pub fn execute_full(
        bytecode: &EvaporBytecode,
        method: &str,
        args: Vec<Value>,
        state: HashMap<String, Value>,
        ctx: &ExecutionContext,
        gas_limit: u64,
        mut external: Option<&mut dyn ExternalCaller>,
    ) -> Result<ScriptCallResult, ScriptError> {
        let mut vm = Self::new(state, gas_limit);
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
        let result = EvaporVM::execute(
            &bytecode,
            "check",
            vec![Value::U64(0)],
            empty_state(),
            &ctx,
        );

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
        let result =
            EvaporVM::execute(&bytecode, "fire", vec![], empty_state(), &ctx).unwrap();

        assert_eq!(result.events.len(), 2);
        assert_eq!(result.events[0], "hello world");
        assert_eq!(result.events[1], "second event");
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
        let r1 = EvaporVM::execute(
            &bytecode,
            "increment",
            vec![Value::U64(5)],
            state,
            &ctx,
        )
        .unwrap();

        // Get — use state from previous execution
        let r2 =
            EvaporVM::execute(&bytecode, "get", vec![], r1.state_changes, &ctx).unwrap();
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
        let r1 =
            EvaporVM::execute(&bytecode, "is_owner", vec![], empty_state(), &ctx1).unwrap();
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
        let r2 =
            EvaporVM::execute(&bytecode, "is_owner", vec![], empty_state(), &ctx2).unwrap();
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
        assert_eq!(
            r1.state_changes.get("total_issued"),
            Some(&Value::U64(100))
        );
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
        let result = EvaporVM::execute(
            &bytecode,
            "on_evaporate",
            vec![],
            empty_state(),
            &ctx,
        )
        .unwrap();

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
        let result =
            EvaporVM::execute(&bytecode, "on_grace", vec![], empty_state(), &ctx)
                .unwrap();

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
        let r2 =
            EvaporVM::execute(&bytecode2, "run", vec![], empty_state(), &test_ctx()).unwrap();
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
        let mut ops: Vec<Op> = (0..1025)
            .map(|_| Op::Push(Value::U64(1)))
            .collect();
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

        let result = EvaporVM::execute(
            &bytecode,
            "sum_to",
            vec![Value::U64(10)],
            state,
            &ctx,
        )
        .unwrap();
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
        let result = EvaporVM::execute_with_gas_limit(
            &bytecode,
            "loop_forever",
            vec![],
            state,
            &ctx,
            500,
        );
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
        assert_eq!(
            r1.state_changes.get("balance"),
            Some(&Value::U64(50))
        );

        // Withdraw 200 from 100 — should fail with underflow
        let r2 = EvaporVM::execute(
            &bytecode,
            "withdraw",
            vec![Value::U64(200)],
            state,
            &ctx,
        );
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
        let result =
            EvaporVM::execute(&bytecode, "modify", vec![], empty_state(), &ctx).unwrap();
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
        let result = EvaporVM::execute(
            &bytecode, "add_to_element", vec![], empty_state(), &ctx,
        ).unwrap();
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
        let result =
            EvaporVM::execute(&bytecode, "count", vec![], empty_state(), &ctx).unwrap();
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
        state.insert("items".into(), Value::Array(vec![
            Value::U64(100), Value::U64(200), Value::U64(300),
        ]));

        let r1 = EvaporVM::execute(
            &bytecode, "get_item", vec![Value::U64(1)], state.clone(), &ctx,
        ).unwrap();
        assert_eq!(r1.return_value, Value::U64(200));

        let r2 = EvaporVM::execute(
            &bytecode, "set_item", vec![Value::U64(0), Value::U64(999)], state, &ctx,
        ).unwrap();
        let updated_items = r2.state_changes.get("items").unwrap();
        match updated_items {
            Value::Array(arr) => assert_eq!(arr[0], Value::U64(999)),
            other => panic!("expected array, got {other:?}"),
        }
    }
}
