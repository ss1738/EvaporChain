//! `DeployRequest` — the structured payload the chain's contract
//! layer materialises into an actual on-chain instance.
//!
//! Five structural decisions:
//!
//! 1. **Domain-separation tag** in the signing bytes. A deploy
//!    signature can never be replayed for any other purpose
//!    (transfer, parameter vote, oracle attestation). Tag is
//!    `b"evaporchain:deploy:v1\0"`.
//! 2. **`template_class` is part of the signing bytes.** Two requests
//!    with identical params for *different* templates must hash to
//!    different bytes — otherwise a Mayfly mint signature would
//!    materialise a Posthuma testament with the same params.
//! 3. **Canonical JSON for params.** Validators agreeing on the
//!    request hash means agreeing on the params encoding;
//!    `serde_json::to_vec` is *not* canonical (key order is
//!    insertion-order, not sorted). We sort keys at every nesting
//!    level so two requests built from different code paths still
//!    hash identically.
//! 4. **`(deployer, nonce)` replay protection.** The chain's
//!    transaction layer combined with `nonce` makes the same deploy
//!    request submittable exactly once. `submitted_at_epoch` is the
//!    epoch the request *intends* to take effect — not when it was
//!    physically signed; that lets a deployer pre-sign a request and
//!    have a relayer submit it later.
//! 5. **Length-prefixed params.** A naive `tag || class || deployer ||
//!    epoch || nonce || params_json` is ambiguous: extending the
//!    params JSON is indistinguishable from extending the trailing
//!    field if the trailing field is variable-length. We u32-LE
//!    length-prefix the canonical params to make extension forgery
//!    structurally impossible.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use evaporchain_app_templates::TemplateClass;

/// Domain-separation tag for the deploy-request signing payload.
/// Bumping the `v1` suffix is the protocol-level upgrade switch.
pub const DEPLOY_DOMAIN_TAG: &[u8] = b"evaporchain:deploy:v1\0";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RequestError {
    #[error("class id {0:#010x} is outside the app-template range")]
    OutOfRange(u32),
    #[error("params must be a JSON object")]
    ParamsNotObject,
    #[error("params canonical encoding exceeds u32 max bytes")]
    ParamsTooLarge,
    /// SUB-N10 (audit 2026-05-15): canonical-JSON-serialised params
    /// exceed the construction cap. `signing_bytes` already u32-caps,
    /// but constructing the DeployRequest with a 100 MB nested-JSON
    /// `params` blob would force the canonical encoder to walk the
    /// whole tree (and a downstream MaterialiseInstruction would
    /// carry it) before that ceiling is reached. Cap up front in
    /// `new()` to fail fast.
    #[error("params canonical encoding length {got} exceeds cap {cap}")]
    ParamsExceedsConstructionCap { got: usize, cap: usize },
}

/// SUB-N10: construction-time cap on the canonical-JSON-encoded
/// params payload. Matches `MAX_INIT_CALLDATA` (dispatch-level) at
/// 64 KiB — generous for every legitimate template, tight enough
/// that pathological inputs fail at request-build time.
pub const MAX_DEPLOY_PARAMS_BYTES: usize = 64 * 1024;

/// One deploy request — a fully-specified intention to materialise
/// `template_class` with `params` on behalf of `deployer`.
///
/// Built by an off-chain dApp; consumed by the chain's contract
/// layer; signed by the deployer; verified by validators.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployRequest {
    pub template_class: TemplateClass,
    pub params: Value,
    pub deployer: [u8; 32],
    pub submitted_at_epoch: u64,
    pub nonce: u64,
}

impl DeployRequest {
    /// Construct a request, validating shape (range + object-shaped
    /// params). Use [`Self::commitment`] to produce a hash-stable
    /// id, [`Self::signing_bytes`] to produce the bytes a signer
    /// signs over.
    pub fn new(
        template_class: TemplateClass,
        params: Value,
        deployer: [u8; 32],
        submitted_at_epoch: u64,
        nonce: u64,
    ) -> Result<Self, RequestError> {
        if !template_class.is_in_app_range() {
            return Err(RequestError::OutOfRange(template_class.0));
        }
        if !params.is_object() {
            return Err(RequestError::ParamsNotObject);
        }
        // SUB-N10: pre-flight canonical-encoded length cap.
        let encoded_len = canonical_json_bytes(&params).len();
        if encoded_len > MAX_DEPLOY_PARAMS_BYTES {
            return Err(RequestError::ParamsExceedsConstructionCap {
                got: encoded_len,
                cap: MAX_DEPLOY_PARAMS_BYTES,
            });
        }
        Ok(Self {
            template_class,
            params,
            deployer,
            submitted_at_epoch,
            nonce,
        })
    }

