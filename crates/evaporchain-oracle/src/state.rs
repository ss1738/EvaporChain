//! Oracle state management — tracks finalized oracle values on-chain,
//! manages energy decay for oracle objects, and produces inclusion proofs
//! for block headers.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

use super::consensus::FinalizedOracleValue;

// ─── On-Chain Oracle State ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleState {
    entries: HashMap<String, OracleEntry>,
    history_depth: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleEntry {
    pub key: String,
    pub current_value: f64,
    pub current_twap: Option<f64>,
    pub last_round: u64,
    pub last_updated: u64,
    pub update_count: u64,
    pub initial_energy: u64,
    pub half_life: u64,
    pub history: Vec<HistoryPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryPoint {
    pub value: f64,
    pub round: u64,
    pub timestamp: u64,
}

impl Default for OracleState {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            history_depth: 100,
        }
    }
}

impl OracleState {
    pub fn new(history_depth: usize) -> Self {
        Self {
            entries: HashMap::new(),
            history_depth,
        }
    }

    pub fn apply_finalized(&mut self, finalized: &FinalizedOracleValue, energy: u64, half_life: u64) {
        let entry = self.entries.entry(finalized.key.clone()).or_insert_with(|| {
            OracleEntry {
                key: finalized.key.clone(),
                current_value: 0.0,
                current_twap: None,
                last_round: 0,
                last_updated: 0,
                update_count: 0,
                initial_energy: energy,
                half_life,
                history: Vec::new(),
            }
        });

        entry.current_value = finalized.value;
        entry.current_twap = finalized.twap;
        entry.last_round = finalized.round;
        entry.last_updated = finalized.timestamp;
        entry.update_count += 1;
        entry.initial_energy = energy;

        entry.history.push(HistoryPoint {
            value: finalized.value,
            round: finalized.round,
            timestamp: finalized.timestamp,
        });
        if entry.history.len() > self.history_depth {
            let excess = entry.history.len() - self.history_depth;
            entry.history.drain(..excess);
        }
    }

    pub fn get(&self, key: &str) -> Option<&OracleEntry> {
        self.entries.get(key)
    }

    pub fn get_value(&self, key: &str) -> Option<f64> {
        self.entries.get(key).map(|e| e.current_value)
    }

    pub fn get_twap(&self, key: &str) -> Option<f64> {
        self.entries.get(key).and_then(|e| e.current_twap)
    }

    /// NOTE (M-06): Clones all keys into a Vec. Acceptable because this is only
    /// called in tests and the oracle key set is small (tens of price feeds, not
    /// thousands). If the oracle grows to high cardinality, return an iterator.
    pub fn keys(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn remove(&mut self, key: &str) -> Option<OracleEntry> {
        self.entries.remove(key)
    }

    pub fn state_root(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        let mut keys: Vec<&String> = self.entries.keys().collect();
        keys.sort();
        for key in keys {
            if let Some(entry) = self.entries.get(key) {
                hasher.update(key.as_bytes());
                hasher.update(&entry.current_value.to_le_bytes());
                hasher.update(&entry.last_round.to_le_bytes());
                hasher.update(&entry.last_updated.to_le_bytes());
            }
        }
        hasher.finalize().into()
    }

    pub fn energy_for_key(&self, key: &str, current_epoch: u64, creation_epoch: u64) -> u64 {
        if let Some(entry) = self.entries.get(key) {
            let elapsed = current_epoch.saturating_sub(creation_epoch);
            if entry.half_life == 0 {
                return 0;
            }
            let decay_factor = 0.5f64.powf(elapsed as f64 / entry.half_life as f64);
            (entry.initial_energy as f64 * decay_factor) as u64
        } else {
            0
        }
    }
}

// ─── Oracle Inclusion Proof ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleInclusionProof {
    pub key: String,
    pub value: f64,
    pub round: u64,
    pub state_root: [u8; 32],
    pub proof_hash: [u8; 32],
}

impl OracleInclusionProof {
    pub fn generate(state: &OracleState, key: &str) -> Option<Self> {
        let entry = state.get(key)?;
        let state_root = state.state_root();

        let mut hasher = Sha256::new();
        hasher.update(b"oracle-inclusion-v1:");
        hasher.update(key.as_bytes());
        hasher.update(&entry.current_value.to_le_bytes());
        hasher.update(&entry.last_round.to_le_bytes());
        hasher.update(&state_root);
        let proof_hash = hasher.finalize().into();

        Some(Self {
            key: key.to_string(),
            value: entry.current_value,
            round: entry.last_round,
            state_root,
            proof_hash,
        })
    }

