//! Bridge between the node's block production loop and the oracle consensus layer.
//!
//! Each block round, validators submit oracle votes for tracked feeds.
//! When quorum is reached, finalized values are applied to on-chain oracle state
//! and included in the block's oracle state root.

use evaporchain_consensus::validator_set::ValidatorSet;
use evaporchain_crypto::signatures::BlsPublicKey;
use evaporchain_oracle::consensus::{FinalizedOracleValue, OracleConsensusRound, OracleVote};
use evaporchain_oracle::presets;
use evaporchain_oracle::state::{OracleInclusionProof, OracleState};
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
        let round =
            OracleConsensusRound::new(key, self.round_counter, self.quorum, self.twap_window);
        self.active_rounds.insert(key.to_string(), round);
        self.round_counter
    }

    /// Submit a signed oracle vote with an explicitly-supplied pubkey.
    ///
    /// The caller MUST have looked the pubkey up from a trusted validator
    /// set keyed on `vote.validator_id`. This is the fast path used by a
    /// validator submitting its own vote (its registered pubkey is already
    /// in hand). For votes received from other validators (e.g. over P2P),
    /// prefer [`submit_vote_via_validator_set`] which performs the lookup
    /// against `ValidatorSet` so the pubkey can never be supplied from the
    /// vote payload itself.
    pub fn submit_vote(
        &mut self,
        key: &str,
        vote: OracleVote,
        validator_pubkey: &BlsPublicKey,
    ) -> Result<(), String> {
        let round = self
            .active_rounds
            .get_mut(key)
            .ok_or_else(|| format!("no active round for key '{}'", key))?;
        round
            .submit_vote(vote, validator_pubkey)
            .map_err(|e| format!("{:?}", e))
    }

    /// Submit a vote and resolve the validator's BLS pubkey from the
    /// canonical `ValidatorSet`. Use this for votes received from peers —
    /// it eliminates the class of attack where an attacker forges a vote
    /// claiming any `validator_id` and supplies a matching attacker-owned
    /// pubkey, since the pubkey is taken from the validator set rather
    /// than the vote payload.
    ///
    /// Returns an error if `vote.validator_id` is unknown to the
    /// validator set or has no registered BLS public key (validator hasn't
    /// announced their key yet).
    pub fn submit_vote_via_validator_set(
        &mut self,
        key: &str,
        vote: OracleVote,
        validator_set: &ValidatorSet,
    ) -> Result<(), String> {
        let info = validator_set
            .get(vote.validator_id)
            .ok_or_else(|| format!("validator {} unknown to validator set", vote.validator_id))?;
        let pk_bytes = info.bls_public_key.as_ref().ok_or_else(|| {
            format!(
                "validator {} has no registered BLS public key",
                vote.validator_id
            )
        })?;
        let pk = BlsPublicKey(pk_bytes.clone());
        self.submit_vote(key, vote, &pk)
    }

    pub fn try_finalize(&mut self, key: &str) -> Option<FinalizedOracleValue> {
        let round = self.active_rounds.get_mut(key)?;
        if !round.has_quorum() {
            return None;
        }
        match round.finalize() {
            Ok(finalized) => {
                let preset = match key {
                    k if k.contains("usd") => {
                        (presets::PRICE_FEED.energy, presets::PRICE_FEED.half_life)
                    }
                    k if k.contains("weather") => {
                        (presets::WEATHER.energy, presets::WEATHER.half_life)
                    }
                    k if k.contains("earthquake") => {
                        (presets::EARTHQUAKE.energy, presets::EARTHQUAKE.half_life)
                    }
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

    /// Audit O1 (2026-05-15): expose last_updated so the API can include
    /// age metadata in responses, letting callers detect stale data.
    pub fn get_last_updated(&self, key: &str) -> Option<u64> {
        self.state.get(key).map(|e| e.last_updated)
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
    use evaporchain_crypto::signatures::BlsKeypair;
    use evaporchain_oracle::consensus::make_vote;

    fn signed(
        kp: &BlsKeypair,
        validator_id: u64,
        key: &str,
        value: f64,
        round: u64,
        ts: u64,
    ) -> (OracleVote, BlsPublicKey) {
        let mut v = make_vote(validator_id, key, value, round, ts);
        v.sign(kp);
        (v, kp.public_key_bytes())
    }

    #[test]
    fn test_oracle_bridge_full_cycle() {
        let mut bridge = OracleBridge::new(2);
        let round = bridge.start_round("btc_usd");

        let kp0 = BlsKeypair::generate();
        let kp1 = BlsKeypair::generate();
        let (v0, pk0) = signed(&kp0, 0, "btc_usd", 60000.0, round, 1000);
        bridge.submit_vote("btc_usd", v0, &pk0).unwrap();
        assert!(bridge.try_finalize("btc_usd").is_none());

        let (v1, pk1) = signed(&kp1, 1, "btc_usd", 60100.0, round, 1001);
        bridge.submit_vote("btc_usd", v1, &pk1).unwrap();
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

        let kp = BlsKeypair::generate();
        let (v_btc, pk_btc) = signed(&kp, 0, "btc_usd", 60000.0, r1, 1000);
        bridge.submit_vote("btc_usd", v_btc, &pk_btc).unwrap();
        let (v_eth, pk_eth) = signed(&kp, 0, "eth_usd", 3000.0, r2, 1000);
        bridge.submit_vote("eth_usd", v_eth, &pk_eth).unwrap();

        let results = bridge.finalize_all();
        assert_eq!(results.len(), 2);
        assert_eq!(bridge.feed_count(), 2);
    }

    #[test]
    fn test_oracle_inclusion_proof() {
        let mut bridge = OracleBridge::new(1);
        let r = bridge.start_round("btc_usd");
        let kp = BlsKeypair::generate();
        let (v, pk) = signed(&kp, 0, "btc_usd", 60000.0, r, 1000);
        bridge.submit_vote("btc_usd", v, &pk).unwrap();
        bridge.try_finalize("btc_usd");

        let proof = bridge.generate_proof("btc_usd").unwrap();
        let root = bridge.oracle_state_root();
        assert!(proof.verify(&root));
    }

    // ─── Validator-set lookup path ────────────────────────────────────────

    use evaporchain_consensus::validator_set::ValidatorInfo;

    fn validator_set_with(validators: &[(u64, &BlsKeypair)]) -> ValidatorSet {
        let infos: Vec<ValidatorInfo> = validators
            .iter()
            .map(|(id, kp)| {
                let mut addr = [0u8; 32];
                addr[0] = *id as u8;
                ValidatorInfo::with_bls_key(*id, 1000, addr, kp.public_key_bytes().0.clone())
            })
            .collect();
        ValidatorSet::with_validators(infos)
    }

    #[test]
    fn test_submit_via_validator_set_happy_path() {
        let mut bridge = OracleBridge::new(2);
        let round = bridge.start_round("btc_usd");
        let kp0 = BlsKeypair::generate();
        let kp1 = BlsKeypair::generate();
        let vs = validator_set_with(&[(0, &kp0), (1, &kp1)]);

        let (v0, _) = signed(&kp0, 0, "btc_usd", 60000.0, round, 1000);
        bridge
            .submit_vote_via_validator_set("btc_usd", v0, &vs)
            .unwrap();
        let (v1, _) = signed(&kp1, 1, "btc_usd", 60100.0, round, 1001);
        bridge
            .submit_vote_via_validator_set("btc_usd", v1, &vs)
            .unwrap();
        let finalized = bridge.try_finalize("btc_usd").unwrap();
        assert_eq!(finalized.voter_count, 2);
    }

    #[test]
    fn test_submit_via_validator_set_unknown_validator_rejected() {
        let mut bridge = OracleBridge::new(1);
        let _ = bridge.start_round("btc_usd");
        let kp_real = BlsKeypair::generate();
        let vs = validator_set_with(&[(0, &kp_real)]);

        let kp_attacker = BlsKeypair::generate();
        // validator_id=99 is not in the validator set.
        let (v, _) = signed(&kp_attacker, 99, "btc_usd", 60000.0, 1, 1000);
        let err = bridge
            .submit_vote_via_validator_set("btc_usd", v, &vs)
            .unwrap_err();
        assert!(err.contains("unknown to validator set"), "{err}");
    }

    #[test]
    fn test_submit_via_validator_set_no_registered_pubkey_rejected() {
        let mut bridge = OracleBridge::new(1);
        let _ = bridge.start_round("btc_usd");
        // Validator 0 exists in the set but never registered a BLS pubkey.
        let info = ValidatorInfo::new(0, 1000, [0u8; 32]);
        let vs = ValidatorSet::with_validators(vec![info]);

        let kp = BlsKeypair::generate();
        let (v, _) = signed(&kp, 0, "btc_usd", 60000.0, 1, 1000);
        let err = bridge
            .submit_vote_via_validator_set("btc_usd", v, &vs)
            .unwrap_err();
        assert!(err.contains("no registered BLS public key"), "{err}");
    }

    #[test]
    fn test_submit_via_validator_set_forged_id_rejected() {
        // Attacker forges a vote claiming validator_id=0 but signs it with
        // their own key. The lookup pulls the real validator-0 pubkey
        // from the set, signature verification must fail.
        let mut bridge = OracleBridge::new(1);
        let _ = bridge.start_round("btc_usd");
        let real_kp = BlsKeypair::generate();
        let attacker_kp = BlsKeypair::generate();
        let vs = validator_set_with(&[(0, &real_kp)]);

        let (v, _) = signed(&attacker_kp, 0, "btc_usd", 60000.0, 1, 1000);
        let err = bridge
            .submit_vote_via_validator_set("btc_usd", v, &vs)
            .unwrap_err();
        // Bubbles up from BlsVerifier::verify failing.
        assert!(err.contains("verification failed"), "{err}");
    }
}
