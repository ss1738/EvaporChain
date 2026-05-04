//! `validate_against_descriptor` — schema check before a deploy
//! request is allowed onto the chain's contract layer.
//!
//! The chain enforces *required-keys present*, not *exact-shape
//! match*. Two reasons:
//!
//! 1. Forward-compat. A V2 catalogue entry adding an optional param
//!    must not break V1 clients submitting deploy requests with the
//!    older shape — any unknown key in `params` is permitted.
//! 2. Type-flexibility. JSON numbers decode loosely; bytestrings
//!    might be hex or base64; this validator does not interpret
//!    semantic meaning of keys, just presence. Each contract's
//!    materialiser does the typed parse.
//!
//! What this validator does NOT check:
//! - Value types (e.g., `initial_energy` must be a positive integer).
//!   That is the materialiser's responsibility — the contract VM
//!   knows the typed shape, this generic layer does not.
//! - Cross-key invariants (e.g., `floor < ceiling` in SDDC). Same
//!   reason — domain logic lives in the materialiser.
//! - Signature validity. The transaction layer does that, after
//!   computing the commitment from the request.

use thiserror::Error;

use evaporchain_app_templates::TemplateDescriptor;

use crate::request::DeployRequest;
use crate::required_keys::required_keys_for;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error(
        "request class {req_class:#010x} does not match descriptor class {desc_class:#010x}"
    )]
    ClassMismatch { req_class: u32, desc_class: u32 },
    #[error("request params is not a JSON object (validator should have rejected at construction)")]
    ParamsNotObject,
    #[error("required parameter key '{0}' is missing from request params")]
    MissingRequiredKey(&'static str),
    #[error(
        "no required-keys row for class {0:#010x} — catalogue and required_keys table are out of sync"
    )]
    NoRequiredKeysRow(u32),
}

/// Check a [`DeployRequest`] against the descriptor it claims to
/// instantiate. Returns `Ok(())` iff the request is structurally
/// suitable for deploy.
pub fn validate_against_descriptor(
    req: &DeployRequest,
    descriptor: &TemplateDescriptor,
) -> Result<(), ValidationError> {
    if req.template_class != descriptor.class {
        return Err(ValidationError::ClassMismatch {
            req_class: req.template_class.0,
            desc_class: descriptor.class.0,
        });
    }

    let obj = req
        .params
        .as_object()
        .ok_or(ValidationError::ParamsNotObject)?;

    let required = required_keys_for(req.template_class);
    if required.is_empty() {
        // Every registered template has at least one required key
        // (paradigm primitives have `fragment`). An empty row for a
        // class that *is* registered means the catalogue and the
        // required-keys table drifted — fail closed.
        return Err(ValidationError::NoRequiredKeysRow(req.template_class.0));
    }

    for key in required {
        if !obj.contains_key(*key) {
            return Err(ValidationError::MissingRequiredKey(key));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_app_templates::{
        catalogue,
        class::{MAYFLY, SDDC_AUCTION, SINGH_SABI},
        find,
    };
    use serde_json::json;

    fn sabi_request() -> DeployRequest {
        DeployRequest::new(
            SINGH_SABI,
            json!({"initial_energy": 1000, "floor_pct": 15, "half_life": 365}),
            [0; 32],
            0,
            0,
        )
        .unwrap()
    }

    #[test]
    fn valid_request_passes() {
        let req = sabi_request();
        let desc = find(SINGH_SABI).unwrap();
        validate_against_descriptor(&req, &desc).unwrap();
    }

    #[test]
    fn class_mismatch_is_rejected() {
        let req = sabi_request();
        // Try to validate Sabi request against Mayfly descriptor.
        let mayfly_desc = find(MAYFLY).unwrap();
        let err = validate_against_descriptor(&req, &mayfly_desc).unwrap_err();
        assert!(matches!(err, ValidationError::ClassMismatch { .. }));
    }

    #[test]
    fn missing_required_key_is_rejected() {
        // Build a Sabi request that's missing `floor_pct`.
        let req = DeployRequest::new(
            SINGH_SABI,
            json!({"initial_energy": 1000, "half_life": 365}),
            [0; 32],
            0,
            0,
        )
        .unwrap();
        let desc = find(SINGH_SABI).unwrap();
        let err = validate_against_descriptor(&req, &desc).unwrap_err();
        assert_eq!(err, ValidationError::MissingRequiredKey("floor_pct"));
    }

    #[test]
    fn extra_unknown_keys_are_permitted() {
        // Forward-compat: a V2 client adding an unknown param must
        // not be rejected by V1 validation.
        let req = DeployRequest::new(
            SINGH_SABI,
            json!({
                "initial_energy": 1000,
                "floor_pct": 15,
                "half_life": 365,
                "future_v2_only_field": "ignore_me"
            }),
            [0; 32],
            0,
            0,
        )
        .unwrap();
        let desc = find(SINGH_SABI).unwrap();
        validate_against_descriptor(&req, &desc).unwrap();
    }

    #[test]
    fn every_catalogue_entry_validates_against_its_default_params() {
        // Anti-regression: each descriptor's `default_params` must
        // satisfy its own required-keys row. If a V2 PR moves a key
        // from required → optional in `required_keys.rs` but forgets
        // to also drop it from `default_params`, this passes; the
        // dangerous direction is the other way (required key not in
        // defaults), which this catches.
        for desc in catalogue() {
            let req = DeployRequest::new(
                desc.class,
                desc.default_params.clone(),
                [0; 32],
                0,
                0,
            )
            .expect("default_params is a JSON object");
            validate_against_descriptor(&req, &desc).unwrap_or_else(|e| {
                panic!(
                    "catalogue entry {:#010x} ({}) failed self-validation: {:?}",
                    desc.class.0, desc.brand, e
                );
            });
        }
    }

    #[test]
    fn nft_lane_required_keys_enforced() {
        // Spot-check: a Sabi request with all keys but `half_life`
        // is rejected.
        let req = DeployRequest::new(
            SINGH_SABI,
            json!({"initial_energy": 1000, "floor_pct": 15}),
            [0; 32],
            0,
            0,
        )
        .unwrap();
        let desc = find(SINGH_SABI).unwrap();
        let err = validate_against_descriptor(&req, &desc).unwrap_err();
        assert_eq!(err, ValidationError::MissingRequiredKey("half_life"));
    }

    #[test]
    fn marketplace_lane_required_keys_enforced() {
        // SDDC needs ceiling/floor/lot_lambda/duration_epochs.
        let req = DeployRequest::new(
            SDDC_AUCTION,
            json!({"ceiling": 1000, "floor": 100, "lot_lambda": 50}),
            [0; 32],
            0,
            0,
        )
        .unwrap();
        let desc = find(SDDC_AUCTION).unwrap();
        let err = validate_against_descriptor(&req, &desc).unwrap_err();
        assert_eq!(err, ValidationError::MissingRequiredKey("duration_epochs"));
    }
}
