//! §EvaporScript VM e2e
//!
//! Scenario: "PRIYA runtime dispatch arc" — PRIYA is the EvaporScript
//! contract runtime. She deploys a counter contract for RAFAEL, who
//! increments and reads it. IGOR attempts an underflow — blocked by
//! the require() guard. PRIYA verifies two contracts have isolated
//! state, exercises the gas metering path (tight budget → GasLimitExceeded,
//! generous budget succeeds), confirms malformed source is rejected at
//! parse time with no contract persisted, and ticks a low-energy
//! contract to evaporation.
//!
//! The suite proves: ScriptEngine::deploy assigns sequential ids; state
//! CRUD round-trips; two contracts have independent state; require()
//! rejects with RequireFailed; GasLimitExceeded fires before completion
//! under a tight vm_gas_limit; get_abi returns correct method names;
//! list() returns deployed contracts ordered by id; tick() evaporates
//! zero-energy contracts; calling an evaporated contract is a Runtime
//! error; malformed source is a Parse error.

use evaporchain_script::{ScriptEngine, ScriptError, Value};

const COUNTER: &str = r#"
contract Counter {
    state {
        count: u64 = 0
    }

    fn increment(by: u64) {
        self.count = self.count + by
    }

    fn decrement(by: u64) {
        require(self.count >= by, "underflow")
        self.count = self.count - by
    }

    fn get() -> u64 {
        return self.count
    }

    fn reset() {
        self.count = 0
    }
}
"#;

const BURNER: &str = r#"
contract Burner {
    state { counter: u64 = 0 }

    fn run() -> u64 {
        while (self.counter < 90000) {
            self.counter = self.counter + 1
        }
        return self.counter
    }
}
"#;

