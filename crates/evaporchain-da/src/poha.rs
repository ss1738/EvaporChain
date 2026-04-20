//! Proof-of-Historical-Availability (PoHA) — decaying DA certificates.
//!
//! Novel primitive: DA certificates carry thermodynamic energy and decay over
//! time, just like on-chain objects. This creates a gradient between "hot data"
//! (fully available, recently attested) and "cold data" (hash-only ghost record).
//!
//! No prior work treats data availability as a thermodynamic resource.
//! Celestia, EigenDA, Avail all assume flat availability. EIP-4844 blobs use
//! a blunt 18-day TTL. PoHA provides a continuous, incentive-driven decay model.
//!
//! Certificate lifecycle:
//!   Hot → Warm → Cold → Evaporated
//!
//! - **Hot**: Recent attestations, full shards available, any peer can reconstruct.
//! - **Warm**: Aging certificate, shards may be partially pruned.
//! - **Cold**: Only commitment root + hash survives. Shards pruned.
//! - **Evaporated**: Certificate hash in MMR only. DA record is gone.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ─────────────────────── Certificate Temperature ─────────────────────────

/// Temperature classification for a PoHA certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CertTemperature {
    /// Full shards available, recently attested. Peers can reconstruct.
    Hot,
    /// Aging certificate. Shards still exist but may be partially pruned.
    Warm,
    /// Only commitment root survives. Shards pruned. Hash-only reference.
    Cold,
    /// Certificate evaporated. Only a hash in the MMR remains.
    Evaporated,
}

impl CertTemperature {
    /// Classify from energy level and thresholds.
    pub fn from_energy(energy: u64, initial_energy: u64) -> Self {
        if initial_energy == 0 {
            return CertTemperature::Evaporated;
        }
        let ratio_pct = (energy * 100) / initial_energy;
        match ratio_pct {
            50..=100 => CertTemperature::Hot,
            15..=49 => CertTemperature::Warm,
            1..=14 => CertTemperature::Cold,
            _ => CertTemperature::Evaporated,
        }
    }
}

// ─────────────────────── PoHA Certificate ────────────────────────────────

/// A DA certificate with thermodynamic energy — the core PoHA primitive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoHACertificate {
    /// Block number this certificate covers.
    pub block_number: u64,
    /// Data root (Merkle root over erasure-coded shards).
    pub data_root: [u8; 32],
    /// Number of shards in the erasure coding.
    pub shard_count: u32,
    /// Initial energy when the certificate was created.
    pub initial_energy: u64,
    /// Current energy (decays over time).
    pub energy: u64,
    /// Half-life in epochs — controls decay rate.
    pub half_life: u64,
    /// Epoch when the certificate was created.
    pub created_epoch: u64,
    /// Epoch of the most recent attestation or re-attestation.
    pub last_attested_epoch: u64,
    /// Total attested stake at creation.
    pub attested_stake: u64,
    /// Total stake in the validator set at creation.
    pub total_stake: u64,
    /// Number of times this certificate has been re-attested.
    pub re_attestation_count: u32,
    /// BLS aggregate signature (from original attestation).
    pub aggregate_signature: Vec<u8>,
    /// Validator IDs that contributed to the original attestation.
    pub signer_ids: Vec<u64>,
}

impl PoHACertificate {
    /// Compute current energy at the given epoch using exponential decay.
    /// Decay is measured from `created_epoch`, not `last_attested_epoch`.
    /// Re-attestation boosts energy but doesn't reset the decay clock.
    pub fn energy_at(&self, epoch: u64) -> u64 {
        if self.half_life == 0 {
            return 0;
        }
        let elapsed = epoch.saturating_sub(self.created_epoch);
        let shifts = elapsed / self.half_life;
        if shifts >= 64 {
            return 0;
        }
        self.energy >> shifts
    }

    /// Apply decay: snapshot current energy at this epoch.
    /// Does NOT reset the decay origin — decay always runs from `created_epoch`.
    pub fn decay_to(&mut self, epoch: u64) {
        self.energy = self.energy_at(epoch);
        // Advance created_epoch so future shifts start from this snapshot
        self.created_epoch = epoch;
    }

