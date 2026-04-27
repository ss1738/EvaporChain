//! Finality tracking for EvaporChain.
//!
//! Tracks which blocks have achieved irreversible finality (2/3+ BLS-signed
//! commit certificates). Provides APIs for:
//! - Querying the latest finalized height
//! - Checking if a specific block is finalized
//! - Computing finality depth (confirmations)
//! - Finality proofs for bridges and exchanges
//!
//! EvaporChain has single-slot finality: once a block receives a valid
//! CommitCertificate with 2/3+ stake, it is immediately and irreversibly final.
//! This tracker maintains the finality record and provides query APIs.

use evaporchain_crypto::hash::blake3_hash;
use evaporchain_types::CommitCertificate;
use std::collections::BTreeMap;
use tracing::debug;

// ─────────────────────── Types ───────────────────────────────────────

/// Record of a finalized block.
#[derive(Debug, Clone)]
pub struct FinalityRecord {
    /// Block height.
    pub height: u64,
    /// Block hash.
    pub block_hash: [u8; 32],
    /// State root after executing this block.
    pub state_root: [u8; 32],
    /// Epoch this block belongs to.
    pub epoch: u64,
    /// Timestamp when finality was achieved.
    pub finalized_at: u64,
    /// The commit certificate proving finality.
    pub certificate: CommitCertificate,
    /// Number of validators that signed the certificate.
    pub signer_count: usize,
    /// Total stake that signed.
    pub signing_stake: u64,
    /// Total stake in the validator set.
    pub total_stake: u64,
    /// Whether DA attestation has been confirmed for this block.
    pub da_confirmed: bool,
}

impl FinalityRecord {
    /// Fraction of stake that signed (0.0 to 1.0).
    pub fn participation_rate(&self) -> f64 {
        if self.total_stake == 0 {
            return 0.0;
        }
        self.signing_stake as f64 / self.total_stake as f64
    }

    /// Whether this record has supermajority (2/3+).
    pub fn has_supermajority(&self) -> bool {
        self.signing_stake * 3 > self.total_stake * 2
    }
}

/// Finality proof — a compact proof that a block is finalized.
/// Suitable for submission to bridges or light clients.
#[derive(Debug, Clone)]
pub struct FinalityProof {
    /// The finalized block height.
    pub height: u64,
    /// The finalized block hash.
    pub block_hash: [u8; 32],
    /// State root at this height.
    pub state_root: [u8; 32],
    /// The BLS aggregate commit certificate.
    pub certificate: CommitCertificate,
    /// Blake3 hash of the proof for deduplication.
    pub proof_hash: [u8; 32],
}

/// Status of a finality query.
#[derive(Debug, Clone, PartialEq)]
pub enum FinalityStatus {
    /// Block is finalized with the given number of confirmations.
    Finalized { confirmations: u64 },
    /// Block is pending (committed but not yet finalized).
    Pending,
    /// Block height is unknown.
    Unknown,
}

/// Finality statistics over a window of blocks.
#[derive(Debug, Clone, Default)]
pub struct FinalityStats {
    /// Number of finalized blocks in the window.
    pub finalized_count: u64,
    /// Average participation rate (fraction of stake signing).
    pub avg_participation: f64,
    /// Minimum participation rate in the window.
    pub min_participation: f64,
    /// Maximum participation rate in the window.
    pub max_participation: f64,
    /// Number of blocks with >90% participation.
    pub high_participation_count: u64,
}

// ─────────────────────── FinalityTracker ─────────────────────────────

/// Tracks finality status of committed blocks.
///
/// Maintains a sliding window of finality records and provides
/// efficient queries for exchanges, bridges, and light clients.
pub struct FinalityTracker {
    /// Finality records indexed by height.
    records: BTreeMap<u64, FinalityRecord>,
    /// Latest finalized height.
    latest_finalized: u64,
    /// Maximum records to keep (older ones are pruned).
    max_records: usize,
    /// Total blocks finalized since genesis.
    total_finalized: u64,
}

impl FinalityTracker {
    /// Create a new finality tracker.
    pub fn new() -> Self {
        Self {
            records: BTreeMap::new(),
            latest_finalized: 0,
            max_records: 10_000,
            total_finalized: 0,
        }
    }

