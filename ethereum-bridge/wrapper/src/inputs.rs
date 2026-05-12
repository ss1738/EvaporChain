//! Public-input encoding for the Groth16 wrapper circuit.
//!
//! The L1 verifier consumes 4 BN254 Fr elements as public inputs:
//!
//! | Index | Anchor               | Source                                  |
//! |------:|----------------------|-----------------------------------------|
//! |   0   | `state_root`         | EvaporHeaderInbox-anchored state root   |
//! |   1   | `key`                | Verkle path key                         |
//! |   2   | `value_commitment`   | keccak256(value) domain-bound           |
//! |   3   | `params_fingerprint` | blake3 fingerprint of Halo2 IPA params  |
//!
//! Each is delivered as a `bytes32` from the fixture and reduced mod r
//! (BN254 Fr order ≈ 0x3064...). The fixture emitter (sub-A-finish)
//! pre-masks anchors to `< r` so the reduction is a no-op for
//! well-formed fixtures — but [`decode_anchor`] is defensive and
//! handles any 32-byte input via canonical big-endian Fr::from_be_bytes_mod_order.

use ark_bn254::Fr;
use ark_ff::PrimeField;

/// The 4 public-input anchors the L1 Solidity verifier consumes. Order
/// is canonical and MUST match the IC[] indices baked into the Groth16
/// verifying key by the trusted-setup ceremony (sub-C).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrapperPublicInputs {
    pub state_root: Fr,
    pub key: Fr,
    pub value_commitment: Fr,
    pub params_fingerprint: Fr,
}

impl WrapperPublicInputs {
    /// Pack as a `Vec<Fr>` in the canonical order Groth16 expects.
    pub fn to_vec(&self) -> Vec<Fr> {
        vec![
            self.state_root,
            self.key,
            self.value_commitment,
            self.params_fingerprint,
        ]
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AnchorDecodeError {
    #[error("anchor hex must be 32 bytes (got {0})")]
    WrongLength(usize),
    #[error("hex decode failed: {0}")]
    Hex(#[from] hex::FromHexError),
}

/// Decode a `bytes32` anchor (with or without `0x` prefix) into a BN254
/// Fr element via canonical big-endian modular reduction.
///
/// Well-formed fixture anchors are pre-masked < r (see sub-A-finish's
/// `derive_bn254_anchor`), so the reduction is a no-op. Defensive
/// reduction here lets the wrapper accept any 32-byte bridge calldata
/// without breaking on top-byte-≥0x30 values.
pub fn decode_anchor(hex_str: &str) -> Result<Fr, AnchorDecodeError> {
    let stripped = hex_str.trim_start_matches("0x");
    let bytes = hex::decode(stripped)?;
    if bytes.len() != 32 {
        return Err(AnchorDecodeError::WrongLength(bytes.len()));
    }
    Ok(Fr::from_be_bytes_mod_order(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_anchor_handles_prefixed_and_unprefixed_hex() {
        let a = decode_anchor("0x01020304050607080102030405060708010203040506070801020304050607ff")
            .expect("prefixed must decode");
        let b = decode_anchor("01020304050607080102030405060708010203040506070801020304050607ff")
            .expect("unprefixed must decode");
        assert_eq!(a, b, "prefix must not change the decoded Fr");
    }

    #[test]
    fn decode_anchor_rejects_wrong_length() {
        match decode_anchor("0xdeadbeef") {
            Err(AnchorDecodeError::WrongLength(4)) => (),
            other => panic!("expected WrongLength(4), got {:?}", other),
        }
    }

    #[test]
    fn decode_anchor_masks_high_value_via_modular_reduction() {
        // Top byte 0xff is well above BN254 r (which starts 0x30).
        // Modular reduction must succeed and give a valid Fr element.
        let fr = decode_anchor(
            "0xff00000000000000000000000000000000000000000000000000000000000000",
        )
        .expect("decode must succeed via mod-order reduction");
        // Sanity: the Fr serialises back to a different value than its
        // raw input (because the input exceeded r and was reduced).
        let mut buf = Vec::new();
        use ark_serialize::CanonicalSerialize;
        fr.serialize_compressed(&mut buf).expect("serialize");
        assert_eq!(buf.len(), 32);
        assert_ne!(buf[31], 0xff, "Fr after reduction must differ from raw input");
    }

    #[test]
    fn public_inputs_canonical_order_is_state_root_key_value_fingerprint() {
        let inputs = WrapperPublicInputs {
            state_root: Fr::from(1u64),
            key: Fr::from(2u64),
            value_commitment: Fr::from(3u64),
            params_fingerprint: Fr::from(4u64),
        };
        let v = inputs.to_vec();
        assert_eq!(v.len(), 4);
        assert_eq!(v[0], Fr::from(1u64), "index 0 must be state_root");
        assert_eq!(v[1], Fr::from(2u64), "index 1 must be key");
        assert_eq!(v[2], Fr::from(3u64), "index 2 must be value_commitment");
        assert_eq!(v[3], Fr::from(4u64), "index 3 must be params_fingerprint");
    }
}