    /// Canonical signing bytes — what the deployer's signature is
    /// computed over. Layout:
    ///
    /// ```text
    /// b"evaporchain:deploy:v1\0"   (22 B, fixed)
    /// template_class               (u32 LE, 4 B)
    /// deployer                     (32 B)
    /// submitted_at_epoch           (u64 LE, 8 B)
    /// nonce                        (u64 LE, 8 B)
    /// canonical_params_len         (u32 LE, 4 B)
    /// canonical_params_bytes       (variable)
    /// ```
    ///
    /// Validator-deterministic: the canonical JSON sorts object keys
    /// at every nesting level, so two runtimes produce byte-identical
    /// output for the same request.
    pub fn signing_bytes(&self) -> Result<Vec<u8>, RequestError> {
        let canonical = canonical_json_bytes(&self.params);
        let len: u32 = canonical
            .len()
            .try_into()
            .map_err(|_| RequestError::ParamsTooLarge)?;

        let mut out =
            Vec::with_capacity(DEPLOY_DOMAIN_TAG.len() + 4 + 32 + 8 + 8 + 4 + canonical.len());
        out.extend_from_slice(DEPLOY_DOMAIN_TAG);
        out.extend_from_slice(&self.template_class.0.to_le_bytes());
        out.extend_from_slice(&self.deployer);
        out.extend_from_slice(&self.submitted_at_epoch.to_le_bytes());
        out.extend_from_slice(&self.nonce.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&canonical);
        Ok(out)
    }

    /// BLAKE3 commitment of the signing bytes — usable as the
    /// deploy-tx hash for indexers, mempools, dApps.
    pub fn commitment(&self) -> Result<[u8; 32], RequestError> {
        let bytes = self.signing_bytes()?;
        Ok(*blake3::hash(&bytes).as_bytes())
    }
}

/// Recursively encode a JSON value into canonical bytes — object
/// keys sorted lexicographically at every nesting level. The output
/// is valid JSON; we just normalize the order so two requests built
/// from different call sites produce identical bytes.
fn canonical_json_bytes(v: &Value) -> Vec<u8> {
    let canonical = canonicalize(v);
    serde_json::to_vec(&canonical).expect("canonicalize produces valid JSON")
}