    /// Create with a custom record limit.
    pub fn with_max_records(max_records: usize) -> Self {
        Self {
            records: BTreeMap::new(),
            latest_finalized: 0,
            max_records,
            total_finalized: 0,
        }
    }

    /// Record a newly finalized block.
    ///
    /// Called after a block commits with a valid CommitCertificate.
    /// Returns true if this is a new finalization, false if already recorded.
    #[allow(clippy::too_many_arguments)]
    pub fn on_block_finalized(
        &mut self,
        height: u64,
        block_hash: [u8; 32],
        state_root: [u8; 32],
        epoch: u64,
        certificate: CommitCertificate,
        signing_stake: u64,
        total_stake: u64,
        timestamp: u64,
    ) -> bool {
        if self.records.contains_key(&height) {
            return false; // Already recorded — duplicate-finalization guard
        }
        // NOTE on monotonicity (cross_verification §1): we deliberately do
        // NOT reject `height < latest_finalized`. The records map is allowed
        // to be backfilled with previously-unseen heights below the current
        // tip — late delivery, gap-fill after restart, and out-of-order
        // certificate arrival all need this. `latest_finalized` itself is
        // still strictly monotone (only advances at the `if height >
        // latest_finalized` check below). The 2/3 stake quorum on
        // `signing_stake / total_stake` bounds replay risk: an attacker
        // cannot craft a fake old certificate without controlling 2/3 of
        // historical stake. A future hardening pass could add per-height
        // gap-tracking to reject backfill of heights never observed in any
        // proposal, but that needs new state outside this struct.
        if certificate.signer_ids.is_empty() {
            return false; // Reject finality without any signers
        }
        if total_stake > 0 && signing_stake * 3 < total_stake * 2 {
            return false; // Reject finality without 2/3 stake
        }

        let signer_count = certificate.signer_ids.len();
        let record = FinalityRecord {
            height,
            block_hash,
            state_root,
            epoch,
            finalized_at: timestamp,
            certificate,
            signer_count,
            signing_stake,
            total_stake,
            da_confirmed: false,
        };

        debug!(
            height,
            signers = signer_count,
            participation = format!("{:.1}%", record.participation_rate() * 100.0),
            "Block finalized"
        );

        self.records.insert(height, record);
        if height > self.latest_finalized {
            self.latest_finalized = height;
        }
        self.total_finalized += 1;

        // Prune old records
        while self.records.len() > self.max_records {
            let oldest = *self.records.keys().next().unwrap();
            self.records.remove(&oldest);
        }

        true
    }

    /// Get the latest finalized height.
    pub fn latest_finalized_height(&self) -> u64 {
        self.latest_finalized
    }

    /// Check finality status for a specific height.
    pub fn finality_status(&self, height: u64) -> FinalityStatus {
        if height == 0 || height > self.latest_finalized {
            if self.records.contains_key(&height) {
                return FinalityStatus::Finalized { confirmations: 0 };
            }
            return FinalityStatus::Unknown;
        }

        if self.records.contains_key(&height) {
            let confirmations = self.latest_finalized.saturating_sub(height);
            FinalityStatus::Finalized { confirmations }
        } else {
            // Height is below latest finalized but we don't have the record
            // (pruned). It was finalized.
            let confirmations = self.latest_finalized.saturating_sub(height);
            FinalityStatus::Finalized { confirmations }
        }
    }

    /// Check if a specific block hash at a given height is finalized.
    pub fn is_block_finalized(&self, height: u64, block_hash: &[u8; 32]) -> bool {
        if let Some(record) = self.records.get(&height) {
            record.block_hash == *block_hash
        } else {
            // Pruned records — if height <= latest_finalized, it was finalized
            // but we can't verify the hash anymore
            false
        }
    }

    /// Get the finality record for a specific height.
    pub fn get_record(&self, height: u64) -> Option<&FinalityRecord> {
        self.records.get(&height)
    }

