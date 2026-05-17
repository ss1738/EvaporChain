use crate::parser::{AssignTarget, BinOp, Contract, Expr, LifecycleHook, Stmt, UnaryOp};
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
    /// Pop N values, create Array, push onto stack.
    ArrayNew(usize),
    /// Pop index (u64), pop array, push array[index].
    ArrayGet,
    /// Pop index (u64), pop value, modify local variable's array at index.
    ArraySet(String),

    /// Pop string, emit event.
    Emit,
    /// Emit a structured event: pop `topic_count` topics + 1 data value.
    EmitEvent {
        name: String,
        topic_count: usize,
    },
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
    CallExternal {
        arg_count: usize,
    },
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
                    AssignTarget::ArrayElement(name, index) => {
                        self.compile_expr(value)?;
                        self.compile_expr(index)?;
                        self.emit(Op::ArraySet(name.clone()));
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
                        // Evaluate key once to avoid double side-effects
                        self.compile_expr(key)?;
                        self.emit(Op::Store("__map_key_tmp".into()));
                        // Load current: push cached key, MapGet
                        self.emit(Op::Load("__map_key_tmp".into()));
                        self.emit(Op::MapGet(field.clone()));
                        // Compute new value
                        self.compile_expr(value)?;
                        self.emit_binop(*op);
                        // Store back: need key, value on stack for MapSet
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
                    AssignTarget::ArrayElement(name, index) => {
                        self.compile_expr(index)?;
                        self.emit(Op::Store("__arr_idx_tmp".into()));
                        self.emit(Op::Load(name.clone()));
                        self.emit(Op::Load("__arr_idx_tmp".into()));
                        self.emit(Op::ArrayGet);
                        self.compile_expr(value)?;
                        self.emit_binop(*op);
                        self.emit(Op::Load("__arr_idx_tmp".into()));
                        self.emit(Op::ArraySet(name.clone()));
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
                    self.emit(Op::CallExternal {
                        arg_count: args.len(),
                    });
                } else {
                    self.emit(Op::Call(name.clone(), args.len()));
                }
            }

            Expr::ArrayLiteral(elements) => {
                for elem in elements {
                    self.compile_expr(elem)?;
                }
                self.emit(Op::ArrayNew(elements.len()));
            }

            Expr::ArrayAccess(array, index) => {
                self.compile_expr(array)?;
                self.compile_expr(index)?;
                self.emit(Op::ArrayGet);
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
            let default = f.default.as_ref().and_then(eval_const_expr);
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
        Expr::ArrayLiteral(elements) => {
            let vals: Option<Vec<Value>> = elements.iter().map(eval_const_expr).collect();
            vals.map(Value::Array)
        }
        _ => None,
    }
}

// ─── Optimization Passes ──────────────────────────────────────────────────