fn canonicalize(v: &Value) -> Value {
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
    use evaporchain_app_templates::class::{MAYFLY, SINGH_SABI};
    use serde_json::json;

    fn req() -> DeployRequest {
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
    fn rejects_out_of_range_class() {
        let err =
            DeployRequest::new(TemplateClass(0x9999_9999), json!({}), [0; 32], 0, 0).unwrap_err();
        assert_eq!(err, RequestError::OutOfRange(0x9999_9999));
    }

    #[test]
    fn rejects_non_object_params() {
        let err = DeployRequest::new(MAYFLY, json!([1, 2, 3]), [0; 32], 0, 0).unwrap_err();
        assert_eq!(err, RequestError::ParamsNotObject);
        let err = DeployRequest::new(MAYFLY, json!("string"), [0; 32], 0, 0).unwrap_err();
        assert_eq!(err, RequestError::ParamsNotObject);
        let err = DeployRequest::new(MAYFLY, json!(null), [0; 32], 0, 0).unwrap_err();
        assert_eq!(err, RequestError::ParamsNotObject);
    }

    #[test]
    fn signing_bytes_starts_with_domain_tag() {
        let bytes = req().signing_bytes().unwrap();
        assert!(bytes.starts_with(DEPLOY_DOMAIN_TAG));
    }

    #[test]
    fn signing_bytes_are_deterministic() {
        // Validator-determinism: two builds produce identical bytes.
        let a = req().signing_bytes().unwrap();
        let b = req().signing_bytes().unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn signing_bytes_canonicalize_param_key_order() {
        // Same logical params, different insertion order — canonical
        // bytes must agree.
        let r1 = DeployRequest::new(
            SINGH_SABI,
            json!({"initial_energy": 1000, "floor_pct": 15, "half_life": 365}),
            [0xAB; 32],
            1000,
            42,
        )
        .unwrap();
        let r2 = DeployRequest::new(
            SINGH_SABI,
            json!({"half_life": 365, "floor_pct": 15, "initial_energy": 1000}),
            [0xAB; 32],
            1000,
            42,
        )
        .unwrap();
        assert_eq!(r1.signing_bytes().unwrap(), r2.signing_bytes().unwrap());
    }

    #[test]
    fn canonicalize_recursive_into_nested_objects() {
        // Nested object key ordering must also be normalized.
        let r1 = DeployRequest::new(
            SINGH_SABI,
            json!({"outer": {"a": 1, "b": 2, "c": 3}}),
            [0; 32],
            0,
            0,
        )
        .unwrap();
        let r2 = DeployRequest::new(
            SINGH_SABI,
            json!({"outer": {"c": 3, "b": 2, "a": 1}}),
            [0; 32],
            0,
            0,
        )
        .unwrap();
        assert_eq!(r1.signing_bytes().unwrap(), r2.signing_bytes().unwrap());
    }

    #[test]
    fn canonicalize_does_not_reorder_arrays() {
        // Arrays are positional; element order is meaningful and
        // must not be sorted.
        let r1 = DeployRequest::new(SINGH_SABI, json!({"v": [3, 1, 2]}), [0; 32], 0, 0).unwrap();
        let r2 = DeployRequest::new(SINGH_SABI, json!({"v": [1, 2, 3]}), [0; 32], 0, 0).unwrap();
        assert_ne!(r1.signing_bytes().unwrap(), r2.signing_bytes().unwrap());
    }

    #[test]
    fn different_classes_produce_different_signing_bytes() {
        // Same params, different class — bytes must differ. Anti-
        // replay across templates.
        let p = json!({"initial_energy": 1000, "half_life": 100});
        let r1 = DeployRequest::new(SINGH_SABI, p.clone(), [0; 32], 0, 0).unwrap();
        let r2 = DeployRequest::new(MAYFLY, p, [0; 32], 0, 0).unwrap();
        assert_ne!(r1.signing_bytes().unwrap(), r2.signing_bytes().unwrap());
    }

    #[test]
    fn different_nonces_produce_different_signing_bytes() {
        // Replay protection: same payload, different nonce.
        let mut a = req();
        a.nonce = 1;
        let mut b = req();
        b.nonce = 2;
        assert_ne!(a.signing_bytes().unwrap(), b.signing_bytes().unwrap());
    }

    #[test]
    fn different_deployers_produce_different_signing_bytes() {
        let mut a = req();
        a.deployer = [0xAA; 32];
        let mut b = req();
        b.deployer = [0xBB; 32];
        assert_ne!(a.signing_bytes().unwrap(), b.signing_bytes().unwrap());
    }

    #[test]
    fn commitment_is_blake3_of_signing_bytes() {
        let r = req();
        let bytes = r.signing_bytes().unwrap();
        let expected: [u8; 32] = *blake3::hash(&bytes).as_bytes();
        assert_eq!(r.commitment().unwrap(), expected);
    }

    #[test]
    fn commitment_is_deterministic() {
        // Two commits of the same request hash to the same id.
        assert_eq!(req().commitment().unwrap(), req().commitment().unwrap());
    }

    #[test]
    fn round_trip_serde() {
        let r = req();
        let s = serde_json::to_string(&r).unwrap();
        let back: DeployRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn signing_bytes_layout_is_documented_offsets() {
        // Anti-regression: lock in the byte offsets so future
        // refactors can't silently shuffle the layout.
        let r = req();
        let bytes = r.signing_bytes().unwrap();
        let tag_len = DEPLOY_DOMAIN_TAG.len();
        // Tag ends at tag_len.
        assert_eq!(&bytes[..tag_len], DEPLOY_DOMAIN_TAG);
        // Class id (u32 LE) follows.
        let class = u32::from_le_bytes(bytes[tag_len..tag_len + 4].try_into().unwrap());
        assert_eq!(class, SINGH_SABI.0);
        // Deployer (32 B) follows.
        assert_eq!(&bytes[tag_len + 4..tag_len + 36], &[0xAB; 32]);
        // Epoch (u64 LE) follows.
        let epoch = u64::from_le_bytes(bytes[tag_len + 36..tag_len + 44].try_into().unwrap());
        assert_eq!(epoch, 1000);
        // Nonce (u64 LE) follows.
        let nonce = u64::from_le_bytes(bytes[tag_len + 44..tag_len + 52].try_into().unwrap());
        assert_eq!(nonce, 42);
    }
}