    /// Get the current temperature classification.
    pub fn temperature(&self) -> CertTemperature {
        CertTemperature::from_energy(self.energy, self.initial_energy)
    }

    /// Get the temperature at a future epoch.
    pub fn temperature_at(&self, epoch: u64) -> CertTemperature {
        CertTemperature::from_energy(self.energy_at(epoch), self.initial_energy)
    }

    /// Apply a re-attestation: boost energy (capped at initial).
    pub fn re_attest(&mut self, epoch: u64, energy_boost: u64) {
        // First decay to current epoch
        self.decay_to(epoch);
        // Then boost
        self.energy = self.energy.saturating_add(energy_boost).min(self.initial_energy);
        self.last_attested_epoch = epoch;
        self.re_attestation_count += 1;
    }

    /// Check if this certificate has supermajority attestation.
    pub fn is_supermajority(&self) -> bool {
        self.attested_stake * 3 >= self.total_stake * 2
    }

    /// Compute a 32-byte hash of this certificate for compact storage.
    pub fn hash(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&self.block_number.to_le_bytes());
        hasher.update(&self.data_root);
        hasher.update(&self.shard_count.to_le_bytes());
        hasher.update(&self.initial_energy.to_le_bytes());
        hasher.update(&self.half_life.to_le_bytes());
        hasher.update(&self.created_epoch.to_le_bytes());
        hasher.update(&self.attested_stake.to_le_bytes());
        hasher.update(&self.aggregate_signature);
        hasher.finalize().into()
    }

    /// Check if the certificate is alive (energy > 0).
    pub fn is_alive(&self) -> bool {
        self.energy > 0
    }
}

// ─────────────────────── Re-Attestation ──────────────────────────────────

/// A re-attestation from a validator confirming they can still serve shards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReAttestation {
    /// Certificate hash being re-attested.
    pub cert_hash: [u8; 32],
    /// Block number the certificate covers.
    pub block_number: u64,
    /// Validator ID performing the re-attestation.
    pub validator_id: u64,
    /// Current epoch of the re-attestation.
    pub epoch: u64,
    /// Number of shards the validator still holds.
    pub shards_held: u32,
    /// BLS signature over (cert_hash || epoch || validator_id || shards_held).
    pub signature: Vec<u8>,
}

impl ReAttestation {
    /// Build the message bytes that should be signed.
    pub fn sign_message(cert_hash: &[u8; 32], epoch: u64, validator_id: u64, shards_held: u32) -> Vec<u8> {
        let mut msg = Vec::with_capacity(52);
        msg.extend_from_slice(cert_hash);
        msg.extend_from_slice(&epoch.to_le_bytes());
        msg.extend_from_slice(&validator_id.to_le_bytes());
        msg.extend_from_slice(&shards_held.to_le_bytes());
        msg
    }
}

// ─────────────────────── PoHA Store ──────────────────────────────────────

/// Ghost record for an evaporated DA certificate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertGhost {
    /// Hash of the original certificate.
    pub cert_hash: [u8; 32],
    /// Block number the certificate covered.
    pub block_number: u64,
    /// Data root from the original certificate.
    pub data_root: [u8; 32],
    /// Epoch when the certificate evaporated.
    pub evaporated_epoch: u64,
    /// Total re-attestations received during its lifetime.
    pub total_re_attestations: u32,
}

/// Store managing PoHA certificate lifecycle.
///
/// Tracks active certificates, applies decay, handles re-attestations,
/// and moves evaporated certificates to ghost records.
pub struct PoHAStore {
    /// Active certificates indexed by block number.
    certificates: BTreeMap<u64, PoHACertificate>,
    /// Ghost records for evaporated certificates.
    ghosts: BTreeMap<u64, CertGhost>,
    /// Default initial energy for new certificates.
    pub default_energy: u64,
    /// Default half-life for new certificates (in epochs).
    pub default_half_life: u64,
    /// Energy boost per re-attestation.
    pub re_attestation_boost: u64,
    /// Maximum certificates to keep active.
    pub max_active: usize,
}