/// M-12: Constant folding — evaluate constant BinaryOp/UnaryOp at compile time.
/// Scans for patterns like `Push(a), Push(b), Add` and replaces with `Push(a+b)`.
fn constant_fold(opcodes: &mut Vec<Op>) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i < opcodes.len() {
        // Binary constant fold: Push(a), Push(b), BinOp → Push(result)
        if i + 2 < opcodes.len() {
            if let (Op::Push(ref a), Op::Push(ref b)) = (&opcodes[i], &opcodes[i + 1]) {
                let folded = match &opcodes[i + 2] {
                    Op::Add => fold_binop(a, b, BinOp::Add),
                    Op::Sub => fold_binop(a, b, BinOp::Sub),
                    Op::Mul => fold_binop(a, b, BinOp::Mul),
                    Op::Div => fold_binop(a, b, BinOp::Div),
                    Op::Mod => fold_binop(a, b, BinOp::Mod),
                    Op::Eq => fold_binop(a, b, BinOp::Eq),
                    Op::Neq => fold_binop(a, b, BinOp::Neq),
                    Op::Gt => fold_binop(a, b, BinOp::Gt),
                    Op::Lt => fold_binop(a, b, BinOp::Lt),
                    Op::Gte => fold_binop(a, b, BinOp::Gte),
                    Op::Lte => fold_binop(a, b, BinOp::Lte),
                    Op::And => fold_binop(a, b, BinOp::And),
                    Op::Or => fold_binop(a, b, BinOp::Or),
                    _ => None,
                };
                if let Some(result) = folded {
                    opcodes[i] = Op::Push(result);
                    opcodes.remove(i + 2);
                    opcodes.remove(i + 1);
                    changed = true;
                    continue;
                }
            }
        }
        // Unary constant fold: Push(a), Not → Push(!a)
        if i + 1 < opcodes.len() {
            if let Op::Push(ref a) = opcodes[i] {
                let folded = match &opcodes[i + 1] {
                    Op::Not => match a {
                        Value::Bool(b) => Some(Value::Bool(!b)),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(result) = folded {
                    opcodes[i] = Op::Push(result);
                    opcodes.remove(i + 1);
                    changed = true;
                    continue;
                }
            }
        }
        i += 1;
    }
    changed
}

/// SCR-N7 (audit 2026-05-15): hard cap on the size of a string
/// literal produced by `fold_binop`'s `+` arm. Matches the VM's
/// runtime `MAX_STRING_LITERAL` so compile-time and runtime
/// semantics agree.
pub const MAX_FOLD_STRING_LEN: usize = 65_536;

fn fold_binop(a: &Value, b: &Value, op: BinOp) -> Option<Value> {
    match (a, b, op) {
        (Value::U64(x), Value::U64(y), BinOp::Add) => Some(Value::U64(x.saturating_add(*y))),
        (Value::U64(x), Value::U64(y), BinOp::Sub) => Some(Value::U64(x.saturating_sub(*y))),
        (Value::U64(x), Value::U64(y), BinOp::Mul) => Some(Value::U64(x.saturating_mul(*y))),
        (Value::U64(x), Value::U64(y), BinOp::Div) if *y != 0 => Some(Value::U64(x / y)),
        (Value::U64(x), Value::U64(y), BinOp::Mod) if *y != 0 => Some(Value::U64(x % y)),
        (Value::U64(x), Value::U64(y), BinOp::Eq) => Some(Value::Bool(x == y)),
        (Value::U64(x), Value::U64(y), BinOp::Neq) => Some(Value::Bool(x != y)),
        (Value::U64(x), Value::U64(y), BinOp::Gt) => Some(Value::Bool(x > y)),
        (Value::U64(x), Value::U64(y), BinOp::Lt) => Some(Value::Bool(x < y)),
        (Value::U64(x), Value::U64(y), BinOp::Gte) => Some(Value::Bool(x >= y)),
        (Value::U64(x), Value::U64(y), BinOp::Lte) => Some(Value::Bool(x <= y)),
        (Value::Bool(x), Value::Bool(y), BinOp::And) => Some(Value::Bool(*x && *y)),
        (Value::Bool(x), Value::Bool(y), BinOp::Or) => Some(Value::Bool(*x || *y)),
        (Value::Bool(x), Value::Bool(y), BinOp::Eq) => Some(Value::Bool(x == y)),
        (Value::Bool(x), Value::Bool(y), BinOp::Neq) => Some(Value::Bool(x != y)),
        (Value::Str(x), Value::Str(y), BinOp::Add) => {
            // SCR-N7 (audit 2026-05-15): refuse to fold a string
            // concat when the result would exceed
            // `MAX_FOLD_STRING_LEN`. Two 64 KiB string literals
            // concatenated by `+` then folded into a single 128 KiB
            // Push (and chained further by repeat `+` folds) would
            // amplify compile-time RAM unboundedly. The runtime VM's
            // `MAX_STRING_LITERAL` (64 KiB) is the legitimate ceiling;
            // we cap the folded result there too, returning `None`
            // when oversized so the compiler emits an explicit Add
            // opcode (and the runtime's string-grow check fires).
            if x.len().saturating_add(y.len()) > MAX_FOLD_STRING_LEN {
                return None;
            }
            let mut s = x.clone();
            s.push_str(y);
            Some(Value::Str(s))
        }
        (Value::Str(x), Value::Str(y), BinOp::Eq) => Some(Value::Bool(x == y)),
        (Value::Str(x), Value::Str(y), BinOp::Neq) => Some(Value::Bool(x != y)),
        _ => None,
    }
}

/// M-11: Dead code elimination — remove unreachable opcodes after Return/Halt.
/// Uses reachability analysis: starting from each method entry point, follow
/// control flow; any opcode not reached is replaced with a no-op (Pop).
#[allow(clippy::ptr_arg)]
fn eliminate_dead_code(opcodes: &mut Vec<Op>, methods: &HashMap<String, usize>) -> bool {
    if opcodes.is_empty() {
        return false;
    }
    let len = opcodes.len();
    let mut reachable = vec![false; len];
    let mut worklist: Vec<usize> = methods.values().copied().collect();

    while let Some(pc) = worklist.pop() {
        if pc >= len || reachable[pc] {
            continue;
        }
        reachable[pc] = true;
        match &opcodes[pc] {
            Op::Return | Op::Halt => {
                // Terminal — don't enqueue successor
            }
            Op::Jump(target) => {
                worklist.push(*target);
            }
            Op::JumpIf(target) | Op::JumpIfFalse(target) => {
                worklist.push(*target);
                worklist.push(pc + 1);
            }
            _ => {
                worklist.push(pc + 1);
            }
        }
    }

    let mut changed = false;
    for i in 0..len {
        if !reachable[i] && opcodes[i] != Op::Pop {
            opcodes[i] = Op::Pop;
            changed = true;
        }
    }
    changed
}

/// Run all optimization passes until fixpoint.
fn optimize(opcodes: &mut Vec<Op>, methods: &HashMap<String, usize>) {
    for _ in 0..16 {
        let folded = constant_fold(opcodes);
        let eliminated = eliminate_dead_code(opcodes, methods);
        if !folded && !eliminated {
            break;
        }
    }
}

// ─── Public API ─────────────────────────────────────────────────────────────

/// Compile a parsed Contract AST into EvaporBytecode.
pub fn compile(contract: &Contract) -> Result<EvaporBytecode, ScriptError> {
    // NFT-1 (audit 2026-05-17): reject state fields that shadow builtin names.
    // Bare `owner`/`caller`/`epoch`/`energy` always resolve to the execution-
    // context builtin in EvaporScript; a state field with the same name would
    // be silently inaccessible without the `self.` prefix, misleading authors.
    const RESERVED: &[&str] = &["owner", "caller", "epoch", "energy"];
    for field in &contract.state_fields {
        if RESERVED.contains(&field.name.as_str()) {
            return Err(ScriptError::Compile(format!(
                "state field '{}' shadows a builtin reserved name; use a different name (e.g. 'holder' instead of 'owner')",
                field.name
            )));
        }
    }

    let mut compiler = Compiler::new();
    compiler.compile_contract(contract)?;

    let state_schema = extract_state_schema(contract);

    optimize(&mut compiler.opcodes, &compiler.methods);

    Ok(EvaporBytecode {
        methods: compiler.methods,
        opcodes: compiler.opcodes,
        state_schema,
        name: contract.name.clone(),
    })
}

/// Generate ABI from a parsed contract AST.
pub fn generate_abi(contract: &Contract) -> crate::ContractAbi {
    use crate::{AbiMethod, AbiParam, AbiStateField, ContractAbi};

    let methods = contract
        .functions
        .iter()
        .map(|f| {
            let mutates = fn_mutates_state(&f.body);
            AbiMethod {
                name: f.name.clone(),
                params: f
                    .params
                    .iter()
                    .map(|(name, ty)| AbiParam {
                        name: name.clone(),
                        ty: ty.clone(),
                    })
                    .collect(),
                return_type: f.return_type.clone(),
                mutates_state: mutates,
            }
        })
        .collect();

    let state = contract
        .state_fields
        .iter()
        .map(|f| AbiStateField {
            name: f.name.clone(),
            ty: f.ty.clone(),
            has_default: f.default.is_some(),
        })
        .collect();

    let lifecycle_hooks = contract
        .lifecycle_hooks
        .iter()
        .map(|h| match h {
            LifecycleHook::OnEvaporate(_) => "on_evaporate".to_string(),
            LifecycleHook::OnGrace(_) => "on_grace".to_string(),
            LifecycleHook::OnRefresh(_) => "on_refresh".to_string(),
        })
        .collect();

    ContractAbi {
        name: contract.name.clone(),
        methods,
        state,
        lifecycle_hooks,
    }
}

fn fn_mutates_state(stmts: &[Stmt]) -> bool {
    use crate::parser::AssignTarget;
    for stmt in stmts {
        match stmt {
            Stmt::Assign {
                target: AssignTarget::StateField(_),
                ..
            }
            | Stmt::CompoundAssign {
                target: AssignTarget::StateField(_),
                ..
            }
            | Stmt::Assign {
                target: AssignTarget::MapEntry(_, _),
                ..
            }
            | Stmt::CompoundAssign {
                target: AssignTarget::MapEntry(_, _),
                ..
            } => return true,
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                if fn_mutates_state(then_body) {
                    return true;
                }
                if let Some(eb) = else_body {
                    if fn_mutates_state(eb) {
                        return true;
                    }
                }
            }
            Stmt::While { body, .. } if fn_mutates_state(body) => return true,
            _ => {}
        }
    }
    false
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
        let has_jump = bytecode.opcodes.iter().any(|op| matches!(op, Op::Jump(_)));
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
        assert_eq!(schema.fields[0].default, Some(Value::Str("hello".into())));
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

    #[test]
    fn test_generate_abi() {
        let src = r#"
contract Token {
    state {
        name: string = "MyToken"
        balances: map[address -> u64]
        total: u64 = 0
    }

    fn mint(to: address, amount: u64) {
        require(caller == owner, "only owner")
        self.balances[to] += amount
        self.total += amount
    }

    fn balance_of(addr: address) -> u64 {
        return self.balances[addr]
    }

    on_evaporate() {
        emit("token evaporated")
    }
}
"#;
        let ast = parser::parse(src).unwrap();
        let abi = generate_abi(&ast);

        assert_eq!(abi.name, "Token");
        assert_eq!(abi.methods.len(), 2);

        let mint = &abi.methods[0];
        assert_eq!(mint.name, "mint");
        assert_eq!(mint.params.len(), 2);
        assert_eq!(mint.params[0].name, "to");
        assert_eq!(mint.params[0].ty, crate::ScriptType::Address);
        assert_eq!(mint.params[1].name, "amount");
        assert_eq!(mint.params[1].ty, crate::ScriptType::U64);
        assert!(mint.return_type.is_none());
        assert!(mint.mutates_state);

        let balance = &abi.methods[1];
        assert_eq!(balance.name, "balance_of");
        assert_eq!(balance.params.len(), 1);
        assert_eq!(balance.return_type, Some(crate::ScriptType::U64));
        assert!(!balance.mutates_state);

        assert_eq!(abi.state.len(), 3);
        assert_eq!(abi.state[0].name, "name");
        assert!(abi.state[0].has_default);
        assert_eq!(abi.state[1].name, "balances");
        assert!(!abi.state[1].has_default);

        assert_eq!(abi.lifecycle_hooks, vec!["on_evaporate"]);
    }

    #[test]
    fn test_abi_json_serialization() {
        let src = r#"
contract Simple {
    state { v: u64 = 0 }
    fn get() -> u64 { return self.v }
    fn set(x: u64) { self.v = x }
}
"#;
        let ast = parser::parse(src).unwrap();
        let abi = generate_abi(&ast);
        let json = serde_json::to_string_pretty(&abi).unwrap();
        assert!(json.contains("\"name\": \"Simple\""));
        assert!(json.contains("\"get\""));
        assert!(json.contains("\"set\""));
        assert!(json.contains("\"mutates_state\": true"));
        assert!(json.contains("\"mutates_state\": false"));
    }

    // ── M-12: Constant folding tests ──────────────────────────────────────

    #[test]
    fn test_constant_fold_arithmetic() {
        let mut ops = vec![Op::Push(Value::U64(10)), Op::Push(Value::U64(20)), Op::Add];
        let changed = constant_fold(&mut ops);
        assert!(changed);
        assert_eq!(ops, vec![Op::Push(Value::U64(30))]);
    }

    #[test]
    fn test_constant_fold_comparison() {
        let mut ops = vec![Op::Push(Value::U64(5)), Op::Push(Value::U64(3)), Op::Gt];
        constant_fold(&mut ops);
        assert_eq!(ops, vec![Op::Push(Value::Bool(true))]);
    }

    #[test]
    fn test_constant_fold_boolean() {
        let mut ops = vec![
            Op::Push(Value::Bool(true)),
            Op::Push(Value::Bool(false)),
            Op::And,
        ];
        constant_fold(&mut ops);
        assert_eq!(ops, vec![Op::Push(Value::Bool(false))]);
    }

    #[test]
    fn test_constant_fold_string_concat() {
        let mut ops = vec![
            Op::Push(Value::Str("hello ".into())),
            Op::Push(Value::Str("world".into())),
            Op::Add,
        ];
        constant_fold(&mut ops);
        assert_eq!(ops, vec![Op::Push(Value::Str("hello world".into()))]);
    }

    #[test]
    fn test_constant_fold_not() {
        let mut ops = vec![Op::Push(Value::Bool(true)), Op::Not];
        constant_fold(&mut ops);
        assert_eq!(ops, vec![Op::Push(Value::Bool(false))]);
    }

    #[test]
    fn test_constant_fold_no_div_by_zero() {
        let mut ops = vec![Op::Push(Value::U64(10)), Op::Push(Value::U64(0)), Op::Div];
        let changed = constant_fold(&mut ops);
        assert!(!changed);
        assert_eq!(ops.len(), 3);
    }

    #[test]
    fn test_constant_fold_chained() {
        // (2 + 3) * 4 → Push(5), Push(4), Mul → Push(20)
        let mut ops = vec![
            Op::Push(Value::U64(2)),
            Op::Push(Value::U64(3)),
            Op::Add,
            Op::Push(Value::U64(4)),
            Op::Mul,
        ];
        let mut methods = HashMap::new();
        methods.insert("main".into(), 0);
        optimize(&mut ops, &methods);
        assert!(ops.contains(&Op::Push(Value::U64(20))));
    }

    #[test]
    fn test_constant_fold_in_compiled_contract() {
        let src = r#"
contract ConstTest {
    state { x: u64 = 0 }
    fn get_const() -> u64 {
        return 10 + 20
    }
}
"#;
        let ast = parser::parse(src).unwrap();
        let bytecode = compile(&ast).unwrap();
        assert!(
            bytecode.opcodes.contains(&Op::Push(Value::U64(30))),
            "10+20 should be folded to 30, opcodes: {:?}",
            bytecode.opcodes
        );
        assert!(
            !bytecode.opcodes.contains(&Op::Add),
            "Add should be eliminated by constant folding"
        );
    }

    // ── M-11: Dead code elimination tests ─────────────────────────────────

    #[test]
    fn test_dead_code_after_return() {
        let src = r#"
contract DeadCode {
    state { x: u64 = 0 }
    fn early_return() -> u64 {
        return 42
        self.x = 999
    }
}
"#;
        let ast = parser::parse(src).unwrap();
        let bytecode = compile(&ast).unwrap();
        // The StateStore("x") after return should be eliminated (replaced with Pop)
        assert!(
            !bytecode.opcodes.contains(&Op::StateStore("x".into())),
            "dead code after return should be eliminated, opcodes: {:?}",
            bytecode.opcodes
        );
    }

    #[test]
    fn test_reachable_code_preserved() {
        let src = r#"
contract Alive {
    state { x: u64 = 0 }
    fn set(v: u64) {
        self.x = v
    }
}
"#;
        let ast = parser::parse(src).unwrap();
        let bytecode = compile(&ast).unwrap();
        assert!(bytecode.opcodes.contains(&Op::StateStore("x".into())));
    }

    #[test]
    fn test_dce_preserves_both_branches() {
        let src = r#"
contract Branching {
    state { x: u64 = 0 }
    fn test(v: u64) {
        if v > 0 {
            self.x = 1
        } else {
            self.x = 2
        }
    }
}
"#;
        let ast = parser::parse(src).unwrap();
        let bytecode = compile(&ast).unwrap();
        let state_stores: Vec<_> = bytecode
            .opcodes
            .iter()
            .filter(|op| matches!(op, Op::StateStore(_)))
            .collect();
        assert_eq!(state_stores.len(), 2, "both branches should be reachable");
    }
}
