use crate::db::StateDB;
use evaporchain_types::{DecayRate, Epoch};

/// Engine that processes energy decay and object evaporation each epoch.
pub struct EvaporationEngine {
    pub decay_rate: DecayRate,
    pub grace_period: u64,
}

impl EvaporationEngine {
    /// Create a new evaporation engine.
    pub fn new(decay_rate: DecayRate, grace_period: u64) -> Self {
        Self {
            decay_rate,
            grace_period,
        }
    }

    /// Process all objects for the given epoch, applying decay and evaporation rules.
    pub fn process_epoch(&self, _db: &mut dyn StateDB, _current_epoch: Epoch) {
        todo!("Evaporation epoch processing not yet implemented")
    }
}
