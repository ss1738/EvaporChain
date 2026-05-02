//! Oracle consensus — validator-signed oracle reports with BLS-style
//! aggregation, TWAP computation, and outlier rejection.
//!
//! Each validator signs their oracle observation. Once a quorum of validators
//! agree on a value (within tolerance), the aggregated report is finalized
//! and can be included in a block.

use evaporchain_crypto::signatures::{BlsKeypair, BlsPublicKey, BlsSignature, BlsVerifier};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

use super::OracleValue;

// ─── Validator Vote ──────────────────────────────────────────────────────

/// Domain separation tag for oracle vote BLS signatures.
pub const ORACLE_VOTE_DST: &[u8] = b"evaporchain:oracle-vote:v1:";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OracleVote {
    pub validator_id: u64,
    pub key: String,
    pub value: OracleValue,
    pub timestamp: u64,
    pub round: u64,
    pub signature: Vec<u8>,
}

impl OracleVote {
    pub fn signable_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(ORACLE_VOTE_DST);
        buf.extend_from_slice(&self.validator_id.to_le_bytes());
        buf.extend_from_slice(self.key.as_bytes());
        buf.push(b':');
        match &self.value {
            OracleValue::Numeric(v) => buf.extend_from_slice(&v.to_le_bytes()),
            OracleValue::Text(s) => buf.extend_from_slice(s.as_bytes()),
            OracleValue::Json(s) => buf.extend_from_slice(s.as_bytes()),
        }
        buf.extend_from_slice(&self.timestamp.to_le_bytes());
        buf.extend_from_slice(&self.round.to_le_bytes());
        buf
    }

    pub fn vote_hash(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(self.signable_bytes());
        hasher.finalize().into()
    }

    /// Sign this vote in place with the validator's BLS keypair.
    ///
    /// Replaces any existing `signature`. The matching public key must be
    /// supplied to `OracleConsensusRound::submit_vote` so the verifier can
    /// authenticate the sender. Callers are responsible for ensuring the
    /// `validator_id` field matches the keypair's identity in the live
    /// validator set — this method does not perform that check.
    pub fn sign(&mut self, kp: &BlsKeypair) {
        let sig = kp.sign(&self.signable_bytes());
        self.signature = sig.0;
    }
}

// ─── Finalized Oracle Value ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FinalizedOracleValue {
    pub key: String,
    pub value: f64,
    pub round: u64,
    pub timestamp: u64,
    pub voter_count: usize,
    pub aggregate_hash: [u8; 32],
    pub twap: Option<f64>,
}

// ─── TWAP (Time-Weighted Average Price) ──────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct TwapAccumulator {
    entries: Vec<(u64, f64)>,
    window_secs: u64,
}

impl TwapAccumulator {
    pub fn new(window_secs: u64) -> Self {
        Self {
            entries: Vec::new(),
            window_secs,
        }
    }

    pub fn push(&mut self, timestamp: u64, value: f64) {
        self.entries.push((timestamp, value));
        self.entries.sort_by_key(|&(t, _)| t);
        if let Some(&(latest, _)) = self.entries.last() {
            let cutoff = latest.saturating_sub(self.window_secs);
            self.entries.retain(|&(t, _)| t >= cutoff);
        }
    }