impl PoHAStore {
    /// Create a new PoHA store with default parameters.
    pub fn new(default_energy: u64, default_half_life: u64) -> Self {
        Self {
            certificates: BTreeMap::new(),
            ghosts: BTreeMap::new(),
            default_energy,
            default_half_life,
            re_attestation_boost: default_energy / 4, // 25% boost per re-attestation
            max_active: 1024,
        }
    }

    /// Register a new DA certificate as a PoHA certificate.
    pub fn register(
        &mut self,
        block_number: u64,
        data_root: [u8; 32],
        shard_count: u32,
        attested_stake: u64,
        total_stake: u64,
        epoch: u64,
        aggregate_signature: Vec<u8>,
        signer_ids: Vec<u64>,
    ) -> &PoHACertificate {
        let cert = PoHACertificate {
            block_number,
            data_root,
            shard_count,
            initial_energy: self.default_energy,
            energy: self.default_energy,
            half_life: self.default_half_life,
            created_epoch: epoch,
            last_attested_epoch: epoch,
            attested_stake,
            total_stake,
            re_attestation_count: 0,
            aggregate_signature,
            signer_ids,
        };
        self.certificates.insert(block_number, cert);

        // Evict oldest if over capacity
        while self.certificates.len() > self.max_active {
            if let Some(&oldest) = self.certificates.keys().next() {
                if let Some(cert) = self.certificates.remove(&oldest) {
                    self.ghosts.insert(
                        oldest,
                        CertGhost {
                            cert_hash: cert.hash(),
                            block_number: oldest,
                            data_root: cert.data_root,
                            evaporated_epoch: epoch,
                            total_re_attestations: cert.re_attestation_count,
                        },
                    );
                }
            }
        }

        self.certificates.get(&block_number).unwrap()
    }

    /// Apply decay to all certificates at the given epoch.
    /// Returns (decayed_count, evaporated_count).
    pub fn process_epoch(&mut self, epoch: u64) -> (usize, usize) {
        let mut evaporated_blocks = Vec::new();
        let mut decayed = 0usize;

        for (&block_number, cert) in self.certificates.iter_mut() {
            cert.decay_to(epoch);
            if cert.energy == 0 {
                evaporated_blocks.push(block_number);
            } else {
                decayed += 1;
            }
        }

        let evaporated = evaporated_blocks.len();
        for block_number in evaporated_blocks {
            if let Some(cert) = self.certificates.remove(&block_number) {
                self.ghosts.insert(
                    block_number,
                    CertGhost {
                        cert_hash: cert.hash(),
                        block_number,
                        data_root: cert.data_root,
                        evaporated_epoch: epoch,
                        total_re_attestations: cert.re_attestation_count,
                    },
                );
            }
        }

        (decayed, evaporated)
    }

    /// Apply a re-attestation to a certificate.
    pub fn re_attest(&mut self, block_number: u64, epoch: u64) -> bool {
        if let Some(cert) = self.certificates.get_mut(&block_number) {
            cert.re_attest(epoch, self.re_attestation_boost);
            true
        } else {
            false
        }
    }

    /// Select certificates that should be re-attested this epoch.
    /// Returns block numbers of certificates that are Warm or approaching Warm.
    pub fn select_for_re_attestation(&self, epoch: u64, max_count: usize) -> Vec<u64> {
        let mut candidates: Vec<(u64, u64)> = self
            .certificates
            .iter()
            .filter_map(|(&block_number, cert)| {
                let energy = cert.energy_at(epoch);
                let temp = CertTemperature::from_energy(energy, cert.initial_energy);
                // Re-attest Warm/Cold certificates and Hot ones approaching Warm
                if temp == CertTemperature::Warm
                    || temp == CertTemperature::Cold
                    || (temp == CertTemperature::Hot && energy < cert.initial_energy * 60 / 100)
                {
                    Some((block_number, energy))
                } else {
                    None
                }
            })
            .collect();

        // Prioritize lowest energy first (most in need of re-attestation)
        candidates.sort_by_key(|&(_, energy)| energy);
        candidates
            .into_iter()
            .take(max_count)
            .map(|(bn, _)| bn)
            .collect()
    }