    pub fn verify(&self, expected_state_root: &[u8; 32]) -> bool {
        if self.state_root != *expected_state_root {
            return false;
        }
        let mut hasher = Sha256::new();
        hasher.update(b"oracle-inclusion-v1:");
        hasher.update(self.key.as_bytes());
        hasher.update(&self.value.to_le_bytes());
        hasher.update(&self.round.to_le_bytes());
        hasher.update(&self.state_root);
        let expected_hash: [u8; 32] = hasher.finalize().into();
        self.proof_hash == expected_hash
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::FinalizedOracleValue;

    fn make_finalized(key: &str, value: f64, round: u64, ts: u64) -> FinalizedOracleValue {
        FinalizedOracleValue {
            key: key.to_string(),
            value,
            round,
            timestamp: ts,
            voter_count: 3,
            aggregate_hash: [0u8; 32],
            twap: Some(value),
        }
    }

    #[test]
    fn test_apply_and_get() {
        let mut state = OracleState::default();
        let f = make_finalized("btc_usd", 60000.0, 1, 1000);
        state.apply_finalized(&f, 5000, 300);
        assert_eq!(state.get_value("btc_usd"), Some(60000.0));
        assert_eq!(state.len(), 1);
    }

    #[test]
    fn test_update_overwrites() {
        let mut state = OracleState::default();
        state.apply_finalized(&make_finalized("btc_usd", 60000.0, 1, 1000), 5000, 300);
        state.apply_finalized(&make_finalized("btc_usd", 61000.0, 2, 1060), 5000, 300);
        assert_eq!(state.get_value("btc_usd"), Some(61000.0));
        let entry = state.get("btc_usd").unwrap();
        assert_eq!(entry.update_count, 2);
        assert_eq!(entry.last_round, 2);
    }

    #[test]
    fn test_history_tracking() {
        let mut state = OracleState::new(3);
        for i in 0..5 {
            let f = make_finalized("btc_usd", 60000.0 + i as f64 * 100.0, i, 1000 + i * 60);
            state.apply_finalized(&f, 5000, 300);
        }
        let entry = state.get("btc_usd").unwrap();
        assert_eq!(entry.history.len(), 3);
        assert_eq!(entry.history[0].round, 2);
    }

    #[test]
    fn test_state_root_deterministic() {
        let mut s1 = OracleState::default();
        s1.apply_finalized(&make_finalized("btc_usd", 60000.0, 1, 1000), 5000, 300);
        s1.apply_finalized(&make_finalized("eth_usd", 3000.0, 1, 1000), 3000, 300);

        let mut s2 = OracleState::default();
        s2.apply_finalized(&make_finalized("eth_usd", 3000.0, 1, 1000), 3000, 300);
        s2.apply_finalized(&make_finalized("btc_usd", 60000.0, 1, 1000), 5000, 300);

        assert_eq!(s1.state_root(), s2.state_root());
    }

    #[test]
    fn test_state_root_changes_on_update() {
        let mut state = OracleState::default();
        state.apply_finalized(&make_finalized("btc_usd", 60000.0, 1, 1000), 5000, 300);
        let root1 = state.state_root();
        state.apply_finalized(&make_finalized("btc_usd", 61000.0, 2, 1060), 5000, 300);
        let root2 = state.state_root();
        assert_ne!(root1, root2);
    }

    #[test]
    fn test_energy_decay() {
        let mut state = OracleState::default();
        state.apply_finalized(&make_finalized("btc_usd", 60000.0, 1, 1000), 10000, 100);
        let e0 = state.energy_for_key("btc_usd", 0, 0);
        assert_eq!(e0, 10000);
        let e100 = state.energy_for_key("btc_usd", 100, 0);
        assert_eq!(e100, 5000);
        let e200 = state.energy_for_key("btc_usd", 200, 0);
        assert_eq!(e200, 2500);
    }

    #[test]
    fn test_inclusion_proof_generate_and_verify() {
        let mut state = OracleState::default();
        state.apply_finalized(&make_finalized("btc_usd", 60000.0, 1, 1000), 5000, 300);
        let root = state.state_root();
        let proof = OracleInclusionProof::generate(&state, "btc_usd").unwrap();
        assert!(proof.verify(&root));
    }

    #[test]
    fn test_inclusion_proof_fails_wrong_root() {
        let mut state = OracleState::default();
        state.apply_finalized(&make_finalized("btc_usd", 60000.0, 1, 1000), 5000, 300);
        let proof = OracleInclusionProof::generate(&state, "btc_usd").unwrap();
        assert!(!proof.verify(&[0xAB; 32]));
    }

    #[test]
    fn test_missing_key_returns_none() {
        let state = OracleState::default();
        assert!(state.get_value("nonexistent").is_none());
        assert!(OracleInclusionProof::generate(&state, "nonexistent").is_none());
    }

    #[test]
    fn test_remove_entry() {
        let mut state = OracleState::default();
        state.apply_finalized(&make_finalized("btc_usd", 60000.0, 1, 1000), 5000, 300);
        assert_eq!(state.len(), 1);
        state.remove("btc_usd");
        assert_eq!(state.len(), 0);
    }

    #[test]
    fn test_twap_stored() {
        let mut state = OracleState::default();
        let f = make_finalized("btc_usd", 60000.0, 1, 1000);
        state.apply_finalized(&f, 5000, 300);
        assert_eq!(state.get_twap("btc_usd"), Some(60000.0));
    }

    #[test]
    fn test_multiple_keys() {
        let mut state = OracleState::default();
        state.apply_finalized(&make_finalized("btc_usd", 60000.0, 1, 1000), 5000, 300);
        state.apply_finalized(&make_finalized("eth_usd", 3000.0, 1, 1000), 3000, 300);
        state.apply_finalized(&make_finalized("sol_usd", 150.0, 1, 1000), 2000, 300);
        assert_eq!(state.len(), 3);
        let mut keys = state.keys();
        keys.sort();
        assert_eq!(keys, vec!["btc_usd", "eth_usd", "sol_usd"]);
    }
}
