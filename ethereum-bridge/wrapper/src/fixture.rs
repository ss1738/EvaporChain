//! Fixture loader for the Halo2 IPA proof + 4 anchors emitted by
//! `verkle-fixture-emit` in the sister `ethereum-bridge/circuits/`
//! workspace.
//!
//! The schema is pinned cross-side:
//!   - Rust source: `ethereum-bridge/circuits/src/bin/fixture_emit.rs`
//!   - Solidity guard: `test_loadsSampleFixture_innerProofBlock_schema`
//!     in `ethereum-bridge/contracts/test/VerkleProofVerifier.t.sol`
//!
//! Any field name/shape change here MUST update both sides.

use crate::inputs::{decode_anchor, AnchorDecodeError, WrapperPublicInputs};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum FixtureLoadError {
    #[error("I/O reading fixture: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON parse: {0}")]
    Json(#[from] serde_json::Error),
    #[error("anchor decode: {0}")]
    Anchor(#[from] AnchorDecodeError),
    #[error("hex decode of proof bytes: {0}")]
    Hex(#[from] hex::FromHexError),
    #[error("inner verkle_proof_v2.k expected 11, got {0}")]
    UnsupportedK(u32),
    #[error("inner verkle_proof_v2._schema_version expected 1, got {0}")]
    UnsupportedSchemaVersion(u32),
}

#[derive(Debug, Clone, Deserialize)]
struct RawFixture {
    state_root: String,
    key: String,
    value_commitment: String,
    params_fingerprint: String,
    verkle_proof_v2: RawInner,
}

#[derive(Debug, Clone, Deserialize)]
struct RawInner {
    #[serde(rename = "_schema_version")]
    schema_version: u32,
    proof_bytes_hex: String,
    k: u32,
}

/// In-memory representation of a loaded fixture: the 4 BN254 Fr anchors
/// + the raw Halo2 IPA proof bytes that the wrapper circuit will (in
/// sub-B-finish) verify in-circuit.
#[derive(Debug, Clone)]
pub struct VerkleFixture {
    pub public_inputs: WrapperPublicInputs,
    pub halo2_ipa_proof_bytes: Vec<u8>,
    pub k: u32,
}

impl VerkleFixture {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, FixtureLoadError> {
        let raw = std::fs::read_to_string(path)?;
        Self::from_json_str(&raw)
    }

    pub fn from_json_str(json: &str) -> Result<Self, FixtureLoadError> {
        let raw: RawFixture = serde_json::from_str(json)?;

        if raw.verkle_proof_v2.schema_version != 1 {
            return Err(FixtureLoadError::UnsupportedSchemaVersion(
                raw.verkle_proof_v2.schema_version,
            ));
        }
        if raw.verkle_proof_v2.k != 11 {
            return Err(FixtureLoadError::UnsupportedK(raw.verkle_proof_v2.k));
        }

        let public_inputs = WrapperPublicInputs {
            state_root: decode_anchor(&raw.state_root)?,
            key: decode_anchor(&raw.key)?,
            value_commitment: decode_anchor(&raw.value_commitment)?,
            params_fingerprint: decode_anchor(&raw.params_fingerprint)?,
        };

        let halo2_ipa_proof_bytes = hex::decode(&raw.verkle_proof_v2.proof_bytes_hex)?;

        Ok(Self {
            public_inputs,
            halo2_ipa_proof_bytes,
            k: raw.verkle_proof_v2.k,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Schema guard: schema_version != 1 must be rejected. Bumping the
    /// schema is a deliberate cross-side break that must update both
    /// the emitter and this loader together.
    #[test]
    fn rejects_unsupported_schema_version() {
        let bad = r#"{
            "state_root": "0x0101010101010101010101010101010101010101010101010101010101010101",
            "key": "0x0202020202020202020202020202020202020202020202020202020202020202",
            "value_commitment": "0x0303030303030303030303030303030303030303030303030303030303030303",
            "params_fingerprint": "0x0404040404040404040404040404040404040404040404040404040404040404",
            "verkle_proof_v2": {
                "_schema_version": 2,
                "proof_bytes_hex": "abcd",
                "k": 11
            }
        }"#;
        match VerkleFixture::from_json_str(bad) {
            Err(FixtureLoadError::UnsupportedSchemaVersion(2)) => (),
            other => panic!("expected UnsupportedSchemaVersion(2), got {:?}", other),
        }
    }

    /// Circuit-shape guard: only k=11 is supported in this starter.
    /// Sub-B-finish may relax this if the Halo2 IPA verifier circuit
    /// is parameterised by k, but for now the trusted-setup ceremony
    /// (sub-C) will be tied to k=11.
    #[test]
    fn rejects_unsupported_k() {
        let bad = r#"{
            "state_root": "0x0101010101010101010101010101010101010101010101010101010101010101",
            "key": "0x0202020202020202020202020202020202020202020202020202020202020202",
            "value_commitment": "0x0303030303030303030303030303030303030303030303030303030303030303",
            "params_fingerprint": "0x0404040404040404040404040404040404040404040404040404040404040404",
            "verkle_proof_v2": {
                "_schema_version": 1,
                "proof_bytes_hex": "abcd",
                "k": 12
            }
        }"#;
        match VerkleFixture::from_json_str(bad) {
            Err(FixtureLoadError::UnsupportedK(12)) => (),
            other => panic!("expected UnsupportedK(12), got {:?}", other),
        }
    }

    /// Happy path — schema_version=1, k=11, all anchors decode.
    #[test]
    fn loads_well_formed_fixture() {
        let good = r#"{
            "state_root": "0x0101010101010101010101010101010101010101010101010101010101010101",
            "key": "0x0202020202020202020202020202020202020202020202020202020202020202",
            "value_commitment": "0x0303030303030303030303030303030303030303030303030303030303030303",
            "params_fingerprint": "0x0404040404040404040404040404040404040404040404040404040404040404",
            "verkle_proof_v2": {
                "_schema_version": 1,
                "proof_bytes_hex": "deadbeef",
                "k": 11
            }
        }"#;
        let fx = VerkleFixture::from_json_str(good).expect("must load");
        assert_eq!(fx.k, 11);
        assert_eq!(fx.halo2_ipa_proof_bytes, vec![0xde, 0xad, 0xbe, 0xef]);
        // All 4 anchors must be distinct (just a sanity check that we
        // didn't accidentally fan out the same value to all fields).
        let pi = &fx.public_inputs;
        assert_ne!(pi.state_root, pi.key);
        assert_ne!(pi.key, pi.value_commitment);
        assert_ne!(pi.value_commitment, pi.params_fingerprint);
    }
}