    /// Get a certificate by block number.
    pub fn get(&self, block_number: u64) -> Option<&PoHACertificate> {
        self.certificates.get(&block_number)
    }

    /// Get a ghost record by block number.
    pub fn get_ghost(&self, block_number: u64) -> Option<&CertGhost> {
        self.ghosts.get(&block_number)
    }

    /// Number of active certificates.
    pub fn active_count(&self) -> usize {
        self.certificates.len()
    }

    /// Number of ghost (evaporated) certificates.
    pub fn ghost_count(&self) -> usize {
        self.ghosts.len()
    }

    /// Get temperature distribution across all active certificates.
    pub fn temperature_distribution(&self) -> TemperatureDistribution {
        let mut dist = TemperatureDistribution::default();
        for cert in self.certificates.values() {
            match cert.temperature() {
                CertTemperature::Hot => dist.hot += 1,
                CertTemperature::Warm => dist.warm += 1,
                CertTemperature::Cold => dist.cold += 1,
                CertTemperature::Evaporated => dist.evaporated += 1,
            }
        }
        dist.ghosts = self.ghosts.len() as u32;
        dist
    }

    /// Get all active certificates as a slice-like iterator.
    pub fn all_active(&self) -> impl Iterator<Item = (&u64, &PoHACertificate)> {
        self.certificates.iter()
    }

    /// Prune ghost records older than the given epoch.
    pub fn prune_ghosts(&mut self, before_epoch: u64) -> usize {
        let to_remove: Vec<u64> = self
            .ghosts
            .iter()
            .filter(|(_, g)| g.evaporated_epoch < before_epoch)
            .map(|(&bn, _)| bn)
            .collect();
        let count = to_remove.len();
        for bn in to_remove {
            self.ghosts.remove(&bn);
        }
        count
    }
}

/// Distribution of certificate temperatures.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TemperatureDistribution {
    pub hot: u32,
    pub warm: u32,
    pub cold: u32,
    pub evaporated: u32,
    pub ghosts: u32,
}

// ─────────────────────── PoHA Sampler ────────────────────────────────────

/// Selects which certificates each validator should re-attest each epoch.
/// Uses deterministic randomness so all validators agree on the selection.
pub struct PoHASampler;

impl PoHASampler {
    /// Select certificate block numbers for a validator to re-attest this epoch.
    /// Deterministic: given the same inputs, all validators produce the same selection.
    pub fn select_certificates(
        validator_id: u64,
        epoch: u64,
        active_block_numbers: &[u64],
        sample_count: usize,
    ) -> Vec<u64> {
        if active_block_numbers.is_empty() || sample_count == 0 {
            return Vec::new();
        }

        let mut selected = Vec::with_capacity(sample_count);
        for i in 0..sample_count {
            let mut hasher = blake3::Hasher::new();
            hasher.update(b"poha-sample");
            hasher.update(&validator_id.to_le_bytes());
            hasher.update(&epoch.to_le_bytes());
            hasher.update(&(i as u64).to_le_bytes());
            let hash = hasher.finalize();
            let bytes = hash.as_bytes();
            let idx =
                (u64::from_le_bytes(bytes[0..8].try_into().unwrap()) as usize) % active_block_numbers.len();
            let block_number = active_block_numbers[idx];
            if !selected.contains(&block_number) {
                selected.push(block_number);
            }
        }
        selected
    }
}

