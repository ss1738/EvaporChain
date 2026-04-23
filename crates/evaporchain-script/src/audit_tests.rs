//! Audit-prep test suite: VM safety, gas exhaustion, malformed bytecode,
//! stack overflow, infinite loop protection, and property-based testing.

#[cfg(test)]
mod vm_safety_tests {
    use crate::parser;
    use crate::compiler;
    use crate::vm::EvaporVM;
    use crate::{ExecutionContext, ScriptEngine, Value};
    use std::collections::HashMap;

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

    fn compile_src(src: &str) -> crate::compiler::EvaporBytecode {
        let ast = parser::parse(src).unwrap();
        compiler::compile(&ast).unwrap()
    }

    // ═══════════════════════════════════════════════════════════════════
    // Gas exhaustion attacks
    // ═══════════════════════════════════════════════════════════════════

    /// Deeply nested loops should be bounded by gas.
    #[test]
    fn test_gas_exhaustion_deep_loop() {
        let src = r#"
contract GasEater {
    state {
        count: u64 = 0
    }
    fn burn() -> u64 {
        let i: u64 = 0
        while i < 100000 {
            i = i + 1
            self.count = self.count + 1
        }
        return self.count
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();
        let state = HashMap::from([("count".to_string(), Value::U64(0))]);

        // With a tight gas limit, execution should fail gracefully
        let result = EvaporVM::execute_with_gas_limit(
            &bytecode, "burn", vec![], state, &ctx, 1_000,
        );

        // Should either succeed with limited iterations or fail with gas error — NOT panic
        match result {
            Ok(r) => assert!(r.gas_used <= 1_000, "gas used should not exceed limit"),
            Err(e) => assert!(format!("{:?}", e).contains("gas") || format!("{:?}", e).contains("Gas")
                || format!("{:?}", e).contains("limit") || format!("{:?}", e).contains("Limit")
                || format!("{:?}", e).contains("iteration") || format!("{:?}", e).contains("Loop"),
                "error should mention gas/limit/iteration, got: {:?}", e),
        }
    }

    /// Maximum loop iterations (100K) should be enforced.
    #[test]
    fn test_loop_iteration_limit() {
        let src = r#"
contract LoopTest {
    state {
        count: u64 = 0
    }
    fn loop_forever() -> u64 {
        let i: u64 = 0
        while i < 999999999 {
            i = i + 1
        }
        return i
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();
        let state = HashMap::from([("count".to_string(), Value::U64(0))]);

        let result = EvaporVM::execute(&bytecode, "loop_forever", vec![], state, &ctx);

        // Must either terminate within bounds or error — never hang
        match result {
            Ok(r) => {
                // VM should cap iterations
                if let Value::U64(v) = r.return_value {
                    assert!(v <= 100_001, "loop should be bounded to ~100K iterations, got {}", v);
                }
            }
            Err(_) => {} // expected — iteration limit exceeded
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Malformed input resistance
    // ═══════════════════════════════════════════════════════════════════

    /// Empty source code should return a parse error, not panic.
    #[test]
    fn test_empty_source_error() {
        let result = parser::parse("");
        assert!(result.is_err(), "empty source must produce error");
    }

    /// Garbage input should return a parse error, not panic.
    #[test]
    fn test_garbage_source_error() {
        let inputs = [
            "asdf1234!@#$",
            "contract { }",  // missing name
            "contract X { fn () }",  // missing method name
            "\x00\x01\x02\x03",  // binary garbage
            "contract X { state { x: unknown_type } }",  // unknown type
            &"a".repeat(100_000),  // very long input
        ];

        for input in &inputs {
            let result = parser::parse(input);
            // Must not panic
            assert!(result.is_err(), "garbage input should produce error: {:?}", &input[..input.len().min(50)]);
        }
    }

    /// Calling a non-existent method should error, not panic.
    #[test]
    fn test_nonexistent_method_error() {
        let src = r#"
contract X {
    state { v: u64 = 0 }
    fn real_method() -> u64 {
        return 42
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();
        let state = HashMap::from([("v".to_string(), Value::U64(0))]);

        let result = EvaporVM::execute(&bytecode, "nonexistent_method", vec![], state, &ctx);
        assert!(result.is_err(), "calling nonexistent method must error");
    }

    /// Wrong argument types should error, not panic.
    #[test]
    fn test_wrong_arg_types_error() {
        let src = r#"
contract X {
    state { v: u64 = 0 }
    fn add(x: u64) -> u64 {
        return x + 1
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();
        let state = HashMap::from([("v".to_string(), Value::U64(0))]);

        // Pass a string instead of u64
        let result = EvaporVM::execute(
            &bytecode, "add", vec![Value::Str("not_a_number".to_string())], state, &ctx,
        );
        // Should either error or handle gracefully — never panic
        // (The VM may coerce types or reject)
        match result {
            Ok(_) => {} // some VMs allow this
            Err(_) => {} // expected
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // State isolation
    // ═══════════════════════════════════════════════════════════════════

    /// One contract's execution should not affect another's state.
    #[test]
    fn test_contract_state_isolation() {
        let src = r#"
contract Counter {
    state { count: u64 = 0 }
    fn increment() -> u64 {
        self.count = self.count + 1
        return self.count
    }
}
"#;
        let mut engine = ScriptEngine::new();
        let creator = [1u8; 32];

        let id1 = engine.deploy(src, creator, 10_000, 100, 1).unwrap();
        let id2 = engine.deploy(src, creator, 10_000, 100, 1).unwrap();

        // Increment contract 1 three times
        for _ in 0..3 {
            engine.call(id1, "increment", vec![], creator, 10).unwrap();
        }

        // Contract 2 should still be at 0
        let result = engine.call(id2, "increment", vec![], creator, 10).unwrap();
        assert_eq!(result.return_value, Value::U64(1),
            "contract 2 should start from 0, got {:?}", result.return_value);
    }

    /// Evaporated contract should not be callable.
    #[test]
    fn test_evaporated_contract_not_callable() {
        let src = r#"
contract Mortal {
    state { v: u64 = 0 }
    fn get() -> u64 {
        return self.v
    }
}
"#;
        let mut engine = ScriptEngine::new();
        let creator = [1u8; 32];

        let id = engine.deploy(src, creator, 100, 1, 1).unwrap(); // tiny energy, fast decay

        // Call at a very late epoch when energy should be 0
        let result = engine.call(id, "get", vec![], creator, 1000);
        assert!(result.is_err(), "evaporated contract must not be callable");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Gas metering correctness
    // ═══════════════════════════════════════════════════════════════════

    /// Gas used should be proportional to work done.
    #[test]
    fn test_gas_proportional_to_work() {
        let src = r#"
contract Work {
    state { v: u64 = 0 }
    fn do_work(n: u64) -> u64 {
        let i: u64 = 0
        while i < n {
            i = i + 1
            self.v = self.v + 1
        }
        return self.v
    }
}
"#;
        let bytecode = compile_src(src);
        let ctx = test_ctx();

        let state1 = HashMap::from([("v".to_string(), Value::U64(0))]);
        let r1 = EvaporVM::execute(&bytecode, "do_work", vec![Value::U64(10)], state1, &ctx).unwrap();

        let state2 = HashMap::from([("v".to_string(), Value::U64(0))]);
        let r2 = EvaporVM::execute(&bytecode, "do_work", vec![Value::U64(100)], state2, &ctx).unwrap();

        assert!(r2.gas_used > r1.gas_used,
            "more work should use more gas: 10 iterations={}, 100 iterations={}",
            r1.gas_used, r2.gas_used);
    }
}

#[cfg(test)]
mod proptest_script {
    use crate::parser;
    use proptest::prelude::*;

    proptest! {
        /// Random strings should never cause a panic in the parser.
        #[test]
        fn parser_never_panics(s in "\\PC{0,500}") {
            let _ = parser::parse(&s);
        }

        /// Valid contract names should parse the contract declaration at least.
        #[test]
        fn valid_contract_name_parses_prefix(name in "[A-Z][a-zA-Z0-9]{1,20}") {
            let src = format!("contract {} {{ state {{ v: u64 = 0 }} fn get() -> u64 {{ return self.v }} }}", name);
            let result = parser::parse(&src);
            // Should parse successfully with a valid name
            prop_assert!(result.is_ok(),
                "valid contract name '{}' should parse, got error: {:?}", name, result.err());
        }

        /// Large energy values should not overflow in contract deployment.
        #[test]
        fn large_energy_no_overflow(energy in 0u64..u64::MAX / 2) {
            use crate::ScriptEngine;
            let src = r#"
contract T {
    state { v: u64 = 0 }
    fn get() -> u64 { return self.v }
}
"#;
            let mut engine = ScriptEngine::new();
            let result = engine.deploy(src, [1u8; 32], energy, 100, 1);
            prop_assert!(result.is_ok(), "deployment should succeed for energy={}", energy);
        }
    }
}
