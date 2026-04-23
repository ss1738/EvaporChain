use crate::parser::{
    AssignTarget, BinOp, Contract, Expr, LifecycleHook, Stmt, UnaryOp,
};
use crate::{ScriptError, StateFieldSchema, StateSchema, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── Opcodes ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Op {
    /// Push a constant value onto the stack.
    Push(Value),
    /// Pop top of stack.
    Pop,
    /// Load a local variable onto the stack.
    Load(String),
    /// Pop stack into a local variable.
    Store(String),
    /// Load a state field onto the stack.
    StateLoad(String),
    /// Pop stack and store into a state field.
    StateStore(String),

    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,

    // Comparison
    Eq,
    Neq,
    Gt,
    Lt,
    Gte,
    Lte,

    // Logic
    And,
    Or,
    Not,
    Neg,

    /// Unconditional jump to absolute offset.
    Jump(usize),
    /// Pop bool; jump if true.
    JumpIf(usize),
    /// Pop bool; jump if false.
    JumpIfFalse(usize),

    /// Call a built-in function by name. Args are on stack, count is arg count.
    Call(String, usize),

    /// Load from map: pop key, then load map[key] from state field.
    MapGet(String),
    /// Store into map: pop value, pop key, then store map[key]=value in state field.
    MapSet(String),

    /// Pop bool + message string. Revert if false.
    Require,
    /// Pop string, emit event.
    Emit,
    /// Emit a structured event: pop `topic_count` topics + 1 data value.
    EmitEvent { name: String, topic_count: usize },
    /// Return top of stack.
    Return,
    /// Halt execution.
    Halt,

    // ── Temporal Opcodes ──

    /// Push the current epoch onto the stack.
    EpochNow,
    /// Push the current block number onto the stack.
    BlockNum,
    /// Pop object_id (string), push its current energy onto the stack.
    EnergyOf,
    /// Pop max_epoch, pop min_epoch. Require current epoch is in [min, max).
    RequireEpochRange,
    /// Pop half_life, pop initial_energy, pop epochs_elapsed. Push decayed energy.
    ComputeDecay,

    // ── VRF / Randomness Opcodes ──

    /// Push the current block's VRF randomness beacon value (truncated to u64).
    VrfRandomness,
    /// Pop a domain string, push domain-separated randomness from the beacon.
    VrfDomainRandomness,
    /// Pop max (exclusive), push random u64 in [0, max) derived from beacon.
    RandomRange,

    // ── Cross-Contract Call ──

    /// Pop contract_id (u64), method name (string), then `arg_count` args.
    /// Calls the target contract and pushes the return value.
    CallExternal { arg_count: usize },
}

// ─── Bytecode ───────────────────────────────────────────────────────────────

/// Compiled EvaporScript bytecode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaporBytecode {
    /// Method name → starting opcode index.
    pub methods: HashMap<String, usize>,
    /// Flat opcode list.
    pub opcodes: Vec<Op>,
    /// State schema describing contract fields.
    pub state_schema: StateSchema,
    /// Contract name.
    pub name: String,
}

// ─── Compiler ───────────────────────────────────────────────────────────────

struct Compiler {
    opcodes: Vec<Op>,
    methods: HashMap<String, usize>,
}

impl Compiler {
    fn new() -> Self {
        Self {
            opcodes: Vec::new(),
            methods: HashMap::new(),
        }
    }

    fn emit(&mut self, op: Op) -> usize {
        let idx = self.opcodes.len();
        self.opcodes.push(op);
        idx
    }

    fn current_offset(&self) -> usize {
        self.opcodes.len()
    }

    /// Patch a jump instruction to point to the given target.
    fn patch_jump(&mut self, jump_idx: usize, target: usize) {
        match &mut self.opcodes[jump_idx] {
            Op::Jump(ref mut off) => *off = target,
            Op::JumpIf(ref mut off) => *off = target,
            Op::JumpIfFalse(ref mut off) => *off = target,
            _ => panic!("tried to patch non-jump instruction"),
        }
    }

    fn compile_contract(&mut self, contract: &Contract) -> Result<(), ScriptError> {
        // Compile each function
        for func in &contract.functions {
            let offset = self.current_offset();
            self.methods.insert(func.name.clone(), offset);

            // Store parameters as local variables (args pushed onto stack by VM)
            // Args are pushed left-to-right, so we store them in reverse
            for (name, _) in func.params.iter().rev() {
                self.emit(Op::Store(name.clone()));
            }

            // Compile body
            for stmt in &func.body {
                self.compile_stmt(stmt)?;
            }

            // Implicit return null if no explicit return
            self.emit(Op::Push(Value::Null));
            self.emit(Op::Return);
        }

        // Compile lifecycle hooks
        for hook in &contract.lifecycle_hooks {
            let (name, body) = match hook {
                LifecycleHook::OnEvaporate(body) => ("on_evaporate", body),
                LifecycleHook::OnGrace(body) => ("on_grace", body),
                LifecycleHook::OnRefresh(body) => ("on_refresh", body),
            };

            let offset = self.current_offset();
            self.methods.insert(name.to_string(), offset);

            for stmt in body {
                self.compile_stmt(stmt)?;
            }

            self.emit(Op::Push(Value::Null));
            self.emit(Op::Return);
        }

        Ok(())
    }

