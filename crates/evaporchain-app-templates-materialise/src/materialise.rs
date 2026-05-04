//! `MaterialiseInstruction` — the dispatch envelope the chain's
//! `ContractEngine` consumes to instantiate an application template.
//!
//! `materialise_request` is the driver: validate again at execution
//! time → derive the instance id → re-canonicalize the params into
//! `init_calldata` → emit the instruction. Pure function; no state.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use evaporchain_app_templates::{find, CatalogueError, TemplateClass};
use evaporchain_app_templates_deploy::{
    validate_against_descriptor, DeployRequest, ValidationError,
};

use crate::instance::{derive_instance_id, InstanceId};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MaterialiseError {
    #[error("template class {0:#010x} is not registered in the catalogue")]
    UnknownTemplate(u32),
    #[error("schema validation failed at materialise time: {0}")]
    SchemaInvalid(#[from] ValidationError),
    #[error("canonical params encoding exceeds u32 max bytes")]
    CalldataTooLarge,
}

impl From<CatalogueError> for MaterialiseError {
    fn from(e: CatalogueError) -> Self {
        match e {
            CatalogueError::NotFound(c) => Self::UnknownTemplate(c),
        }
    }
}

/// The dispatch envelope. The contract engine reads `template_class`
/// to pick the right typed materialiser, addresses the new instance
/// by `instance_id`, and feeds it `init_calldata` (canonical params).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialiseInstruction {
    pub template_class: TemplateClass,
    pub instance_id: InstanceId,
    pub init_calldata: Vec<u8>,
}

/// Drive a validated [`DeployRequest`] all the way to a
/// [`MaterialiseInstruction`].
pub fn materialise_request(
    req: &DeployRequest,
) -> Result<MaterialiseInstruction, MaterialiseError> {
    // Re-fetch the descriptor at materialise time. Defensive against
    // catalogue drift between submit and execution.
    let descriptor = find(req.template_class)?;

    // Re-validate. Two-phase validation: deploy layer at submit,
    // materialiser at execution. If catalogue's required-keys
    // contract has hardened since submit, this rejects rather than
    // silently materialising an under-specified instance.
    validate_against_descriptor(req, &descriptor)?;

    let instance_id = derive_instance_id(req.template_class, &req.deployer, req.nonce);

    // Init calldata: same canonical-JSON encoding as deploy commitment.
    let init_calldata = canonical_json_bytes(&req.params);
    if init_calldata.len() > u32::MAX as usize {
        return Err(MaterialiseError::CalldataTooLarge);
    }

    Ok(MaterialiseInstruction {
        template_class: req.template_class,
        instance_id,
        init_calldata,
    })
}

fn canonical_json_bytes(v: &serde_json::Value) -> Vec<u8> {
    let canonical = canonicalize(v);
    serde_json::to_vec(&canonical).expect("canonicalize produces valid JSON")
}

