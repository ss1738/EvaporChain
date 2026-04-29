//! `sign` — produce a substrate-quality signature bound to the
//! current period.

use serde::{Deserialize, Serialize};

use crate::key::EgFssKey;

const SIGN_TAG: &[u8] = b"evaporchain-eg-fss-sign";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    pub period_index: u64,
    pub mac: [u8; 32],
}

/// Substrate signature: blake3(SIGN_TAG || key.key_material ||
/// period || message). Real FSS uses a one-way trapdoor — substrate
/// uses keyed hash as the stand-in.
pub fn sign(key: &EgFssKey, message: &[u8]) -> Signature {
    let mut h = blake3::Hasher::new();
    h.update(SIGN_TAG);
    h.update(&key.key_material);
    h.update(&key.period_index.to_le_bytes());
    h.update(message);
    Signature {
        period_index: key.period_index,
        mac: *h.finalize().as_bytes(),
    }
}