    fn compile_stmt(&mut self, stmt: &Stmt) -> Result<(), ScriptError> {
        match stmt {
            Stmt::Let { name, value } => {
                self.compile_expr(value)?;
                self.emit(Op::Store(name.clone()));
            }

            Stmt::Assign { target, value } => {
                match target {
                    AssignTarget::Variable(name) => {
                        self.compile_expr(value)?;
                        self.emit(Op::Store(name.clone()));
                    }
                    AssignTarget::StateField(field) => {
                        self.compile_expr(value)?;
                        self.emit(Op::StateStore(field.clone()));
                    }
                    AssignTarget::MapEntry(field, key) => {
                        // Stack order for MapSet: value, key (key popped first, then value)
                        self.compile_expr(key)?;
                        self.compile_expr(value)?;
                        self.emit(Op::MapSet(field.clone()));
                    }
                }
            }

            Stmt::CompoundAssign { target, op, value } => {
                // Load current value, compute new value, store back
                match target {
                    AssignTarget::StateField(field) => {
                        self.emit(Op::StateLoad(field.clone()));
                        self.compile_expr(value)?;
                        self.emit_binop(*op);
                        self.emit(Op::StateStore(field.clone()));
                    }
                    AssignTarget::MapEntry(field, key) => {
                        // Load current: push key, MapGet
                        self.compile_expr(key)?;
                        self.emit(Op::MapGet(field.clone()));
                        // Compute new value
                        self.compile_expr(value)?;
                        self.emit_binop(*op);
                        // Store back: push key, push value (value is on stack), MapSet
                        // We need key again — compile it again
                        // Stack has: new_value
                        // We need: key, new_value for MapSet
                        self.compile_expr(key)?;
                        // Swap: we have new_value, key on stack but need key, new_value
                        // Actually MapSet pops: value first, then key
                        // So stack should be: key (bottom), value (top) — which means
                        // we push key first, but value is already there...
                        // Let's use Store/Load to shuffle
                        self.emit(Op::Store("__map_key_tmp".into()));
                        self.emit(Op::Store("__map_val_tmp".into()));
                        self.emit(Op::Load("__map_key_tmp".into()));
                        self.emit(Op::Load("__map_val_tmp".into()));
                        self.emit(Op::MapSet(field.clone()));
                    }
                    AssignTarget::Variable(name) => {
                        self.emit(Op::Load(name.clone()));
                        self.compile_expr(value)?;
                        self.emit_binop(*op);
                        self.emit(Op::Store(name.clone()));
                    }
                }
            }

            Stmt::If {
                condition,
                then_body,
                else_body,
            } => {
                self.compile_expr(condition)?;
                // Jump to else if false
                let else_jump = self.emit(Op::JumpIfFalse(0));

                for s in then_body {
                    self.compile_stmt(s)?;
                }

                if let Some(else_stmts) = else_body {
                    // Jump over else block
                    let end_jump = self.emit(Op::Jump(0));
                    let else_start = self.current_offset();
                    self.patch_jump(else_jump, else_start);

                    for s in else_stmts {
                        self.compile_stmt(s)?;
                    }

                    let end = self.current_offset();
                    self.patch_jump(end_jump, end);
                } else {
                    let end = self.current_offset();
                    self.patch_jump(else_jump, end);
                }
            }

            Stmt::While { condition, body } => {
                // while condition { body }
                //
                // Compiled to:
                //   loop_start:
                //     <condition>
                //     JumpIfFalse loop_end
                //     <body>
                //     Jump loop_start
                //   loop_end:
                let loop_start = self.current_offset();
                self.compile_expr(condition)?;
                let exit_jump = self.emit(Op::JumpIfFalse(0));
                for s in body {
                    self.compile_stmt(s)?;
                }
                self.emit(Op::Jump(loop_start));
                let loop_end = self.current_offset();
                self.patch_jump(exit_jump, loop_end);
            }

            Stmt::Return(expr) => {
                if let Some(e) = expr {
                    self.compile_expr(e)?;
                } else {
                    self.emit(Op::Push(Value::Null));
                }
                self.emit(Op::Return);
            }

            Stmt::Require { condition, message } => {
                self.compile_expr(message)?;
                self.compile_expr(condition)?;
                self.emit(Op::Require);
            }

            Stmt::Emit(expr) => {
                self.compile_expr(expr)?;
                self.emit(Op::Emit);
            }

            Stmt::ExprStmt(expr) => {
                self.compile_expr(expr)?;
                self.emit(Op::Pop);
            }
        }

        Ok(())
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<(), ScriptError> {
        match expr {
            Expr::Literal(val) => {
                self.emit(Op::Push(val.clone()));
            }

            Expr::Variable(name) => {
                // Built-in variables that are actually zero-arg function calls
                match name.as_str() {
                    "caller" | "owner" | "epoch" | "energy" => {
                        self.emit(Op::Call(name.clone(), 0));
                    }
                    _ => {
                        self.emit(Op::Load(name.clone()));
                    }
                }
            }

            Expr::StateAccess(field) => {
                self.emit(Op::StateLoad(field.clone()));
            }

            Expr::MapAccess(field, key) => {
                self.compile_expr(key)?;
                self.emit(Op::MapGet(field.clone()));
            }

            Expr::BinaryOp { left, op, right } => {
                self.compile_expr(left)?;
                self.compile_expr(right)?;
                self.emit_binop(*op);
            }

            Expr::UnaryOp { op, expr } => {
                self.compile_expr(expr)?;
                match op {
                    UnaryOp::Not => {
                        self.emit(Op::Not);
                    }
                    UnaryOp::Neg => {
                        self.emit(Op::Neg);
                    }
                }
            }

            Expr::FunctionCall { name, args } => {
                // Push args left-to-right
                for arg in args {
                    self.compile_expr(arg)?;
                }
                if name == "call_contract" {
                    self.emit(Op::CallExternal { arg_count: args.len() });
                } else {
                    self.emit(Op::Call(name.clone(), args.len()));
                }
            }
        }

        Ok(())
    }

    fn emit_binop(&mut self, op: BinOp) {
        match op {
            BinOp::Add => self.emit(Op::Add),
            BinOp::Sub => self.emit(Op::Sub),
            BinOp::Mul => self.emit(Op::Mul),
            BinOp::Div => self.emit(Op::Div),
            BinOp::Mod => self.emit(Op::Mod),
            BinOp::Eq => self.emit(Op::Eq),
            BinOp::Neq => self.emit(Op::Neq),
            BinOp::Gt => self.emit(Op::Gt),
            BinOp::Lt => self.emit(Op::Lt),
            BinOp::Gte => self.emit(Op::Gte),
            BinOp::Lte => self.emit(Op::Lte),
            BinOp::And => self.emit(Op::And),
            BinOp::Or => self.emit(Op::Or),
        };
    }
}

// ─── State Schema Extraction ────────────────────────────────────────────────

fn extract_state_schema(contract: &Contract) -> StateSchema {
    let fields = contract
        .state_fields
        .iter()
        .map(|f| {
            let default = f.default.as_ref().and_then(|expr| eval_const_expr(expr));
            StateFieldSchema {
                name: f.name.clone(),
                ty: f.ty.clone(),
                default,
            }
        })
        .collect();

    StateSchema { fields }
}

/// Evaluate a constant expression (for state field defaults).
fn eval_const_expr(expr: &Expr) -> Option<Value> {
    match expr {
        Expr::Literal(val) => Some(val.clone()),
        _ => None,
    }
}

// ─── Public API ─────────────────────────────────────────────────────────────

/// Compile a parsed Contract AST into EvaporBytecode.
pub fn compile(contract: &Contract) -> Result<EvaporBytecode, ScriptError> {
    let mut compiler = Compiler::new();
    compiler.compile_contract(contract)?;

    let state_schema = extract_state_schema(contract);

    Ok(EvaporBytecode {
        methods: compiler.methods,
        opcodes: compiler.opcodes,
        state_schema,
        name: contract.name.clone(),
    })
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;

    #[test]
    fn test_compile_simple_contract() {
        let src = r#"
contract Counter {
    state { count: u64 = 0 }
    fn get() -> u64 {
        return self.count
    }
}
"#;
        let ast = parser::parse(src).unwrap();
        let bytecode = compile(&ast).unwrap();

        assert_eq!(bytecode.name, "Counter");
        assert!(bytecode.methods.contains_key("get"));
        assert_eq!(bytecode.state_schema.fields.len(), 1);
        assert_eq!(bytecode.state_schema.fields[0].name, "count");
        assert!(!bytecode.opcodes.is_empty());
    }

    #[test]
    fn test_compile_method_table() {
        let src = r#"
contract Multi {
    state { x: u64 = 0 }
    fn a() { self.x = 1 }
    fn b() { self.x = 2 }
    fn c() { self.x = 3 }
}
"#;
        let ast = parser::parse(src).unwrap();
        let bytecode = compile(&ast).unwrap();

        assert_eq!(bytecode.methods.len(), 3);
        assert!(bytecode.methods.contains_key("a"));
        assert!(bytecode.methods.contains_key("b"));
        assert!(bytecode.methods.contains_key("c"));
        // Each method starts at a different offset
        let offsets: Vec<usize> = bytecode.methods.values().copied().collect();
        assert!(offsets[0] != offsets[1] || offsets[1] != offsets[2]);
    }

    #[test]
    fn test_compile_if_else_produces_jumps() {
        let src = r#"
contract Branch {
    state { x: u64 = 0 }
    fn test(val: u64) {
        if val > 10 {
            self.x = 1
        } else {
            self.x = 0
        }
    }
}
"#;
        let ast = parser::parse(src).unwrap();
        let bytecode = compile(&ast).unwrap();

        // Should contain JumpIfFalse and Jump opcodes
        let has_jump_if_false = bytecode
            .opcodes
            .iter()
            .any(|op| matches!(op, Op::JumpIfFalse(_)));
        let has_jump = bytecode
            .opcodes
            .iter()
            .any(|op| matches!(op, Op::Jump(_)));
        assert!(has_jump_if_false, "missing JumpIfFalse");
        assert!(has_jump, "missing Jump");
    }

    #[test]
    fn test_compile_and_verify_opcode_count() {
        let src = r#"
contract Simple {
    state { x: u64 = 0 }
    fn set(val: u64) {
        self.x = val
    }
}
"#;
        let ast = parser::parse(src).unwrap();
        let bytecode = compile(&ast).unwrap();

        // Expected: Store("val"), Load("val"), StateStore("x"), Push(Null), Return
        assert_eq!(bytecode.opcodes.len(), 5);
        assert_eq!(bytecode.opcodes[0], Op::Store("val".into()));
        assert_eq!(bytecode.opcodes[1], Op::Load("val".into()));
        assert_eq!(bytecode.opcodes[2], Op::StateStore("x".into()));
        assert_eq!(bytecode.opcodes[3], Op::Push(Value::Null));
        assert_eq!(bytecode.opcodes[4], Op::Return);
    }

    #[test]
    fn test_compile_lifecycle_hooks() {
        let src = r#"
contract WithHooks {
    state { x: u64 = 0 }
    on_evaporate() { emit("bye") }
    on_grace() { emit("grace") }
}
"#;
        let ast = parser::parse(src).unwrap();
        let bytecode = compile(&ast).unwrap();

        assert!(bytecode.methods.contains_key("on_evaporate"));
        assert!(bytecode.methods.contains_key("on_grace"));
    }

    #[test]
    fn test_compile_require_opcode() {
        let src = r#"
contract Auth {
    state { x: u64 = 0 }
    fn check() {
        require(caller == owner, "not allowed")
    }
}
"#;
        let ast = parser::parse(src).unwrap();
        let bytecode = compile(&ast).unwrap();

        let has_require = bytecode.opcodes.iter().any(|op| matches!(op, Op::Require));
        assert!(has_require, "missing Require opcode");
    }

    #[test]
    fn test_compile_state_schema_defaults() {
        let src = r#"
contract Defaults {
    state {
        name: string = "hello"
        count: u64 = 42
        flag: bool = true
        balances: map[address -> u64]
    }
}
"#;
        let ast = parser::parse(src).unwrap();
        let bytecode = compile(&ast).unwrap();

        let schema = &bytecode.state_schema;
        assert_eq!(schema.fields.len(), 4);
        assert_eq!(
            schema.fields[0].default,
            Some(Value::Str("hello".into()))
        );
        assert_eq!(schema.fields[1].default, Some(Value::U64(42)));
        assert_eq!(schema.fields[2].default, Some(Value::Bool(true)));
        assert_eq!(schema.fields[3].default, None); // map has no const default
    }

    #[test]
    fn test_compile_full_loyalty_points() {
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
        let ast = parser::parse(src).unwrap();
        let bytecode = compile(&ast).unwrap();

        assert_eq!(bytecode.name, "LoyaltyPoints");
        assert_eq!(bytecode.methods.len(), 4); // issue, spend, balance, on_evaporate
        assert!(bytecode.methods.contains_key("issue"));
        assert!(bytecode.methods.contains_key("spend"));
        assert!(bytecode.methods.contains_key("balance"));
        assert!(bytecode.methods.contains_key("on_evaporate"));
        assert!(bytecode.opcodes.len() > 20);
    }
}
