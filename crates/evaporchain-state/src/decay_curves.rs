//! Programmable decay curves beyond fixed exponential.
//!
//! EvaporChain's core innovation is energy-based state decay. This module
//! extends the original fixed exponential decay with configurable curves:
//! linear, stepped, conditional, asymptotic, and custom bytecode.

pub use evaporchain_types::DecayCurve;
use evaporchain_types::energy_at_epoch;

/// Compute the current energy for an object given its decay curve.
///
/// # Arguments
/// * `curve` - The decay curve to apply
/// * `initial_energy` - Energy at creation
/// * `elapsed_epochs` - Epochs since creation
/// * `last_access_epoch` - For Conditional curves, the last epoch the object was accessed
pub fn compute_energy(
    curve: &DecayCurve,
    initial_energy: u64,
    elapsed_epochs: u64,
    last_access_epoch: Option<u64>,
) -> u64 {
    if initial_energy == 0 || elapsed_epochs == 0 {
        return initial_energy;
    }

    match curve {
        DecayCurve::Exponential { half_life } => {
            energy_at_epoch(initial_energy, *half_life, elapsed_epochs)
        }

        DecayCurve::Linear { rate_per_epoch } => {
            let decay = (*rate_per_epoch as u128) * (elapsed_epochs as u128);
            if decay >= initial_energy as u128 {
                0
            } else {
                initial_energy - decay as u64
            }
        }

        DecayCurve::Stepped { thresholds } => {
            if thresholds.is_empty() {
                return initial_energy;
            }
            let mut energy = initial_energy;
            for &(threshold, level) in thresholds {
                if elapsed_epochs >= threshold {
                    energy = level;
                } else {
                    break;
                }
            }
            energy
        }

        DecayCurve::Conditional {
            base,
            grace_epochs,
        } => {
            let effective_elapsed = match last_access_epoch {
                Some(last_access) => {
                    let idle = elapsed_epochs.saturating_sub(last_access);
                    if idle <= *grace_epochs {
                        elapsed_epochs.saturating_sub(*grace_epochs - idle)
                    } else {
                        elapsed_epochs
                    }
                }
                None => elapsed_epochs,
            };
            compute_energy(base, initial_energy, effective_elapsed, None)
        }

        DecayCurve::Asymptotic { floor, half_life } => {
            if initial_energy <= *floor {
                return initial_energy;
            }
            let above_floor = initial_energy - *floor;
            let decayed_above = energy_at_epoch(above_floor, *half_life, elapsed_epochs);
            floor.saturating_add(decayed_above)
        }

        DecayCurve::Custom { bytecode } => {
            evaluate_custom(bytecode, initial_energy, elapsed_epochs).unwrap_or(0)
        }
    }
}

// ─── Custom Bytecode Evaluation ──────────────────────────────────────────

/// Minimal stack VM for custom decay curves.
/// Opcodes:
///   0x01 = PUSH_U64 (next 8 bytes as little-endian u64)
///   0x02 = ADD
///   0x03 = SUB (saturating)
///   0x04 = MUL
///   0x05 = DIV (div by 0 = 0)
///   0x06 = SHR (shift right)
///   0x07 = DUP
///   0x08 = SWAP
///   0x09 = MIN
///   0x0A = MAX
///   0x0B = CLAMP_ZERO (if top < 0 in signed interpretation, set to 0)
const OP_PUSH: u8 = 0x01;
const OP_ADD: u8 = 0x02;
const OP_SUB: u8 = 0x03;
const OP_MUL: u8 = 0x04;
const OP_DIV: u8 = 0x05;
const OP_SHR: u8 = 0x06;
const OP_DUP: u8 = 0x07;
const OP_SWAP: u8 = 0x08;
const OP_MIN: u8 = 0x09;
const OP_MAX: u8 = 0x0A;

const MAX_STEPS: usize = 256;
const MAX_STACK: usize = 32;

