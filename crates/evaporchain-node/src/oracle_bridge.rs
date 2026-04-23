//! Bridge between the node's block production loop and the oracle consensus layer.
//!
//! Each block round, validators submit oracle votes for tracked feeds.
//! When quorum is reached, finalized values are applied to on-chain oracle state
//! and included in the block's oracle state root.

use evaporchain_oracle::consensus::{FinalizedOracleValue, OracleConsensusRound, OracleVote, make_vote};
use evaporchain_oracle::state::{OracleInclusionProof, OracleState};
use evaporchain_oracle::presets;
use std::collections::HashMap;

pub struct OracleBridge {
    state: OracleState,
    active_rounds: HashMap<String, OracleConsensusRound>,
    round_counter: u64,
    quorum: usize,
    twap_window: u64,
}

impl OracleBridge {
    pub fn new(quorum: usize) -> Self {
        Self {
            state: OracleState::new(100),
            active_rounds: HashMap::new(),
            round_counter: 0,
            quorum,
            twap_window: 3600,
        }
    }

    pub fn start_round(&mut self, key: &str) -> u64 {
        self.round_counter += 1;
        let round = OracleConsensusRound::new(key, self.round_counter, self.quorum, self.twap_window);
        self.active_rounds.insert(key.to_string(), round);
        self.round_counter
    }

    pub fn submit_vote(&mut self, key: &str, vote: OracleVote) -> Result<(), String> {
        let round = self.active_rounds.get_mut(key)
            .ok_or_else(|| format!("no active round for key '{}'", key))?;
        round.submit_vote(vote).map_err(|e| format!("{:?}", e))
    }

    pub fn try_finalize(&mut self, key: &str) -> Option<FinalizedOracleValue> {
        let round = self.active_rounds.get_mut(key)?;
        if !round.has_quorum() {
            return None;
        }
        match round.finalize() {
            Ok(finalized) => {
                let preset = match key {
                    k if k.contains("usd") => (presets::PRICE_FEED.energy, presets::PRICE_FEED.half_life),
                    k if k.contains("weather") => (presets::WEATHER.energy, presets::WEATHER.half_life),
                    k if k.contains("earthquake") => (presets::EARTHQUAKE.energy, presets::EARTHQUAKE.half_life),
                    _ => (3000, 300),
                };
                self.state.apply_finalized(&finalized, preset.0, preset.1);
                self.active_rounds.remove(key);
                Some(finalized)
            }
            Err(_) => None,
        }
    }

    pub fn finalize_all(&mut self) -> Vec<FinalizedOracleValue> {
        let keys: Vec<String> = self.active_rounds.keys().cloned().collect();
        let mut finalized = Vec::new();
        for key in keys {
            if let Some(f) = self.try_finalize(&key) {
                finalized.push(f);
            }
        }
        finalized
    }

    pub fn oracle_state_root(&self) -> [u8; 32] {
        self.state.state_root()
    }

    pub fn get_value(&self, key: &str) -> Option<f64> {
        self.state.get_value(key)
    }

    pub fn get_twap(&self, key: &str) -> Option<f64> {
        self.state.get_twap(key)
    }

    pub fn generate_proof(&self, key: &str) -> Option<OracleInclusionProof> {
        OracleInclusionProof::generate(&self.state, key)
    }

    pub fn feed_count(&self) -> usize {
        self.state.len()
    }

    pub fn active_rounds_count(&self) -> usize {
        self.active_rounds.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_oracle::consensus::make_vote;

    #[test]
    fn test_oracle_bridge_full_cycle() {
        let mut bridge = OracleBridge::new(2);
        let round = bridge.start_round("btc_usd");

        bridge.submit_vote("btc_usd", make_vote(0, "btc_usd", 60000.0, round, 1000)).unwrap();
        assert!(bridge.try_finalize("btc_usd").is_none());

        bridge.submit_vote("btc_usd", make_vote(1, "btc_usd", 60100.0, round, 1001)).unwrap();
        let finalized = bridge.try_finalize("btc_usd").unwrap();

        assert_eq!(finalized.key, "btc_usd");
        assert!(finalized.value >= 60000.0 && finalized.value <= 60100.0);
        assert!(bridge.get_value("btc_usd").is_some());
        assert_ne!(bridge.oracle_state_root(), [0u8; 32]);
    }

    #[test]
    fn test_oracle_bridge_multiple_feeds() {
        let mut bridge = OracleBridge::new(1);
        let r1 = bridge.start_round("btc_usd");
        let r2 = bridge.start_round("eth_usd");

        bridge.submit_vote("btc_usd", make_vote(0, "btc_usd", 60000.0, r1, 1000)).unwrap();
        bridge.submit_vote("eth_usd", make_vote(0, "eth_usd", 3000.0, r2, 1000)).unwrap();

        let results = bridge.finalize_all();
        assert_eq!(results.len(), 2);
        assert_eq!(bridge.feed_count(), 2);
    }

    #[test]
    fn test_oracle_inclusion_proof() {
        let mut bridge = OracleBridge::new(1);
        let r = bridge.start_round("btc_usd");
        bridge.submit_vote("btc_usd", make_vote(0, "btc_usd", 60000.0, r, 1000)).unwrap();
        bridge.try_finalize("btc_usd");

        let proof = bridge.generate_proof("btc_usd").unwrap();
        let root = bridge.oracle_state_root();
        assert!(proof.verify(&root));
    }
}