fn creator() -> [u8; 32] {
    [0xAA; 32]
}
fn rafael() -> [u8; 32] {
    [0xBB; 32]
}
fn igor() -> [u8; 32] {
    [0xFF; 32]
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn deploy_assigns_sequential_ids() {
    let mut engine = ScriptEngine::new();
    let id1 = engine.deploy(COUNTER, creator(), 10_000, 100, 0).unwrap();
    let id2 = engine.deploy(COUNTER, creator(), 10_000, 100, 0).unwrap();
    assert_eq!(id1, 1, "first deploy must be id 1");
    assert_eq!(id2, 2, "second deploy must be id 2");
}

#[test]
fn increment_and_get_round_trip() {
    let mut engine = ScriptEngine::new();
    let id = engine.deploy(COUNTER, creator(), 10_000, 100, 0).unwrap();

    let r = engine.call(id, "get", vec![], rafael(), 0).unwrap();
    assert_eq!(r.return_value, Value::U64(0), "initial count must be 0");

    engine
        .call(id, "increment", vec![Value::U64(5)], rafael(), 0)
        .unwrap();
    let r = engine.call(id, "get", vec![], rafael(), 0).unwrap();
    assert_eq!(r.return_value, Value::U64(5));

    engine
        .call(id, "increment", vec![Value::U64(3)], rafael(), 0)
        .unwrap();
    let r = engine.call(id, "get", vec![], rafael(), 0).unwrap();
    assert_eq!(r.return_value, Value::U64(8));
}

#[test]
fn decrement_reduces_count() {
    let mut engine = ScriptEngine::new();
    let id = engine.deploy(COUNTER, creator(), 10_000, 100, 0).unwrap();
    engine
        .call(id, "increment", vec![Value::U64(10)], creator(), 0)
        .unwrap();
    engine
        .call(id, "decrement", vec![Value::U64(4)], creator(), 0)
        .unwrap();
    let r = engine.call(id, "get", vec![], creator(), 0).unwrap();
    assert_eq!(r.return_value, Value::U64(6));
}

#[test]
fn decrement_underflow_fires_require_failed() {
    let mut engine = ScriptEngine::new();
    let id = engine.deploy(COUNTER, creator(), 10_000, 100, 0).unwrap();
    // count is 0; decrement by 1 must hit require(0 >= 1) → false.
    let err = engine
        .call(id, "decrement", vec![Value::U64(1)], igor(), 0)
        .unwrap_err();
    assert!(
        matches!(err, ScriptError::RequireFailed(_)),
        "underflow must produce RequireFailed, got {err:?}"
    );
}

#[test]
fn reset_zeroes_the_counter() {
    let mut engine = ScriptEngine::new();
    let id = engine.deploy(COUNTER, creator(), 10_000, 100, 0).unwrap();
    engine
        .call(id, "increment", vec![Value::U64(99)], creator(), 0)
        .unwrap();
    engine.call(id, "reset", vec![], creator(), 0).unwrap();
    let r = engine.call(id, "get", vec![], creator(), 0).unwrap();
    assert_eq!(r.return_value, Value::U64(0), "reset must zero the counter");
}

#[test]
fn two_contracts_have_isolated_state() {
    let mut engine = ScriptEngine::new();
    let id_a = engine.deploy(COUNTER, creator(), 10_000, 100, 0).unwrap();
    let id_b = engine.deploy(COUNTER, creator(), 10_000, 100, 0).unwrap();

    engine
        .call(id_a, "increment", vec![Value::U64(7)], creator(), 0)
        .unwrap();
    engine
        .call(id_b, "increment", vec![Value::U64(3)], creator(), 0)
        .unwrap();

    let ra = engine.call(id_a, "get", vec![], creator(), 0).unwrap();
    let rb = engine.call(id_b, "get", vec![], creator(), 0).unwrap();

    assert_eq!(ra.return_value, Value::U64(7), "contract A count must be 7");
    assert_eq!(rb.return_value, Value::U64(3), "contract B count must be 3");
}

#[test]
fn call_tracks_gas_used() {
    let mut engine = ScriptEngine::new();
    let id = engine.deploy(COUNTER, creator(), 10_000, 100, 0).unwrap();
    let r = engine
        .call(id, "increment", vec![Value::U64(1)], creator(), 0)
        .unwrap();
    assert!(r.gas_used > 0, "gas_used must be positive after a call");
}

#[test]
fn tight_vm_gas_fires_gas_limit_exceeded() {
    let mut engine = ScriptEngine::new();
    let id = engine.deploy(BURNER, creator(), 10_000, 100, 0).unwrap();
    // 50 VM-gas units cannot complete even one while-loop iteration.
    let err = engine
        .call_with_vm_gas(id, "run", vec![], creator(), 0, 50)
        .unwrap_err();
    assert!(
        matches!(err, ScriptError::GasLimitExceeded { .. }),
        "tight vm_gas_limit must produce GasLimitExceeded, got {err:?}"
    );
}

#[test]
fn generous_vm_gas_lets_loop_complete() {
    let mut engine = ScriptEngine::new();
    let id = engine.deploy(BURNER, creator(), 10_000, 100, 0).unwrap();
    let r = engine
        .call_with_vm_gas(id, "run", vec![], creator(), 0, 5_000_000)
        .unwrap();
    assert_eq!(r.return_value, Value::U64(90_000));
}

#[test]
fn malformed_source_is_parse_error() {
    let mut engine = ScriptEngine::new();
    let bad = "contract Broken { state { x: u64 method no_close";
    let err = engine.deploy(bad, creator(), 10_000, 100, 0).unwrap_err();
    assert!(
        matches!(err, ScriptError::Parse { .. }),
        "malformed source must produce Parse error, got {err:?}"
    );
}

#[test]
fn failed_deploy_does_not_persist_contract() {
    let mut engine = ScriptEngine::new();
    let _ = engine.deploy(
        "not valid evaporscript source !!",
        creator(),
        10_000,
        100,
        0,
    );
    assert_eq!(
        engine.list().len(),
        0,
        "failed deploy must not persist a contract"
    );
}

#[test]
fn unknown_contract_id_is_runtime_error() {
    let mut engine = ScriptEngine::new();
    let err = engine.call(999, "get", vec![], creator(), 0).unwrap_err();
    assert!(
        matches!(err, ScriptError::Runtime(_)),
        "unknown contract id must produce Runtime error, got {err:?}"
    );
}

#[test]
fn get_abi_returns_method_names() {
    let mut engine = ScriptEngine::new();
    let id = engine.deploy(COUNTER, creator(), 10_000, 100, 0).unwrap();
    let abi = engine.get_abi(id).unwrap();
    let names: Vec<&str> = abi.methods.iter().map(|m| m.name.as_str()).collect();
    assert!(names.contains(&"increment"), "ABI must list 'increment'");
    assert!(names.contains(&"decrement"), "ABI must list 'decrement'");
    assert!(names.contains(&"get"), "ABI must list 'get'");
    assert!(names.contains(&"reset"), "ABI must list 'reset'");
}

#[test]
fn list_returns_contracts_sorted_by_id() {
    let mut engine = ScriptEngine::new();
    let id1 = engine.deploy(COUNTER, creator(), 10_000, 100, 0).unwrap();
    let id2 = engine.deploy(COUNTER, creator(), 10_000, 100, 0).unwrap();
    let contracts = engine.list();
    assert_eq!(contracts.len(), 2);
    assert_eq!(contracts[0].id, id1);
    assert_eq!(contracts[1].id, id2);
}

#[test]
fn tick_evaporates_zero_energy_contract() {
    let mut engine = ScriptEngine::new();
    // energy_at_epoch(64, half_life=1, elapsed=7) = 64 >> 7 = 0
    let id = engine.deploy(COUNTER, creator(), 64, 1, 0).unwrap();

    // Callable before evaporation at epoch 0 (elapsed = 0).
    engine
        .call(id, "increment", vec![Value::U64(1)], creator(), 0)
        .unwrap();

    let tick = engine.tick(7);
    assert!(
        tick.contracts_evaporated.contains(&id),
        "zero-energy contract must be evaporated by tick"
    );
}

#[test]
fn call_on_evaporated_contract_is_runtime_error() {
    let mut engine = ScriptEngine::new();
    let id = engine.deploy(COUNTER, creator(), 64, 1, 0).unwrap();
    engine.tick(7); // evaporate

    let err = engine.call(id, "get", vec![], creator(), 7).unwrap_err();
    assert!(
        matches!(err, ScriptError::Runtime(_)),
        "call on evaporated contract must produce Runtime error, got {err:?}"
    );
}

#[test]
fn priya_runtime_full_arc() {
    // Full arc: PRIYA deploys two contracts. RAFAEL uses contract A.
    // IGOR attempts underflow — blocked. State isolation verified.
    // Gas metering rejects a tight-budget run. Tick evaporates B.

    let mut engine = ScriptEngine::new();

    let id_a = engine.deploy(COUNTER, creator(), 10_000, 100, 0).unwrap();
    // Contract B: low energy, short half_life — evaporates at epoch 7.
    let id_b = engine.deploy(COUNTER, creator(), 64, 1, 0).unwrap();

    // RAFAEL increments contract A repeatedly.
    engine
        .call(id_a, "increment", vec![Value::U64(100)], rafael(), 0)
        .unwrap();
    engine
        .call(id_a, "increment", vec![Value::U64(50)], rafael(), 0)
        .unwrap();
    let r = engine.call(id_a, "get", vec![], rafael(), 0).unwrap();
    assert_eq!(r.return_value, Value::U64(150));

    // IGOR tries to underflow contract A — require() blocks it.
    let err = engine
        .call(id_a, "decrement", vec![Value::U64(999)], igor(), 0)
        .unwrap_err();
    assert!(matches!(err, ScriptError::RequireFailed(_)));

    // Contract A state is unaffected by IGOR's failed attempt.
    let r = engine.call(id_a, "get", vec![], rafael(), 0).unwrap();
    assert_eq!(r.return_value, Value::U64(150));

    // State isolation: contract B's count is still 0.
    let rb = engine.call(id_b, "get", vec![], creator(), 0).unwrap();
    assert_eq!(rb.return_value, Value::U64(0));

    // Gas metering: tight budget rejects a loop on contract A.
    let id_burn = engine.deploy(BURNER, creator(), 10_000, 100, 0).unwrap();
    let gas_err = engine
        .call_with_vm_gas(id_burn, "run", vec![], creator(), 0, 50)
        .unwrap_err();
    assert!(matches!(gas_err, ScriptError::GasLimitExceeded { .. }));

    // RAFAEL resets contract A.
    engine.call(id_a, "reset", vec![], rafael(), 0).unwrap();
    let r = engine.call(id_a, "get", vec![], rafael(), 0).unwrap();
    assert_eq!(r.return_value, Value::U64(0));

    // Tick: contract B evaporates; A survives.
    let tick = engine.tick(7);
    assert!(
        tick.contracts_evaporated.contains(&id_b),
        "B must evaporate at epoch 7"
    );
    assert!(!tick.contracts_evaporated.contains(&id_a), "A must survive");

    // Contract B is dead — subsequent call rejected.
    assert!(engine.call(id_b, "get", vec![], creator(), 7).is_err());

    // Contract A still accepts calls.
    engine
        .call(id_a, "increment", vec![Value::U64(1)], rafael(), 7)
        .unwrap();
}