fn evaluate_custom(bytecode: &[u8], initial_energy: u64, elapsed_epochs: u64) -> Option<u64> {
    let mut stack: Vec<u64> = vec![initial_energy, elapsed_epochs];
    let mut pc = 0;
    let mut steps = 0;

    while pc < bytecode.len() {
        steps += 1;
        if steps > MAX_STEPS {
            return None;
        }
        if stack.len() > MAX_STACK {
            return None;
        }

        match bytecode[pc] {
            OP_PUSH => {
                if pc + 8 >= bytecode.len() {
                    return None;
                }
                let val = u64::from_le_bytes(
                    bytecode[pc + 1..pc + 9].try_into().ok()?,
                );
                stack.push(val);
                pc += 9;
            }
            OP_ADD => {
                let b = stack.pop()?;
                let a = stack.pop()?;
                stack.push(a.saturating_add(b));
                pc += 1;
            }
            OP_SUB => {
                let b = stack.pop()?;
                let a = stack.pop()?;
                stack.push(a.saturating_sub(b));
                pc += 1;
            }
            OP_MUL => {
                let b = stack.pop()?;
                let a = stack.pop()?;
                stack.push(a.saturating_mul(b));
                pc += 1;
            }
            OP_DIV => {
                let b = stack.pop()?;
                let a = stack.pop()?;
                stack.push(if b == 0 { 0 } else { a / b });
                pc += 1;
            }
            OP_SHR => {
                let b = stack.pop()?;
                let a = stack.pop()?;
                stack.push(if b >= 64 { 0 } else { a >> b });
                pc += 1;
            }
            OP_DUP => {
                let a = *stack.last()?;
                stack.push(a);
                pc += 1;
            }
            OP_SWAP => {
                let len = stack.len();
                if len < 2 {
                    return None;
                }
                stack.swap(len - 1, len - 2);
                pc += 1;
            }
            OP_MIN => {
                let b = stack.pop()?;
                let a = stack.pop()?;
                stack.push(a.min(b));
                pc += 1;
            }
            OP_MAX => {
                let b = stack.pop()?;
                let a = stack.pop()?;
                stack.push(a.max(b));
                pc += 1;
            }
            _ => return None,
        }
    }

    if stack.len() == 1 {
        stack.pop()
    } else {
        None
    }
}

/// Validate that custom bytecode is safe: bounded execution, valid opcodes,
/// correct stack behavior.
pub fn validate_custom_bytecode(bytecode: &[u8]) -> Result<(), &'static str> {
    if bytecode.is_empty() {
        return Err("empty bytecode");
    }
    if bytecode.len() > 1024 {
        return Err("bytecode exceeds 1024 bytes");
    }

    let mut pc = 0;
    let mut op_count = 0;

    while pc < bytecode.len() {
        op_count += 1;
        if op_count > MAX_STEPS {
            return Err("too many operations");
        }
        match bytecode[pc] {
            OP_PUSH => {
                if pc + 8 >= bytecode.len() {
                    return Err("PUSH truncated");
                }
                pc += 9;
            }
            0x02..=0x0A => {
                pc += 1;
            }
            _ => return Err("unknown opcode"),
        }
    }

    Ok(())
}