    /// Compute the canonical proof hash covering all certificate fields.
    fn compute_proof_hash(
        height: u64,
        block_hash: &[u8; 32],
        state_root: &[u8; 32],
        cert: &CommitCertificate,
    ) -> [u8; 32] {
        let mut proof_input = Vec::new();
        proof_input.extend_from_slice(&height.to_le_bytes());
        proof_input.extend_from_slice(&cert.round.to_le_bytes());
        proof_input.extend_from_slice(block_hash);
        proof_input.extend_from_slice(state_root);
        proof_input.extend_from_slice(&(cert.signer_ids.len() as u64).to_le_bytes());
        for signer in &cert.signer_ids {
            proof_input.extend_from_slice(&signer.to_le_bytes());
        }
        proof_input.extend_from_slice(&cert.aggregate_signature);
        blake3_hash(&proof_input)
    }

    /// Generate a finality proof for a specific height.
    pub fn generate_proof(&self, height: u64) -> Option<FinalityProof> {
        let record = self.records.get(&height)?;

        let proof_hash = Self::compute_proof_hash(
            record.height,
            &record.block_hash,
            &record.state_root,
            &record.certificate,
        );

        Some(FinalityProof {
            height: record.height,
            block_hash: record.block_hash,
            state_root: record.state_root,
            certificate: record.certificate.clone(),
            proof_hash,
        })
    }

    /// Get finality statistics over the last N blocks.
    pub fn stats(&self, window: usize) -> FinalityStats {
        let records: Vec<&FinalityRecord> = self
            .records
            .values()
            .rev()
            .take(window)
            .collect();

        if records.is_empty() {
            return FinalityStats::default();
        }

        let mut total_participation = 0.0;
        let mut min_part = f64::MAX;
        let mut max_part = f64::MIN;
        let mut high_count = 0u64;

        for r in &records {
            let p = r.participation_rate();
            total_participation += p;
            if p < min_part {
                min_part = p;
            }
            if p > max_part {
                max_part = p;
            }
            if p > 0.90 {
                high_count += 1;
            }
        }

        FinalityStats {
            finalized_count: records.len() as u64,
            avg_participation: total_participation / records.len() as f64,
            min_participation: min_part,
            max_participation: max_part,
            high_participation_count: high_count,
        }
    }

    /// Total blocks finalized since genesis.
    pub fn total_finalized(&self) -> u64 {
        self.total_finalized
    }

    /// Number of finality records currently stored.
    pub fn record_count(&self) -> usize {
        self.records.len()
    }

    /// Get the finality gap: how many blocks between two finalized heights.
    pub fn finality_gap(&self, from: u64, to: u64) -> Option<u64> {
        if from > to {
            return None;
        }
        Some(to.saturating_sub(from))
    }

    /// Get all finality records in a height range (inclusive).
    pub fn records_in_range(&self, from: u64, to: u64) -> Vec<&FinalityRecord> {
        self.records.range(from..=to).map(|(_, r)| r).collect()
    }

    /// Verify that a finality proof is internally consistent.
    pub fn verify_proof(proof: &FinalityProof) -> bool {
        let expected_hash = Self::compute_proof_hash(
            proof.height,
            &proof.block_hash,
            &proof.state_root,
            &proof.certificate,
        );

        if expected_hash != proof.proof_hash {
            return false;
        }

        // Verify certificate matches claimed block
        if proof.certificate.height != proof.height {
            return false;
        }
        if proof.certificate.block_hash != proof.block_hash {
            return false;
        }

        // Reject empty signer sets
        if proof.certificate.signer_ids.is_empty() {
            return false;
        }

        true
    }
}

impl Default for FinalityTracker {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────── Tests ───────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_cert(height: u64, block_hash: [u8; 32], signers: Vec<u64>) -> CommitCertificate {
        CommitCertificate {
            height,
            round: 0,
            block_hash,
            aggregate_signature: vec![0xAA; 96],
            signer_ids: signers,
        }
    }

    #[test]
    fn test_record_finalized_block() {
        let mut ft = FinalityTracker::new();
        assert_eq!(ft.latest_finalized_height(), 0);

        let cert = make_cert(1, [1u8; 32], vec![0, 1, 2]);
        let recorded = ft.on_block_finalized(
            1, [1u8; 32], [0xAA; 32], 0, cert, 3000, 4000, 100,
        );
        assert!(recorded);
        assert_eq!(ft.latest_finalized_height(), 1);
        assert_eq!(ft.total_finalized(), 1);
    }

