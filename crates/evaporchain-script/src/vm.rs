use crate::compiler::{EvaporBytecode, Op};
use crate::{ExecutionContext, ScriptCallResult, ScriptError, Value};
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
const GAS_RETURN: u64 = 1;

// ─── VM ─────────────────────────────────────────────────────────────────────

/// Stack-based virtual machine for EvaporScript bytecode.
pub struct EvaporVM {
    stack: Vec<Value>,
    locals: HashMap<String, Value>,
    state: HashMap<String, Value>,
    events: Vec<String>,
    gas_used: u64,
    gas_limit: u64,
}

impl EvaporVM {
    fn new(state: HashMap<String, Value>, gas_limit: u64) -> Self {
        Self {
            stack: Vec::new(),
            locals: HashMap::new(),
            state,
            events: Vec::new(),
            gas_used: 0,
            gas_limit,
        }
    }

    fn charge_gas(&mut self, cost: u64) -> Result<(), ScriptError> {
        self.gas_used += cost;
        if self.gas_limit > 0 && self.gas_used > self.gas_limit {
            return Err(ScriptError::GasLimitExceeded {
                used: self.gas_used,
                limit: self.gas_limit,
            });
        }
        Ok(())
    }

    fn push(&mut self, val: Value) {
        self.stack.push(val);
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
    ) -> Result<Value, ScriptError> {
        let start_offset = bytecode
            .methods
            .get(method)
            .ok_or_else(|| ScriptError::Runtime(format!("method '{method}' not found")))?;

        // Push args onto stack (left-to-right, so first arg is deepest)
        for arg in args {
            self.push(arg);
        }

        let mut ip = *start_offset;

        loop {
            if ip >= bytecode.opcodes.len() {
                return Ok(Value::Null);
            }

            let op = &bytecode.opcodes[ip];

            match op {
                Op::Push(val) => {
                    self.charge_gas(GAS_PUSH)?;
                    self.push(val.clone());
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
                    self.push(val);
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
                    self.push(val);
                }

                Op::StateStore(field) => {
                    self.charge_gas(GAS_STATE_STORE)?;
                    let val = self.pop()?;
                    self.state.insert(field.clone(), val);
                }

                Op::Add => {
                    self.charge_gas(GAS_ADD)?;
                    let b = self.pop()?;
                    let a = self.pop()?;
                    let result = match (&a, &b) {
                        (Value::U64(x), Value::U64(y)) => Value::U64(x.wrapping_add(*y)),
                        (Value::Str(x), Value::Str(y)) => Value::Str(format!("{x}{y}")),
                        _ => {
                            return Err(ScriptError::Runtime(format!(
                                "cannot add {a:?} and {b:?}"
                            )))
                        }
                    };
                    self.push(result);
                }

                Op::Sub => {
                    self.charge_gas(GAS_SUB)?;
                    let b = self.pop()?.as_u64()?;
                    let a = self.pop()?.as_u64()?;
                    self.push(Value::U64(a.wrapping_sub(b)));
                }

                Op::Mul => {
                    self.charge_gas(GAS_MUL)?;
                    let b = self.pop()?.as_u64()?;
                    let a = self.pop()?.as_u64()?;
                    self.push(Value::U64(a.wrapping_mul(b)));
                }

                Op::Div => {
                    self.charge_gas(GAS_DIV)?;
                    let b = self.pop()?.as_u64()?;
                    let a = self.pop()?.as_u64()?;
                    if b == 0 {
                        return Err(ScriptError::Runtime("division by zero".into()));
                    }
                    self.push(Value::U64(a / b));
                }

                Op::Eq => {
                    self.charge_gas(GAS_CMP)?;
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(Value::Bool(a == b));
                }

                Op::Neq => {
                    self.charge_gas(GAS_CMP)?;
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(Value::Bool(a != b));
                }

                Op::Gt => {
                    self.charge_gas(GAS_CMP)?;
                    let b = self.pop()?.as_u64()?;
                    let a = self.pop()?.as_u64()?;
                    self.push(Value::Bool(a > b));
                }

                Op::Lt => {
                    self.charge_gas(GAS_CMP)?;
                    let b = self.pop()?.as_u64()?;
                    let a = self.pop()?.as_u64()?;
                    self.push(Value::Bool(a < b));
                }

                Op::Gte => {
                    self.charge_gas(GAS_CMP)?;
                    let b = self.pop()?.as_u64()?;
                    let a = self.pop()?.as_u64()?;
                    self.push(Value::Bool(a >= b));
                }

                Op::Lte => {
                    self.charge_gas(GAS_CMP)?;
                    let b = self.pop()?.as_u64()?;
                    let a = self.pop()?.as_u64()?;
                    self.push(Value::Bool(a <= b));
                }

                Op::And => {
                    self.charge_gas(GAS_LOGIC)?;
                    let b = self.pop()?.as_bool()?;
                    let a = self.pop()?.as_bool()?;
                    self.push(Value::Bool(a && b));
                }

                Op::Or => {
                    self.charge_gas(GAS_LOGIC)?;
                    let b = self.pop()?.as_bool()?;
                    let a = self.pop()?.as_bool()?;
                    self.push(Value::Bool(a || b));
                }

                Op::Not => {
                    self.charge_gas(GAS_LOGIC)?;
                    let a = self.pop()?.as_bool()?;
                    self.push(Value::Bool(!a));
                }

                Op::Neg => {
                    self.charge_gas(GAS_SUB)?;
                    let a = self.pop()?.as_u64()?;
                    // For u64, negation wraps
                    self.push(Value::U64(0u64.wrapping_sub(a)));
                }

                Op::Jump(target) => {
                    self.charge_gas(GAS_JUMP)?;
                    ip = *target;
                    continue;
                }

                Op::JumpIf(target) => {
                    self.charge_gas(GAS_JUMP)?;
                    let cond = self.pop()?.as_bool()?;
                    if cond {
                        ip = *target;
                        continue;
                    }
                }

                Op::JumpIfFalse(target) => {
                    self.charge_gas(GAS_JUMP)?;
                    let cond = self.pop()?.as_bool()?;
                    if !cond {
                        ip = *target;
                        continue;
                    }
                }

                Op::Call(name, arg_count) => {
                    self.charge_gas(GAS_CALL)?;
                    let result = self.call_builtin(name, *arg_count, ctx)?;
                    self.push(result);
                }

                Op::MapGet(field) => {
                    self.charge_gas(GAS_MAP_GET)?;
                    let key = self.pop()?;
                    let key_str = key.to_map_key();
                    let val = match self.state.get(field) {
                        Some(Value::Map(map)) => {
                            map.get(&key_str).cloned().unwrap_or(Value::U64(0))
                        }
                        _ => Value::U64(0),
                    };
                    self.push(val);
                }

                Op::MapSet(field) => {
                    self.charge_gas(GAS_MAP_SET)?;
                    let val = self.pop()?;
                    let key = self.pop()?;
                    let key_str = key.to_map_key();

                    let map = self
                        .state
                        .entry(field.clone())
                        .or_insert_with(|| Value::Map(HashMap::new()));
                    match map {
                        Value::Map(m) => {
                            m.insert(key_str, val);
                        }
                        _ => {
                            return Err(ScriptError::Runtime(format!(
                                "state field '{field}' is not a map"
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
                    self.events.push(msg);
                }

                Op::Return => {
                    self.charge_gas(GAS_RETURN)?;
                    return self.pop().or(Ok(Value::Null));
                }

                Op::Halt => {
                    return Ok(Value::Null);
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

            other => Err(ScriptError::Runtime(format!(
                "unknown built-in function: {other}"
            ))),
        }
    }

    /// Execute a method on compiled bytecode.
    pub fn execute(
        bytecode: &EvaporBytecode,
        method: &str,
        args: Vec<Value>,
        state: HashMap<String, Value>,
        ctx: &ExecutionContext,
    ) -> Result<ScriptCallResult, ScriptError> {
        Self::execute_with_gas_limit(bytecode, method, args, state, ctx, 0)
    }

    /// Execute with an explicit gas limit (0 = unlimited).
    pub fn execute_with_gas_limit(
        bytecode: &EvaporBytecode,
        method: &str,
        args: Vec<Value>,
        state: HashMap<String, Value>,
        ctx: &ExecutionContext,
        gas_limit: u64,
    ) -> Result<ScriptCallResult, ScriptError> {
        let mut vm = Self::new(state, gas_limit);
        let return_value = vm.execute_method(bytecode, method, args, ctx)?;

        Ok(ScriptCallResult {
            return_value,
            events: vm.events,
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
        }
    }

    fn compile_src(src: &str) -> EvaporBytecode {
        let ast = parser::parse(src).unwrap();
        compiler::compile(&ast).unwrap()
    }

    fn empty_state() -> HashMap<String, Value> {
        HashMap::new()
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
        };
        let ctx_user = ExecutionContext {
            caller: user,
            owner,
            epoch: 100,
            energy: 5000,
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
}