    pub fn twap(&self) -> Option<f64> {
        // Require at least 2 entries with non-zero total duration.
        // A single-entry TWAP is trivially manipulable in one block;
        // returning None forces callers to wait for real history.
        if self.entries.len() < 2 {
            return None;
        }
        let mut weighted_sum = 0.0f64;
        let mut total_duration = 0u64;
        for i in 1..self.entries.len() {
            let dt = self.entries[i].0 - self.entries[i - 1].0;
            if dt > 0 {
                weighted_sum += self.entries[i - 1].1 * dt as f64;
                total_duration += dt;
            }
        }
        if total_duration == 0 {
            return None;
        }
        Some(weighted_sum / total_duration as f64)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ─── Outlier Rejection ──────────────────────────────────────────────────

fn reject_outliers(values: &[f64], max_deviation_factor: f64) -> Vec<f64> {
    if values.len() < 3 {
        return values.to_vec();
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = sorted[sorted.len() / 2];
    if median == 0.0 {
        return values.to_vec();
    }
    values
        .iter()
        .copied()
        .filter(|&v| {
            let deviation = ((v - median) / median).abs();
            deviation <= max_deviation_factor
        })
        .collect()
}

// ─── Oracle Consensus Round ──────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum ConsensusError {
    InsufficientVotes { have: usize, need: usize },
    NoNumericValues,
    ExcessiveSpread { spread_pct: f64, max_pct: f64 },
    DuplicateVoter(u64),
    RoundMismatch { expected: u64, got: u64 },
    InvalidVote(String),
}

#[derive(Debug)]
pub struct OracleConsensusRound {
    key: String,
    round: u64,
    quorum: usize,
    max_spread_pct: f64,
    outlier_factor: f64,
    votes: HashMap<u64, OracleVote>,
    twap: TwapAccumulator,
}

impl OracleConsensusRound {
    pub fn new(key: &str, round: u64, quorum: usize, twap_window: u64) -> Self {
        Self {
            key: key.to_string(),
            round,
            quorum,
            max_spread_pct: 5.0,
            outlier_factor: 0.10,
            votes: HashMap::new(),
            twap: TwapAccumulator::new(twap_window),
        }
    }

    pub fn with_spread_tolerance(mut self, pct: f64) -> Self {
        self.max_spread_pct = pct;
        self
    }

    pub fn with_outlier_factor(mut self, factor: f64) -> Self {
        self.outlier_factor = factor;
        self
    }

    /// Submit a vote signed by the validator identified by `vote.validator_id`.
    ///
    /// `validator_pubkey` MUST be the canonical BLS public key for that
    /// validator as recorded in the validator set — it must NOT be supplied
    /// from the vote payload. The caller is responsible for the validator-set
    /// lookup; this function only verifies the BLS signature against the key
    /// it is given.
    ///
    /// Empty signatures are rejected. There is no test/dev bypass.
    pub fn submit_vote(
        &mut self,
        vote: OracleVote,
        validator_pubkey: &BlsPublicKey,
    ) -> Result<(), ConsensusError> {
        if vote.round != self.round {
            return Err(ConsensusError::RoundMismatch {
                expected: self.round,
                got: vote.round,
            });
        }
        if self.votes.contains_key(&vote.validator_id) {
            return Err(ConsensusError::DuplicateVoter(vote.validator_id));
        }
        if vote.signature.is_empty() {
            return Err(ConsensusError::InvalidVote("missing signature".into()));
        }
        let sig = BlsSignature(vote.signature.clone());
        if !BlsVerifier::verify(&vote.signable_bytes(), &sig, validator_pubkey) {
            return Err(ConsensusError::InvalidVote(
                "BLS signature verification failed".into(),
            ));
        }
        self.votes.insert(vote.validator_id, vote);
        Ok(())
    }

    pub fn vote_count(&self) -> usize {
        self.votes.len()
    }

    pub fn has_quorum(&self) -> bool {
        self.votes.len() >= self.quorum
    }

    pub fn finalize(&mut self) -> Result<FinalizedOracleValue, ConsensusError> {
        if !self.has_quorum() {
            return Err(ConsensusError::InsufficientVotes {
                have: self.votes.len(),
                need: self.quorum,
            });
        }

        let raw_values: Vec<f64> = self
            .votes
            .values()
            .filter_map(|v| v.value.as_f64())
            .collect();

        if raw_values.is_empty() {
            return Err(ConsensusError::NoNumericValues);
        }

        let values = reject_outliers(&raw_values, self.outlier_factor);
        if values.is_empty() {
            return Err(ConsensusError::NoNumericValues);
        }

        let mut sorted = values.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median = sorted[sorted.len() / 2];

        let min = sorted[0];
        let max = sorted[sorted.len() - 1];
        if median > 0.0 {
            let spread_pct = ((max - min) / median) * 100.0;
            if spread_pct > self.max_spread_pct {
                return Err(ConsensusError::ExcessiveSpread {
                    spread_pct,
                    max_pct: self.max_spread_pct,
                });
            }
        }

        let timestamp = self.votes.values().map(|v| v.timestamp).max().unwrap_or(0);

        self.twap.push(timestamp, median);
        let twap_value = self.twap.twap();

        let aggregate_hash = self.compute_aggregate_hash();

        Ok(FinalizedOracleValue {
            key: self.key.clone(),
            value: median,
            round: self.round,
            timestamp,
            voter_count: self.votes.len(),
            aggregate_hash,
            twap: twap_value,
        })
    }

    fn compute_aggregate_hash(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(self.key.as_bytes());
        hasher.update(self.round.to_le_bytes());
        let mut ids: Vec<u64> = self.votes.keys().copied().collect();
        ids.sort();
        for id in ids {
            if let Some(vote) = self.votes.get(&id) {
                hasher.update(vote.vote_hash());
            }
        }
        hasher.finalize().into()
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────

pub fn make_vote(
    validator_id: u64,
    key: &str,
    value: f64,
    round: u64,
    timestamp: u64,
) -> OracleVote {
    OracleVote {
        validator_id,
        key: key.to_string(),
        value: OracleValue::Numeric(value),
        timestamp,
        round,
        signature: vec![],
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a signed test vote and return it together with its public key
    /// for `submit_vote`.
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
    fn test_vote_hash_deterministic() {
        let v = make_vote(1, "btc_usd", 60000.0, 1, 1000);
        assert_eq!(v.vote_hash(), v.vote_hash());
    }

    #[test]
    fn test_vote_hash_differs_on_validator() {
        let v1 = make_vote(1, "btc_usd", 60000.0, 1, 1000);
        let v2 = make_vote(2, "btc_usd", 60000.0, 1, 1000);
        assert_ne!(v1.vote_hash(), v2.vote_hash());
    }

    #[test]
    fn test_quorum_2_of_3() {
        let mut round = OracleConsensusRound::new("btc_usd", 1, 2, 3600);
        let kp0 = BlsKeypair::generate();
        let kp1 = BlsKeypair::generate();
        let (v0, pk0) = signed(&kp0, 0, "btc_usd", 60000.0, 1, 1000);
        round.submit_vote(v0, &pk0).unwrap();
        assert!(!round.has_quorum());
        let (v1, pk1) = signed(&kp1, 1, "btc_usd", 60100.0, 1, 1001);
        round.submit_vote(v1, &pk1).unwrap();
        assert!(round.has_quorum());
    }

    #[test]
    fn test_finalize_success() {
        let mut round = OracleConsensusRound::new("btc_usd", 1, 2, 3600);
        for (id, value, ts) in [
            (0u64, 60000.0, 1000u64),
            (1, 60100.0, 1001),
            (2, 60050.0, 1002),
        ] {
            let kp = BlsKeypair::generate();
            let (v, pk) = signed(&kp, id, "btc_usd", value, 1, ts);
            round.submit_vote(v, &pk).unwrap();
        }
        let result = round.finalize().unwrap();
        assert_eq!(result.key, "btc_usd");
        assert_eq!(result.voter_count, 3);
        assert_eq!(result.round, 1);
        assert!(result.value >= 60000.0 && result.value <= 60100.0);
    }

    #[test]
    fn test_insufficient_votes() {
        let mut round = OracleConsensusRound::new("btc_usd", 1, 3, 3600);
        for (id, value, ts) in [(0u64, 60000.0, 1000u64), (1, 60100.0, 1001)] {
            let kp = BlsKeypair::generate();
            let (v, pk) = signed(&kp, id, "btc_usd", value, 1, ts);
            round.submit_vote(v, &pk).unwrap();
        }
        match round.finalize() {
            Err(ConsensusError::InsufficientVotes { have: 2, need: 3 }) => {}
            other => panic!("expected InsufficientVotes, got {:?}", other),
        }
    }

    #[test]
    fn test_duplicate_voter_rejected() {
        let mut round = OracleConsensusRound::new("btc_usd", 1, 2, 3600);
        let kp = BlsKeypair::generate();
        let (v0, pk0) = signed(&kp, 0, "btc_usd", 60000.0, 1, 1000);
        round.submit_vote(v0, &pk0).unwrap();
        let (v0_dup, _) = signed(&kp, 0, "btc_usd", 60100.0, 1, 1001);
        match round.submit_vote(v0_dup, &pk0) {
            Err(ConsensusError::DuplicateVoter(0)) => {}
            other => panic!("expected DuplicateVoter, got {:?}", other),
        }
    }

    #[test]
    fn test_round_mismatch_rejected() {
        let mut round = OracleConsensusRound::new("btc_usd", 1, 2, 3600);
        let kp = BlsKeypair::generate();
        let (v, pk) = signed(&kp, 0, "btc_usd", 60000.0, 2, 1000);
        match round.submit_vote(v, &pk) {
            Err(ConsensusError::RoundMismatch {
                expected: 1,
                got: 2,
            }) => {}
            other => panic!("expected RoundMismatch, got {:?}", other),
        }
    }

    #[test]
    fn test_missing_signature_rejected() {
        let mut round = OracleConsensusRound::new("btc_usd", 1, 2, 3600);
        let kp = BlsKeypair::generate();
        let v = make_vote(0, "btc_usd", 60000.0, 1, 1000);
        match round.submit_vote(v, &kp.public_key_bytes()) {
            Err(ConsensusError::InvalidVote(msg)) if msg.contains("missing signature") => {}
            other => panic!("expected InvalidVote(missing signature), got {:?}", other),
        }
    }

    #[test]
    fn test_forged_signature_rejected() {
        let mut round = OracleConsensusRound::new("btc_usd", 1, 2, 3600);
        let real_kp = BlsKeypair::generate();
        let attacker_kp = BlsKeypair::generate();
        // Attacker signs the vote with their own key but presents the real
        // validator's public key — the signature verification must fail.
        let (v, _attacker_pk) = signed(&attacker_kp, 0, "btc_usd", 60000.0, 1, 1000);
        match round.submit_vote(v, &real_kp.public_key_bytes()) {
            Err(ConsensusError::InvalidVote(msg)) if msg.contains("verification failed") => {}
            other => panic!("expected InvalidVote(verification failed), got {:?}", other),
        }
    }

    #[test]
    fn test_excessive_spread_rejected() {
        let mut round = OracleConsensusRound::new("btc_usd", 1, 2, 3600).with_spread_tolerance(1.0);
        for (id, value, ts) in [
            (0u64, 60000.0, 1000u64),
            (1, 65000.0, 1001),
            (2, 62000.0, 1002),
        ] {
            let kp = BlsKeypair::generate();
            let (v, pk) = signed(&kp, id, "btc_usd", value, 1, ts);
            round.submit_vote(v, &pk).unwrap();
        }
        match round.finalize() {
            Err(ConsensusError::ExcessiveSpread { .. }) => {}
            other => panic!("expected ExcessiveSpread, got {:?}", other),
        }
    }

    #[test]
    fn test_outlier_rejection() {
        let values = vec![60000.0, 60050.0, 60100.0, 99999.0];
        let filtered = reject_outliers(&values, 0.05);
        assert!(!filtered.contains(&99999.0));
        assert!(filtered.contains(&60000.0));
    }

    #[test]
    fn test_twap_single_entry() {
        // Single-entry TWAP returns None — requires at least 2 data points
        // with non-zero time separation to prevent single-block manipulation.
        let mut twap = TwapAccumulator::new(3600);
        twap.push(1000, 60000.0);
        assert_eq!(twap.twap(), None);
    }

    #[test]
    fn test_twap_two_entries() {
        let mut twap = TwapAccumulator::new(3600);
        twap.push(1000, 60000.0);
        twap.push(2000, 61000.0);
        let t = twap.twap().unwrap();
        assert!((t - 60000.0).abs() < 0.01);
    }

    #[test]
    fn test_twap_window_eviction() {
        let mut twap = TwapAccumulator::new(100);
        twap.push(1000, 50000.0);
        twap.push(1050, 55000.0);
        twap.push(1120, 60000.0);
        // Window=100, latest=1120, cutoff=1020. Entry at 1000 evicted, 1050+1120 remain.
        assert_eq!(twap.len(), 2);
    }

    #[test]
    fn test_aggregate_hash_deterministic() {
        // Use the same keypairs across both rounds so signatures match
        // byte-for-byte and the aggregate hash is reproducible.
        let kp0 = BlsKeypair::generate();
        let kp1 = BlsKeypair::generate();
        let (v0a, pk0) = signed(&kp0, 0, "btc_usd", 60000.0, 1, 1000);
        let (v1a, pk1) = signed(&kp1, 1, "btc_usd", 60100.0, 1, 1001);
        let (v0b, _) = signed(&kp0, 0, "btc_usd", 60000.0, 1, 1000);
        let (v1b, _) = signed(&kp1, 1, "btc_usd", 60100.0, 1, 1001);

        let mut r1 = OracleConsensusRound::new("btc_usd", 1, 2, 3600);
        r1.submit_vote(v0a, &pk0).unwrap();
        r1.submit_vote(v1a, &pk1).unwrap();

        let mut r2 = OracleConsensusRound::new("btc_usd", 1, 2, 3600);
        r2.submit_vote(v0b, &pk0).unwrap();
        r2.submit_vote(v1b, &pk1).unwrap();

        assert_eq!(r1.compute_aggregate_hash(), r2.compute_aggregate_hash());
    }

    #[test]
    fn test_finalize_includes_twap() {
        // TWAP requires ≥ 2 accumulator entries with non-zero time separation
        // (see test_twap_single_entry). Each finalize() pushes a single
        // (max_vote_timestamp, median) into the round's accumulator, so the
        // first finalize on a fresh round always reports twap=None.
        // Submit a second vote with a later timestamp and finalize again to
        // confirm the wiring populates the field once history exists.
        let mut round = OracleConsensusRound::new("btc_usd", 1, 1, 3600);
        let kp0 = BlsKeypair::generate();
        let (v0, pk0) = signed(&kp0, 0, "btc_usd", 60000.0, 1, 1000);
        round.submit_vote(v0, &pk0).unwrap();
        let first = round.finalize().unwrap();
        assert!(first.twap.is_none(), "first finalize: only 1 accumulator entry");

        let kp1 = BlsKeypair::generate();
        let (v1, pk1) = signed(&kp1, 1, "btc_usd", 60100.0, 1, 2000);
        round.submit_vote(v1, &pk1).unwrap();
        let second = round.finalize().unwrap();
        assert!(second.twap.is_some(), "second finalize: TWAP should be populated");
    }

    #[test]
    fn test_vote_serialization_roundtrip() {
        let v = make_vote(42, "eth_usd", 3000.0, 5, 9999);
        let json = serde_json::to_string(&v).unwrap();
        let v2: OracleVote = serde_json::from_str(&json).unwrap();
        assert_eq!(v, v2);
    }
}