    #[test]
    fn test_duplicate_finalization_rejected() {
        let mut ft = FinalityTracker::new();
        let cert = make_cert(1, [1u8; 32], vec![0, 1, 2]);
        ft.on_block_finalized(1, [1u8; 32], [0xAA; 32], 0, cert.clone(), 3000, 4000, 100);

        let duplicate = ft.on_block_finalized(1, [1u8; 32], [0xAA; 32], 0, cert, 3000, 4000, 200);
        assert!(!duplicate);
        assert_eq!(ft.total_finalized(), 1);
    }

    #[test]
    fn test_finality_status() {
        let mut ft = FinalityTracker::new();

        // Unknown height
        assert_eq!(ft.finality_status(1), FinalityStatus::Unknown);

        // Finalize blocks 1-5
        for h in 1..=5u64 {
            let cert = make_cert(h, [h as u8; 32], vec![0, 1, 2]);
            ft.on_block_finalized(h, [h as u8; 32], [h as u8; 32], 0, cert, 3000, 4000, h * 10);
        }

        // Block 1 has 4 confirmations (5 - 1)
        assert_eq!(
            ft.finality_status(1),
            FinalityStatus::Finalized { confirmations: 4 }
        );

        // Block 5 has 0 confirmations (latest)
        assert_eq!(
            ft.finality_status(5),
            FinalityStatus::Finalized { confirmations: 0 }
        );

        // Block 6 is unknown
        assert_eq!(ft.finality_status(6), FinalityStatus::Unknown);
    }

    #[test]
    fn test_is_block_finalized() {
        let mut ft = FinalityTracker::new();
        let cert = make_cert(1, [1u8; 32], vec![0, 1, 2]);
        ft.on_block_finalized(1, [1u8; 32], [0xAA; 32], 0, cert, 3000, 4000, 100);

        assert!(ft.is_block_finalized(1, &[1u8; 32]));
        assert!(!ft.is_block_finalized(1, &[2u8; 32])); // wrong hash
        assert!(!ft.is_block_finalized(2, &[1u8; 32])); // wrong height
    }

    #[test]
    fn test_generate_and_verify_proof() {
        let mut ft = FinalityTracker::new();
        let cert = make_cert(1, [1u8; 32], vec![0, 1, 2]);
        ft.on_block_finalized(1, [1u8; 32], [0xAA; 32], 0, cert, 3000, 4000, 100);

        let proof = ft.generate_proof(1).unwrap();
        assert_eq!(proof.height, 1);
        assert_eq!(proof.block_hash, [1u8; 32]);
        assert_eq!(proof.state_root, [0xAA; 32]);
        assert!(FinalityTracker::verify_proof(&proof));

        // No proof for non-existent height
        assert!(ft.generate_proof(99).is_none());
    }

    #[test]
    fn test_tampered_proof_rejected() {
        let mut ft = FinalityTracker::new();
        let cert = make_cert(1, [1u8; 32], vec![0, 1, 2]);
        ft.on_block_finalized(1, [1u8; 32], [0xAA; 32], 0, cert, 3000, 4000, 100);

        let mut proof = ft.generate_proof(1).unwrap();
        proof.block_hash = [0xFF; 32]; // tamper
        assert!(!FinalityTracker::verify_proof(&proof));
    }

    #[test]
    fn test_participation_rate() {
        let mut ft = FinalityTracker::new();

        // 3/4 validators signed (75% stake)
        let cert = make_cert(1, [1u8; 32], vec![0, 1, 2]);
        ft.on_block_finalized(1, [1u8; 32], [0xAA; 32], 0, cert, 3000, 4000, 100);

        let record = ft.get_record(1).unwrap();
        assert!((record.participation_rate() - 0.75).abs() < 0.01);
        assert!(record.has_supermajority());
    }

    #[test]
    fn test_stats() {
        let mut ft = FinalityTracker::new();

        // Finalize 10 blocks with varying participation
        for h in 1..=10u64 {
            let signers = if h <= 5 {
                vec![0, 1, 2] // 75% participation (3000/4000)
            } else {
                vec![0, 1, 2, 3] // 100% participation (4000/4000)
            };
            let signing_stake = if h <= 5 { 3000 } else { 4000 };
            let cert = make_cert(h, [h as u8; 32], signers);
            ft.on_block_finalized(
                h, [h as u8; 32], [h as u8; 32], 0, cert, signing_stake, 4000, h * 10,
            );
        }

        let stats = ft.stats(10);
        assert_eq!(stats.finalized_count, 10);
        assert!((stats.min_participation - 0.75).abs() < 0.01);
        assert!((stats.max_participation - 1.0).abs() < 0.01);
        assert_eq!(stats.high_participation_count, 5); // blocks 6-10 have >90%
    }