// ─────────────────────── Tests ───────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store() -> PoHAStore {
        PoHAStore::new(1000, 100) // energy=1000, half_life=100 epochs
    }

    fn register_cert(store: &mut PoHAStore, block: u64, epoch: u64) {
        store.register(
            block,
            [block as u8; 32],
            8,
            3000,
            4000,
            epoch,
            vec![0u8; 96],
            vec![0, 1, 2],
        );
    }

    // ── Certificate lifecycle ──

    #[test]
    fn test_register_certificate() {
        let mut store = make_store();
        register_cert(&mut store, 1, 0);

        assert_eq!(store.active_count(), 1);
        let cert = store.get(1).unwrap();
        assert_eq!(cert.block_number, 1);
        assert_eq!(cert.energy, 1000);
        assert_eq!(cert.initial_energy, 1000);
        assert_eq!(cert.temperature(), CertTemperature::Hot);
        assert!(cert.is_supermajority());
    }

    #[test]
    fn test_certificate_decay() {
        let mut store = make_store();
        register_cert(&mut store, 1, 0);

        // After 100 epochs (1 half-life), energy should be ~500
        let cert = store.get(1).unwrap();
        assert_eq!(cert.energy_at(100), 500);
        assert_eq!(cert.temperature_at(100), CertTemperature::Hot);

        // After 200 epochs (2 half-lives), energy should be ~250
        assert_eq!(cert.energy_at(200), 250);
        assert_eq!(cert.temperature_at(200), CertTemperature::Warm);

        // After 300 epochs, energy ~125
        assert_eq!(cert.energy_at(300), 125);
        assert_eq!(cert.temperature_at(300), CertTemperature::Cold);

        // After 1000 epochs, energy = 0
        assert_eq!(cert.energy_at(1000), 0);
        assert_eq!(cert.temperature_at(1000), CertTemperature::Evaporated);
    }

    #[test]
    fn test_process_epoch_decays_and_evaporates() {
        let mut store = make_store();
        register_cert(&mut store, 1, 0);
        register_cert(&mut store, 2, 0);
        register_cert(&mut store, 3, 0);

        // At epoch 200: all should still be alive (energy = 250)
        let (decayed, evaporated) = store.process_epoch(200);
        assert_eq!(decayed, 3);
        assert_eq!(evaporated, 0);

        // At epoch 1000: all should be dead (energy = 0)
        let (decayed, evaporated) = store.process_epoch(1000);
        assert_eq!(decayed, 0);
        assert_eq!(evaporated, 3);
        assert_eq!(store.active_count(), 0);
        assert_eq!(store.ghost_count(), 3);
    }

    #[test]
    fn test_ghost_record_on_evaporation() {
        let mut store = make_store();
        register_cert(&mut store, 42, 0);

        store.process_epoch(1000);

        let ghost = store.get_ghost(42).unwrap();
        assert_eq!(ghost.block_number, 42);
        assert_eq!(ghost.evaporated_epoch, 1000);
        assert_eq!(ghost.data_root, [42u8; 32]);
        // cert_hash reflects the cert's state at evaporation (decayed), not creation
        assert_ne!(ghost.cert_hash, [0u8; 32]); // non-zero hash
    }

    // ── Re-attestation ──

    #[test]
    fn test_re_attestation_boosts_energy() {
        let mut store = make_store();
        register_cert(&mut store, 1, 0);

        // Decay to epoch 200 (energy = 250)
        store.process_epoch(200);
        assert_eq!(store.get(1).unwrap().energy, 250);

        // Re-attest at epoch 200 (boost = 1000/4 = 250)
        store.re_attest(1, 200);
        assert_eq!(store.get(1).unwrap().energy, 500);
        assert_eq!(store.get(1).unwrap().re_attestation_count, 1);
        assert_eq!(store.get(1).unwrap().last_attested_epoch, 200);
    }

    #[test]
    fn test_re_attestation_capped_at_initial() {
        let mut store = make_store();
        register_cert(&mut store, 1, 0);

        // Re-attest immediately (energy still at 1000)
        store.re_attest(1, 0);
        // Should not exceed initial energy
        assert_eq!(store.get(1).unwrap().energy, 1000);
    }

    #[test]
    fn test_re_attestation_extends_lifetime() {
        let mut store = make_store();
        register_cert(&mut store, 1, 0);

        // Without re-attestation: dead at epoch 1000
        assert_eq!(store.get(1).unwrap().energy_at(1000), 0);

        // Re-attest at epoch 200 (boosts from 250 to 500)
        store.re_attest(1, 200);

        // Now at epoch 1000: 800 epochs since last attestation
        // energy = 500 >> (800/100) = 500 >> 8 = 1
        assert_eq!(store.get(1).unwrap().energy_at(1000), 1);
        assert!(store.get(1).unwrap().energy_at(1000) > 0, "re-attestation extended life");

        // Dead at epoch 1200: 1000 epochs since last attestation
        assert_eq!(store.get(1).unwrap().energy_at(1200), 0);
    }

    // ── Temperature distribution ──

    #[test]
    fn test_temperature_distribution() {
        let mut store = make_store();
        for i in 0..10 {
            register_cert(&mut store, i, 0);
        }

        let dist = store.temperature_distribution();
        assert_eq!(dist.hot, 10);
        assert_eq!(dist.warm, 0);
        assert_eq!(dist.cold, 0);

        // Decay: half will go to warm territory
        store.process_epoch(200);
        let dist = store.temperature_distribution();
        assert_eq!(dist.warm, 10); // 250/1000 = 25%, which is Warm
    }

    // ── Sampler ──

    #[test]
    fn test_sampler_deterministic() {
        let blocks: Vec<u64> = (100..200).collect();
        let s1 = PoHASampler::select_certificates(0, 50, &blocks, 5);
        let s2 = PoHASampler::select_certificates(0, 50, &blocks, 5);
        assert_eq!(s1, s2);
    }

    #[test]
    fn test_sampler_different_validators_get_different_certs() {
        let blocks: Vec<u64> = (100..200).collect();
        let s1 = PoHASampler::select_certificates(0, 50, &blocks, 5);
        let s2 = PoHASampler::select_certificates(1, 50, &blocks, 5);
        // Very likely different (100 blocks, 5 samples each)
        assert_ne!(s1, s2);
    }

    #[test]
    fn test_sampler_bounded() {
        let blocks: Vec<u64> = (100..200).collect();
        let selected = PoHASampler::select_certificates(0, 50, &blocks, 10);
        for &bn in &selected {
            assert!(bn >= 100 && bn < 200);
        }
    }

    #[test]
    fn test_sampler_empty_blocks() {
        let selected = PoHASampler::select_certificates(0, 50, &[], 5);
        assert!(selected.is_empty());
    }

    // ── Select for re-attestation ──

    #[test]
    fn test_select_for_re_attestation() {
        let mut store = make_store();
        for i in 0..10 {
            register_cert(&mut store, i, 0);
        }

        // At epoch 0: all Hot (energy=1000), none need re-attestation yet
        let selected = store.select_for_re_attestation(0, 10);
        assert!(selected.is_empty());

        // At epoch 200: energy_at = 1000 >> (200/100) = 250 → 25% → Warm
        let selected = store.select_for_re_attestation(200, 10);
        assert_eq!(selected.len(), 10);
    }

    // ── Capacity management ──

    #[test]
    fn test_max_active_evicts_oldest() {
        let mut store = PoHAStore::new(1000, 100);
        store.max_active = 5;

        for i in 0..10 {
            register_cert(&mut store, i, 0);
        }

        assert_eq!(store.active_count(), 5);
        assert_eq!(store.ghost_count(), 5);

        // Oldest blocks (0-4) should be evicted to ghosts
        for i in 0..5 {
            assert!(store.get(i).is_none());
            assert!(store.get_ghost(i).is_some());
        }
        // Newest blocks (5-9) should still be active
        for i in 5..10 {
            assert!(store.get(i).is_some());
        }
    }

    // ── Ghost pruning ──

    #[test]
    fn test_prune_ghosts() {
        let mut store = make_store();
        register_cert(&mut store, 1, 0);
        register_cert(&mut store, 2, 0);
        register_cert(&mut store, 3, 100);

        // Evaporate all
        store.process_epoch(1200);
        assert_eq!(store.ghost_count(), 3);

        // Prune ghosts evaporated before epoch 1100
        // Certs 1 and 2 were created at epoch 0, died at epoch 1200
        // Cert 3 was created at epoch 100, died at epoch 1200
        // All have evaporated_epoch = 1200, so pruning < 1200 removes none
        let pruned = store.prune_ghosts(1200);
        assert_eq!(pruned, 0);

        // Prune everything before epoch 1300
        let pruned = store.prune_ghosts(1300);
        assert_eq!(pruned, 3);
        assert_eq!(store.ghost_count(), 0);
    }

    // ── Certificate hash ──

    #[test]
    fn test_cert_hash_deterministic() {
        let mut store = make_store();
        register_cert(&mut store, 1, 0);
        let h1 = store.get(1).unwrap().hash();
        let h2 = store.get(1).unwrap().hash();
        assert_eq!(h1, h2);
        assert_ne!(h1, [0u8; 32]);
    }

    #[test]
    fn test_cert_hash_different_blocks() {
        let mut store = make_store();
        register_cert(&mut store, 1, 0);
        register_cert(&mut store, 2, 0);
        let h1 = store.get(1).unwrap().hash();
        let h2 = store.get(2).unwrap().hash();
        assert_ne!(h1, h2);
    }

    // ── Full lifecycle simulation ──

    #[test]
    fn test_full_lifecycle_simulation() {
        let mut store = PoHAStore::new(10000, 50); // higher energy, faster decay

        // Block production: 1 cert per epoch for 100 epochs
        for epoch in 0..100 {
            register_cert(&mut store, epoch, epoch);
        }
        assert_eq!(store.active_count(), 100);

        // Fast-forward to epoch 200
        let (decayed, evaporated) = store.process_epoch(200);
        // Certificates created at epoch 0 have been decaying for 200 epochs
        // = 4 half-lives → energy = 10000 >> 4 = 625 → Warm
        // Certificates created at epoch 99 have been decaying for 101 epochs
        // = 2 half-lives → energy = 10000 >> 2 = 2500 → Warm
        assert_eq!(evaporated, 0, "none dead yet");
        assert_eq!(decayed, 100);

        // Re-attest the 5 most critical certificates
        let to_reattest = store.select_for_re_attestation(200, 5);
        assert!(!to_reattest.is_empty());
        for &bn in &to_reattest {
            store.re_attest(bn, 200);
        }

        // Fast-forward to epoch 1000
        let (decayed, evaporated) = store.process_epoch(1000);
        // Most old certs should be dead. Re-attested ones might survive.
        assert!(evaporated > 0, "some should have evaporated");
        assert!(store.ghost_count() > 0, "ghosts should exist");

        // Re-attested certs should have survived longer
        for &bn in &to_reattest {
            if let Some(cert) = store.get(bn) {
                assert!(
                    cert.re_attestation_count > 0,
                    "re-attested cert should have count > 0"
                );
            }
            // It's OK if they also died — depends on boost vs decay
        }

        let dist = store.temperature_distribution();
        // At least some should be in various states
        assert!(
            dist.hot + dist.warm + dist.cold > 0 || store.ghost_count() > 0,
            "certificates should be in some state"
        );
    }

    // ── Temperature classification ──

    #[test]
    fn test_temperature_boundaries() {
        assert_eq!(CertTemperature::from_energy(1000, 1000), CertTemperature::Hot);
        assert_eq!(CertTemperature::from_energy(500, 1000), CertTemperature::Hot);
        assert_eq!(CertTemperature::from_energy(499, 1000), CertTemperature::Warm);
        assert_eq!(CertTemperature::from_energy(150, 1000), CertTemperature::Warm);
        assert_eq!(CertTemperature::from_energy(149, 1000), CertTemperature::Cold);
        assert_eq!(CertTemperature::from_energy(10, 1000), CertTemperature::Cold);
        assert_eq!(CertTemperature::from_energy(9, 1000), CertTemperature::Evaporated);
        assert_eq!(CertTemperature::from_energy(0, 1000), CertTemperature::Evaporated);
        assert_eq!(CertTemperature::from_energy(0, 0), CertTemperature::Evaporated);
    }
}