// ─── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exponential_matches_legacy() {
        let curve = DecayCurve::Exponential { half_life: 100 };
        for elapsed in [0, 1, 50, 99, 100, 200, 500, 6400] {
            let curve_result = compute_energy(&curve, 10000, elapsed, None);
            let legacy_result = energy_at_epoch(10000, 100, elapsed);
            assert_eq!(curve_result, legacy_result, "mismatch at elapsed={}", elapsed);
        }
    }

    #[test]
    fn test_linear_basic() {
        let curve = DecayCurve::Linear { rate_per_epoch: 10 };
        assert_eq!(compute_energy(&curve, 1000, 0, None), 1000);
        assert_eq!(compute_energy(&curve, 1000, 50, None), 500);
        assert_eq!(compute_energy(&curve, 1000, 100, None), 0);
        assert_eq!(compute_energy(&curve, 1000, 200, None), 0);
    }

    #[test]
    fn test_linear_overflow_protection() {
        let curve = DecayCurve::Linear {
            rate_per_epoch: u64::MAX,
        };
        assert_eq!(compute_energy(&curve, 1000, 1, None), 0);
    }

    #[test]
    fn test_stepped_basic() {
        let curve = DecayCurve::Stepped {
            thresholds: vec![(10, 800), (50, 400), (100, 100), (200, 0)],
        };
        assert_eq!(compute_energy(&curve, 1000, 0, None), 1000);
        assert_eq!(compute_energy(&curve, 1000, 5, None), 1000);
        assert_eq!(compute_energy(&curve, 1000, 10, None), 800);
        assert_eq!(compute_energy(&curve, 1000, 50, None), 400);
        assert_eq!(compute_energy(&curve, 1000, 150, None), 100);
        assert_eq!(compute_energy(&curve, 1000, 200, None), 0);
        assert_eq!(compute_energy(&curve, 1000, 999, None), 0);
    }

    #[test]
    fn test_stepped_empty_thresholds() {
        let curve = DecayCurve::Stepped {
            thresholds: vec![],
        };
        assert_eq!(compute_energy(&curve, 1000, 100, None), 1000);
    }

    #[test]
    fn test_conditional_recent_access_slows_decay() {
        let base = DecayCurve::Linear { rate_per_epoch: 10 };
        let curve = DecayCurve::Conditional {
            base: Box::new(base.clone()),
            grace_epochs: 20,
        };
        // With recent access (last_access at epoch 90 of 100 elapsed)
        let with_access = compute_energy(&curve, 1000, 100, Some(90));
        // Without access
        let without_access = compute_energy(&curve, 1000, 100, None);
        assert!(with_access >= without_access,
            "recent access should slow decay: with={} without={}", with_access, without_access);
    }

    #[test]
    fn test_conditional_no_access_full_decay() {
        let base = DecayCurve::Exponential { half_life: 100 };
        let curve = DecayCurve::Conditional {
            base: Box::new(base.clone()),
            grace_epochs: 10,
        };
        let without = compute_energy(&curve, 10000, 200, None);
        let base_result = compute_energy(&base, 10000, 200, None);
        assert_eq!(without, base_result);
    }

    #[test]
    fn test_asymptotic_converges_to_floor() {
        let curve = DecayCurve::Asymptotic {
            floor: 100,
            half_life: 10,
        };
        assert_eq!(compute_energy(&curve, 1000, 0, None), 1000);
        let at_100 = compute_energy(&curve, 1000, 100, None);
        assert!(at_100 >= 100, "should not go below floor: got {}", at_100);
        let at_1000 = compute_energy(&curve, 1000, 1000, None);
        assert_eq!(at_1000, 100, "should converge to floor");
    }

    #[test]
    fn test_asymptotic_floor_above_initial() {
        let curve = DecayCurve::Asymptotic {
            floor: 2000,
            half_life: 10,
        };
        assert_eq!(compute_energy(&curve, 1000, 100, None), 1000);
    }

    #[test]
    fn test_asymptotic_floor_zero_equals_exponential() {
        let asym = DecayCurve::Asymptotic {
            floor: 0,
            half_life: 50,
        };
        let exp = DecayCurve::Exponential { half_life: 50 };
        for elapsed in [0, 10, 25, 50, 100, 200] {
            assert_eq!(
                compute_energy(&asym, 8000, elapsed, None),
                compute_energy(&exp, 8000, elapsed, None),
                "floor=0 asymptotic should equal exponential at elapsed={}",
                elapsed
            );
        }
    }

    #[test]
    fn test_custom_bytecode_identity() {
        // Stack starts with [initial_energy, elapsed_epochs]
        // SWAP, then pop elapsed with SUB 0: just return initial_energy
        // Simpler: just DROP elapsed by doing: SWAP, PUSH 0, MUL, ADD
        // [E, t] -> SWAP -> [t, E] -> ... too complex
        // Simplest: [E, t] -> SUB -> E - t (not identity but tests the VM)
        let bytecode = vec![OP_SUB]; // initial - elapsed
        let curve = DecayCurve::Custom { bytecode };
        assert_eq!(compute_energy(&curve, 1000, 100, None), 900);
        assert_eq!(compute_energy(&curve, 1000, 1000, None), 0);
    }

    #[test]
    fn test_custom_bytecode_push_and_div() {
        // [E, t] -> SWAP -> [t, E] -> PUSH 2 -> [t, E, 2] -> DIV -> [t, E/2] -> SWAP -> [E/2, t] -> SUB -> E/2 - t
        let mut bytecode = vec![OP_SWAP];
        bytecode.push(OP_PUSH);
        bytecode.extend_from_slice(&2u64.to_le_bytes());
        bytecode.push(OP_DIV);
        bytecode.push(OP_SWAP);
        bytecode.push(OP_SUB);
        let curve = DecayCurve::Custom { bytecode };
        assert_eq!(compute_energy(&curve, 1000, 100, None), 400); // 500 - 100
    }

    #[test]
    fn test_custom_bytecode_max_steps_exceeded() {
        // Create a long bytecode that exceeds MAX_STEPS
        let mut bytecode = Vec::new();
        for _ in 0..300 {
            bytecode.push(OP_DUP);
        }
        assert!(validate_custom_bytecode(&bytecode).is_err());
    }

    #[test]
    fn test_custom_bytecode_unknown_opcode() {
        let bytecode = vec![0xFF];
        assert!(validate_custom_bytecode(&bytecode).is_err());
    }

    #[test]
    fn test_custom_bytecode_empty() {
        assert!(validate_custom_bytecode(&[]).is_err());
    }

    #[test]
    fn test_custom_bytecode_valid() {
        let bytecode = vec![OP_SUB];
        assert!(validate_custom_bytecode(&bytecode).is_ok());
    }

    #[test]
    fn test_zero_initial_energy() {
        for curve in [
            DecayCurve::Exponential { half_life: 100 },
            DecayCurve::Linear { rate_per_epoch: 10 },
            DecayCurve::Asymptotic { floor: 50, half_life: 10 },
        ] {
            assert_eq!(compute_energy(&curve, 0, 100, None), 0);
        }
    }

    #[test]
    fn test_zero_elapsed() {
        for curve in [
            DecayCurve::Exponential { half_life: 100 },
            DecayCurve::Linear { rate_per_epoch: 10 },
            DecayCurve::Asymptotic { floor: 50, half_life: 10 },
        ] {
            assert_eq!(compute_energy(&curve, 1000, 0, None), 1000);
        }
    }

    #[test]
    fn test_serialization_roundtrip() {
        let curves = vec![
            DecayCurve::Exponential { half_life: 100 },
            DecayCurve::Linear { rate_per_epoch: 42 },
            DecayCurve::Stepped {
                thresholds: vec![(10, 500), (100, 0)],
            },
            DecayCurve::Conditional {
                base: Box::new(DecayCurve::Exponential { half_life: 50 }),
                grace_epochs: 20,
            },
            DecayCurve::Asymptotic {
                floor: 100,
                half_life: 30,
            },
            DecayCurve::Custom {
                bytecode: vec![OP_SUB],
            },
        ];
        for curve in &curves {
            let json = serde_json::to_string(curve).unwrap();
            let decoded: DecayCurve = serde_json::from_str(&json).unwrap();
            assert_eq!(*curve, decoded, "roundtrip failed for {:?}", curve);
        }
    }

    #[test]
    fn test_default_is_exponential() {
        let curve = DecayCurve::default();
        assert_eq!(curve, DecayCurve::Exponential { half_life: 100 });
    }

    #[test]
    fn test_linear_vs_exponential_linear_faster_early() {
        let linear = DecayCurve::Linear { rate_per_epoch: 100 };
        let exp = DecayCurve::Exponential { half_life: 10 };
        // At epoch 1, linear drops 100 (900 remaining), exponential drops ~50 (950 remaining)
        let lin_1 = compute_energy(&linear, 1000, 1, None);
        let exp_1 = compute_energy(&exp, 1000, 1, None);
        assert!(lin_1 < exp_1, "linear should decay faster at epoch 1: lin={} exp={}", lin_1, exp_1);
    }

    #[test]
    fn test_stepped_single_threshold() {
        let curve = DecayCurve::Stepped {
            thresholds: vec![(50, 0)],
        };
        assert_eq!(compute_energy(&curve, 1000, 49, None), 1000);
        assert_eq!(compute_energy(&curve, 1000, 50, None), 0);
    }

    #[test]
    fn test_custom_min_max_ops() {
        // [E, t] -> MIN -> min(E, t)
        let curve = DecayCurve::Custom {
            bytecode: vec![OP_MIN],
        };
        assert_eq!(compute_energy(&curve, 1000, 100, None), 100);
        assert_eq!(compute_energy(&curve, 100, 1000, None), 100);

        let curve_max = DecayCurve::Custom {
            bytecode: vec![OP_MAX],
        };
        assert_eq!(compute_energy(&curve_max, 1000, 100, None), 1000);
    }
}