fn canonicalize(v: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match v {
        Value::Object(map) => {
            let mut sorted: Vec<(&String, &Value)> = map.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(b.0));
            let mut out = serde_json::Map::with_capacity(sorted.len());
            for (k, val) in sorted {
                out.insert(k.clone(), canonicalize(val));
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(canonicalize).collect()),
        _ => v.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_app_templates::{
        catalogue,
        class::{MAYFLY, SDDC_AUCTION, SINGH_SABI},
    };
    use serde_json::json;

    fn sabi_request() -> DeployRequest {
        DeployRequest::new(
            SINGH_SABI,
            json!({"initial_energy": 1000, "floor_pct": 15, "half_life": 365}),
            [0xAB; 32],
            1000,
            42,
        )
        .unwrap()
    }

    #[test]
    fn materialises_valid_request() {
        let req = sabi_request();
        let instr = materialise_request(&req).unwrap();
        assert_eq!(instr.template_class, SINGH_SABI);
        assert_eq!(
            instr.instance_id,
            derive_instance_id(SINGH_SABI, &[0xAB; 32], 42)
        );
        assert!(!instr.init_calldata.is_empty());
    }

    #[test]
    fn reject_unknown_template() {
        // Build a request bypassing the deploy validator (use a class
        // that's in-range-but-unregistered).
        let unknown = TemplateClass(0x0001_0FFF);
        let req = DeployRequest::new(unknown, json!({}), [0; 32], 0, 0).unwrap();
        let err = materialise_request(&req).unwrap_err();
        assert_eq!(err, MaterialiseError::UnknownTemplate(unknown.0));
    }

    #[test]
    fn reject_schema_invalid_request() {
        // Sabi missing floor_pct.
        let req = DeployRequest::new(
            SINGH_SABI,
            json!({"initial_energy": 1000, "half_life": 365}),
            [0; 32],
            0,
            0,
        )
        .unwrap();
        let err = materialise_request(&req).unwrap_err();
        assert!(matches!(err, MaterialiseError::SchemaInvalid(_)));
    }

    #[test]
    fn calldata_is_canonical_across_insertion_orders() {
        // Two requests with the same logical params but different
        // insertion orders produce identical init_calldata. This is
        // what lets two validators agree on the bytes that go into
        // the contract engine.
        let r1 = DeployRequest::new(
            SINGH_SABI,
            json!({"initial_energy": 1000, "floor_pct": 15, "half_life": 365}),
            [0; 32],
            0,
            0,
        )
        .unwrap();
        let r2 = DeployRequest::new(
            SINGH_SABI,
            json!({"half_life": 365, "floor_pct": 15, "initial_energy": 1000}),
            [0; 32],
            0,
            0,
        )
        .unwrap();
        let i1 = materialise_request(&r1).unwrap();
        let i2 = materialise_request(&r2).unwrap();
        assert_eq!(i1.init_calldata, i2.init_calldata);
        assert_eq!(i1.instance_id, i2.instance_id);
    }

    #[test]
    fn materialise_is_deterministic() {
        // Validator-determinism: same request → byte-identical
        // instruction across calls.
        let req = sabi_request();
        let a = materialise_request(&req).unwrap();
        let b = materialise_request(&req).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn different_classes_produce_different_instances() {
        // Same params, same deployer/nonce, different classes — the
        // instance ids must differ. Anti-replay across templates.
        // Use a shape that satisfies both Sabi and Mayfly (Sabi is
        // stricter; superset of Mayfly's required keys).
        let p = json!({"initial_energy": 1000, "floor_pct": 15, "half_life": 100});
        let r1 = DeployRequest::new(SINGH_SABI, p.clone(), [0; 32], 0, 0).unwrap();
        let r2 = DeployRequest::new(MAYFLY, p, [0; 32], 0, 0).unwrap();
        let i1 = materialise_request(&r1).unwrap();
        let i2 = materialise_request(&r2).unwrap();
        assert_ne!(i1.instance_id, i2.instance_id);
    }

    #[test]
    fn instance_id_independent_of_params() {
        // An instance id depends only on (class, deployer, nonce) —
        // see instance.rs structural decision #2. Two requests with
        // the same identity but different (still-valid) params
        // produce the SAME instance id; the params live in the
        // calldata.
        let r1 = DeployRequest::new(
            SINGH_SABI,
            json!({"initial_energy": 1000, "floor_pct": 15, "half_life": 365}),
            [0; 32],
            0,
            7,
        )
        .unwrap();
        let r2 = DeployRequest::new(
            SINGH_SABI,
            json!({"initial_energy": 9999, "floor_pct": 15, "half_life": 365}),
            [0; 32],
            0,
            7,
        )
        .unwrap();
        let i1 = materialise_request(&r1).unwrap();
        let i2 = materialise_request(&r2).unwrap();
        assert_eq!(i1.instance_id, i2.instance_id);
        assert_ne!(i1.init_calldata, i2.init_calldata);
    }

    #[test]
    fn every_catalogue_entry_materialises_from_its_default_params() {
        // Anti-regression: every catalogue entry's default_params
        // shape should be materialise-valid. If a future PR drops a
        // required key from defaults this catches it.
        for desc in catalogue() {
            let req = DeployRequest::new(
                desc.class,
                desc.default_params.clone(),
                [0xAB; 32],
                0,
                1,
            )
            .expect("default_params is a JSON object");
            let instr = materialise_request(&req).unwrap_or_else(|e| {
                panic!(
                    "catalogue entry {:#010x} ({}) failed materialise: {:?}",
                    desc.class.0, desc.brand, e
                );
            });
            assert_eq!(instr.template_class, desc.class);
        }
    }

    #[test]
    fn marketplace_lane_materialises() {
        // SDDC: full deploy → materialise round-trip.
        let req = DeployRequest::new(
            SDDC_AUCTION,
            json!({
                "ceiling": 1000,
                "floor": 100,
                "lot_lambda": 50,
                "duration_epochs": 1000
            }),
            [0xCC; 32],
            500,
            1,
        )
        .unwrap();
        let instr = materialise_request(&req).unwrap();
        assert_eq!(instr.template_class, SDDC_AUCTION);
        assert_eq!(
            instr.instance_id,
            derive_instance_id(SDDC_AUCTION, &[0xCC; 32], 1)
        );
    }

    #[test]
    fn round_trip_serde_instruction() {
        let instr = materialise_request(&sabi_request()).unwrap();
        let s = serde_json::to_string(&instr).unwrap();
        let back: MaterialiseInstruction = serde_json::from_str(&s).unwrap();
        assert_eq!(instr, back);
    }
}