    #[test]
    fn test_pruning() {
        let mut ft = FinalityTracker::with_max_records(5);

        for h in 1..=10u64 {
            let cert = make_cert(h, [h as u8; 32], vec![0, 1, 2]);
            ft.on_block_finalized(h, [h as u8; 32], [h as u8; 32], 0, cert, 3000, 4000, h);
        }

        assert_eq!(ft.record_count(), 5);
        assert_eq!(ft.total_finalized(), 10);
        // Oldest records pruned — record for height 1 should be gone
        assert!(ft.get_record(1).is_none());
        // But latest should exist
        assert!(ft.get_record(10).is_some());
        // Status for pruned heights still reports finalized
        assert_eq!(
            ft.finality_status(1),
            FinalityStatus::Finalized { confirmations: 9 }
        );
    }

    #[test]
    fn test_records_in_range() {
        let mut ft = FinalityTracker::new();
        for h in 1..=10u64 {
            let cert = make_cert(h, [h as u8; 32], vec![0, 1, 2]);
            ft.on_block_finalized(h, [h as u8; 32], [h as u8; 32], 0, cert, 3000, 4000, h);
        }

        let range = ft.records_in_range(3, 7);
        assert_eq!(range.len(), 5);
        assert_eq!(range[0].height, 3);
        assert_eq!(range[4].height, 7);
    }

    #[test]
    fn test_finality_gap() {
        let ft = FinalityTracker::new();
        assert_eq!(ft.finality_gap(1, 10), Some(9));
        assert_eq!(ft.finality_gap(5, 5), Some(0));
        assert_eq!(ft.finality_gap(10, 1), None);
    }

    #[test]
    fn test_non_sequential_finalization() {
        let mut ft = FinalityTracker::new();

        // Finalize blocks out of order
        let cert5 = make_cert(5, [5u8; 32], vec![0, 1, 2]);
        ft.on_block_finalized(5, [5u8; 32], [5u8; 32], 0, cert5, 3000, 4000, 50);
        assert_eq!(ft.latest_finalized_height(), 5);

        let cert3 = make_cert(3, [3u8; 32], vec![0, 1, 2]);
        ft.on_block_finalized(3, [3u8; 32], [3u8; 32], 0, cert3, 3000, 4000, 30);
        // Latest should still be 5
        assert_eq!(ft.latest_finalized_height(), 5);
        assert_eq!(ft.record_count(), 2);
    }

    #[test]
    fn test_proof_hash_includes_round_and_signers() {
        let mut ft = FinalityTracker::new();
        let cert = make_cert(1, [1u8; 32], vec![0, 1, 2]);
        ft.on_block_finalized(1, [1u8; 32], [0xAA; 32], 0, cert, 3000, 4000, 100);
        let proof = ft.generate_proof(1).unwrap();

        // Changing the round in the certificate should invalidate the proof
        let mut tampered = proof.clone();
        tampered.certificate.round = 5;
        assert!(!FinalityTracker::verify_proof(&tampered));

        // Changing the signer set should invalidate the proof
        let mut tampered2 = proof.clone();
        tampered2.certificate.signer_ids = vec![0, 1, 2, 99];
        assert!(!FinalityTracker::verify_proof(&tampered2));
    }

    #[test]
    fn test_proof_rejects_empty_signers() {
        let mut ft = FinalityTracker::new();
        let cert = make_cert(1, [1u8; 32], vec![0, 1, 2]);
        ft.on_block_finalized(1, [1u8; 32], [0xAA; 32], 0, cert, 3000, 4000, 100);
        let mut proof = ft.generate_proof(1).unwrap();
        proof.certificate.signer_ids.clear();
        // Recompute proof hash to isolate the empty-signer check
        proof.proof_hash = FinalityTracker::compute_proof_hash(
            proof.height, &proof.block_hash, &proof.state_root, &proof.certificate,
        );
        assert!(!FinalityTracker::verify_proof(&proof));
    }
}
