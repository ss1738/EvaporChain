//! `Vote` — one validator's proposed target value for a parameter.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Vote {
    pub validator_id: u64,
    pub target: u64,
    pub observed_epoch: u64,
}

impl Vote {
    pub const fn new(validator_id: u64, target: u64, observed_epoch: u64) -> Self {
        Self { validator_id, target, observed_epoch }
    }
}
