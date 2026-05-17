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

// ─────────────────────── DST prefixes ───────────────────────────────────
//
// NN3 (re-audit 2026-05-14): workspace convention is
// `evaporchain:<protocol>:<role>:v1\0`. Pre-NN3 PoHA had no DST on
// either the cert hash or the re-attestation signing message — a
// cross-protocol BLS replay could in principle map a re-attestation
// signature onto an unrelated EvaporChain message that happened to
// share the 52-byte shape.

/// DST for `PoHACertificate::hash` — versioned so a future schema
/// bump (e.g. adding a new field) bumps the suffix to `v2\0` and
/// old/new hashes don't collide.
pub const POHA_CERT_HASH_DST: &[u8] = b"evaporchain:poha:cert:v1\0";
/// DST for `ReAttestation::sign_message` — same scheme.
pub const POHA_REATTEST_DST: &[u8] = b"evaporchain:poha:reattest:v1\0";

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
    ///
    /// Routes through `evaporchain_types::energy_at_epoch` — the
    /// canonical Coq-verified decay (`research/coq/EnergyDecayMonotonicity.v`).
    /// Layer 0 unification: replaces a raw `>> shifts` impl that
    /// lacked the fractional-decay correction between halvings, so
    /// PoHA cooling now matches the kernel's audited model.
    pub fn energy_at(&self, epoch: u64) -> u64 {
        let elapsed = epoch.saturating_sub(self.created_epoch);
        evaporchain_types::energy_at_epoch(self.energy, self.half_life, elapsed)
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
        self.energy = self
            .energy
            .saturating_add(energy_boost)
            .min(self.initial_energy);
        self.last_attested_epoch = epoch;
        self.re_attestation_count += 1;
    }

    /// Check if this certificate has strictly more than 2/3 attestation.
    ///
    /// Q4 (audit 2026-05-17): strict `>` enforces Tendermint safety.
    /// Pre-fix `>=` collapsed to exactly `2T/3` when divisible; Byzantine
    /// T/3 actor could split-quorum two blocks at same height.
    pub fn is_supermajority(&self) -> bool {
        // Audit C2: u128 to prevent wrap when stake > u64::MAX/3.
        (self.attested_stake as u128) * 3 > (self.total_stake as u128) * 2
    }

    /// Compute a 32-byte hash of this certificate for compact storage.
    ///
    /// NN3 (re-audit 2026-05-14): pre-fix this hash omitted
    /// `total_stake`, `signer_ids`, `last_attested_epoch`, and
    /// `re_attestation_count`, and lacked a DST prefix.
    ///
    /// **Forgery surface (pre-fix):** omitting `total_stake` while
    /// including `attested_stake` made `is_supermajority`
    /// (`attested * 3 ≥ total * 2`) unverifiable from the hash. An
    /// attacker could craft a fake cert with `attested = K,
    /// total = 2K` (NOT supermajority) that hash-matched a real cert
    /// with `attested = K, total = K` (supermajority). Omitting
    /// `signer_ids` likewise let a fake cert with different signers
    /// produce the same hash — accountability loss for slashing /
    /// re-attestation. The missing DST (M8 closed it in the audit
    /// but the doc-level closure shipped only the `cert_hash` field,
    /// not the prefix) put PoHA in the cross-protocol BLS replay
    /// surface.
    ///
    /// Post-fix:
    /// - All 11 security-critical fields covered.
    /// - DST prefix `POHA_CERT_HASH_DST` per workspace convention.
    /// - `signer_ids` length-prefixed so two certs with different
    ///   numbers of signers can't be made to collide via padding.
    ///
    /// **Hard fork.** Any persisted cert hash from before this fix
    /// will not match its post-fix recomputation. The chain's
    /// production storage references cert_hash in CertGhost +
    /// ReAttestation only; a migration path is to recompute on
    /// next attestation rather than rewrite ghosts in place.
    pub fn hash(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(POHA_CERT_HASH_DST);
        hasher.update(&self.block_number.to_le_bytes());
        hasher.update(&self.data_root);
        hasher.update(&self.shard_count.to_le_bytes());
        hasher.update(&self.initial_energy.to_le_bytes());
        hasher.update(&self.half_life.to_le_bytes());
        hasher.update(&self.created_epoch.to_le_bytes());
        hasher.update(&self.last_attested_epoch.to_le_bytes());
        hasher.update(&self.attested_stake.to_le_bytes());
        hasher.update(&self.total_stake.to_le_bytes());
        hasher.update(&self.re_attestation_count.to_le_bytes());
        hasher.update(&self.aggregate_signature);
        // signer_ids: length-prefix the count so two certs with
        // different signer-list sizes can't collide via padding.
        hasher.update(&(self.signer_ids.len() as u64).to_le_bytes());
        for id in &self.signer_ids {
            hasher.update(&id.to_le_bytes());
        }
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
    ///
    /// NN3 (re-audit 2026-05-14): now prefixed with
    /// `POHA_REATTEST_DST` so the signature can never be
    /// confused with another EvaporChain message of the same
    /// 52-byte shape (cross-protocol BLS replay defense). M8
    /// closed this in the audit doc at the field level (added
    /// the `cert_hash` reference) but never landed the DST
    /// prefix — NN3 finds the gap.
    pub fn sign_message(
        cert_hash: &[u8; 32],
        epoch: u64,
        validator_id: u64,
        shards_held: u32,
    ) -> Vec<u8> {
        let mut msg = Vec::with_capacity(POHA_REATTEST_DST.len() + 52);
        msg.extend_from_slice(POHA_REATTEST_DST);
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
    /// Most recent epoch a sampler tick fired (for cadence enforcement).
    /// `None` means "never fired" — first call always fires.
    last_sampler_epoch: Option<u64>,
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
            last_sampler_epoch: None,
        }
    }

    /// Epoch-cadence re-attestation sampler. Picks up to `sample_size`
    /// certificates ranked by lowest energy (i.e. most in need) via
    /// [`select_for_re_attestation`] and re-attests them, boosting
    /// their energy by `re_attestation_boost`.
    ///
    /// Cadence: only fires when `epoch >= last_sampler_epoch + interval_epochs`.
    /// Returns the number of certificates actually re-attested this tick
    /// (zero if cadence not met or no candidates).
    ///
    /// Wires Phase-1 of the PoHA temperature gradient (Hot / Warm / Cold /
    /// Evaporated) — without periodic re-attestation, all certificates
    /// decay uniformly and the gradient collapses. Selection is energy-
    /// prioritized rather than RNG-randomized; the term "sampler" is kept
    /// for parity with the proposal vocabulary, and a future iteration
    /// can mix in stake-weighted RNG without API churn.
    pub fn tick_re_attestation_sampler(
        &mut self,
        epoch: u64,
        sample_size: usize,
        interval_epochs: u64,
    ) -> usize {
        if let Some(last) = self.last_sampler_epoch {
            if epoch < last.saturating_add(interval_epochs) {
                return 0;
            }
        }
        let candidates = self.select_for_re_attestation(epoch, sample_size);
        let mut count = 0;
        for bn in candidates {
            if self.re_attest(bn, epoch) {
                count += 1;
            }
        }
        if count > 0 {
            self.last_sampler_epoch = Some(epoch);
        }
        count
    }

    /// Register a new DA certificate as a PoHA certificate.
    #[allow(clippy::too_many_arguments)]
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

        // Evict oldest if over capacity.  Skip eviction if the oldest key
        // IS the cert we just inserted — this happens when an old block's
        // DA cert arrives during sync backfill while the table is already
        // full of newer certs.  Temporarily exceeding max_active by 1 is
        // harmless; the next register() call will evict it.
        while self.certificates.len() > self.max_active {
            match self.certificates.keys().next().copied() {
                Some(oldest) if oldest == block_number => break,
                Some(oldest) => {
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
                None => break,
            }
        }

        // The cert must be present: the guard above ensures we never evict
        // the block we just inserted.
        self.certificates
            .get(&block_number)
            .expect("PoHA cert vanished immediately after insert — logic error")
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

    /// Get all ghost records as an iterator.
    pub fn all_ghosts(&self) -> impl Iterator<Item = (&u64, &CertGhost)> {
        self.ghosts.iter()
    }

    /// Insert a pre-built certificate (for persistence restore).
    pub fn insert_certificate(&mut self, cert: PoHACertificate) {
        self.certificates.insert(cert.block_number, cert);
    }

    /// Insert a pre-built ghost record (for persistence restore).
    pub fn insert_ghost(&mut self, ghost: CertGhost) {
        self.ghosts.insert(ghost.block_number, ghost);
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
            let idx = (u64::from_le_bytes(bytes[0..8].try_into().unwrap()) as usize)
                % active_block_numbers.len();
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

        // At epoch 1000: all should be dead (energy = 0). The test
        // previously bound the second tuple slot as `_decayed`, leaving
        // the assertion below to check the FIRST call's `decayed=3`
        // against `0`. Re-bind so we're asserting the post-evaporation
        // state, which is the actual contract being tested.
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
        assert!(
            store.get(1).unwrap().energy_at(1000) > 0,
            "re-attestation extended life"
        );

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
            assert!((100..200).contains(&bn));
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
        let (_decayed, evaporated) = store.process_epoch(1000);
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
        assert_eq!(
            CertTemperature::from_energy(1000, 1000),
            CertTemperature::Hot
        );
        assert_eq!(
            CertTemperature::from_energy(500, 1000),
            CertTemperature::Hot
        );
        assert_eq!(
            CertTemperature::from_energy(499, 1000),
            CertTemperature::Warm
        );
        assert_eq!(
            CertTemperature::from_energy(150, 1000),
            CertTemperature::Warm
        );
        assert_eq!(
            CertTemperature::from_energy(149, 1000),
            CertTemperature::Cold
        );
        assert_eq!(
            CertTemperature::from_energy(10, 1000),
            CertTemperature::Cold
        );
        assert_eq!(
            CertTemperature::from_energy(9, 1000),
            CertTemperature::Evaporated
        );
        assert_eq!(
            CertTemperature::from_energy(0, 1000),
            CertTemperature::Evaporated
        );
        assert_eq!(
            CertTemperature::from_energy(0, 0),
            CertTemperature::Evaporated
        );
    }

    #[test]
    fn tick_re_attestation_respects_cadence() {
        // Fast decay (half_life=10) so certs slip into Warm/Cold ranges
        // within a small epoch window (decay is integer bit-shift on
        // elapsed/half_life, so we need at least one full half-life).
        let mut store = PoHAStore::new(1000, 10);
        register_cert(&mut store, 1, 0);
        register_cert(&mut store, 2, 0);
        register_cert(&mut store, 3, 0);

        // 30 epochs = 3 half-lives → energy = 1000 >> 3 = 125 (Cold).
        store.process_epoch(30);

        // First tick at epoch 30, interval = 50, sample = 2. Should fire.
        let n1 = store.tick_re_attestation_sampler(30, 2, 50);
        assert!(
            n1 > 0,
            "first tick at epoch 30 must re-attest at least one cert"
        );

        // Immediate second tick at epoch 50 should be SUPPRESSED by cadence
        // (last fired at 30, interval 50 → next allowed at 80).
        let n2 = store.tick_re_attestation_sampler(50, 2, 50);
        assert_eq!(n2, 0, "tick must respect interval_epochs cadence");

        // Past the cadence boundary (last fired at 30, interval 50
        // → next allowed at 80), tick fires again. Use epoch 80
        // exactly — under the canonical `energy_at_epoch` decay,
        // running to epoch 85 (5 epochs into a half-life) shaves the
        // boosted-cert energy from 11 to 9, which crosses the 1%
        // ratio_pct floor and reclassifies the cert as Evaporated
        // (ineligible for re-attestation). At epoch 80 the
        // remainder is 0 so canonical and rogue agree.
        store.process_epoch(80);
        let n3 = store.tick_re_attestation_sampler(80, 2, 50);
        assert!(n3 > 0, "tick must fire again after interval elapsed");
    }

    #[test]
    fn tick_re_attestation_zero_when_no_candidates() {
        let mut store = PoHAStore::new(1000, 100);
        register_cert(&mut store, 1, 0);
        // Cert is Hot at epoch 0 (full energy) — not a re-attestation candidate.
        let n = store.tick_re_attestation_sampler(0, 5, 1);
        assert_eq!(n, 0);
    }

    /// T1.20 — `re_attest` returns `false` when the block_number is
    /// not in the store (lines 384-388). Adversarial: a malicious or
    /// stale re-attestation message must NOT silently create a new
    /// cert or extend a non-existent one.
    #[test]
    fn t1_20_re_attest_returns_false_on_missing_cert() {
        let mut store = PoHAStore::new(1000, 100);
        register_cert(&mut store, 1, 0);
        // Block 999 was never registered.
        assert!(
            !store.re_attest(999, 10),
            "re_attest on unknown block must return false"
        );
        // Active count unchanged.
        assert_eq!(store.active_count(), 1);
        // Original cert untouched (re_attestation_count still 0).
        assert_eq!(store.get(1).unwrap().re_attestation_count, 0);
    }

    /// T1.20 — `get` / `get_ghost` return `None` for unknown block
    /// numbers (lines 421-423, 426-428). Boundary pin that prevents a
    /// silent default-cert leak if a future refactor switched to
    /// `entry().or_insert_with(...)` semantics.
    #[test]
    fn t1_20_get_and_get_ghost_return_none_for_missing() {
        let store = PoHAStore::new(1000, 100);
        assert!(store.get(42).is_none());
        assert!(store.get_ghost(42).is_none());
        // Same after some registrations.
        let mut store = store;
        register_cert(&mut store, 1, 0);
        assert!(store.get(2).is_none(), "registered 1, not 2");
        assert!(store.get_ghost(1).is_none(), "1 is active, not ghost");
    }

    /// T1.20 — `register` with the same `block_number` REPLACES the
    /// existing entry (line 313: `BTreeMap::insert` overwrites).
    /// Doctrine pin: the second register resets energy to
    /// `default_energy` and clears `re_attestation_count` — registering
    /// is NOT a no-op. Any future change to refuse-on-duplicate would
    /// trip this test, surfacing the semantics break before it ships.
    #[test]
    fn t1_20_register_overwrites_existing_block_number() {
        let mut store = PoHAStore::new(1000, 100);
        register_cert(&mut store, 1, 0);
        // Boost the first cert.
        assert!(store.re_attest(1, 5));
        let after_boost = store.get(1).unwrap().re_attestation_count;
        assert_eq!(after_boost, 1);

        // Re-register the same block — overwrites the old cert.
        register_cert(&mut store, 1, 10);
        let fresh = store.get(1).unwrap();
        assert_eq!(
            fresh.re_attestation_count, 0,
            "re-register must reset re_attestation_count"
        );
        assert_eq!(
            fresh.energy, 1000,
            "re-register must reset energy to default"
        );
        assert_eq!(
            fresh.created_epoch, 10,
            "re-register must capture new created_epoch"
        );
        // Active count stays 1 (overwrite, not duplicate).
        assert_eq!(store.active_count(), 1);
    }

    // ── NN3 (re-audit 2026-05-14): cert_hash field omission + DST ──

    /// Construct a minimal `PoHACertificate` directly for the
    /// per-field hash regressions below. Bypasses the PoHAStore so
    /// we can poke individual fields to construct the forgery
    /// pairs the audit identified.
    fn cert_for_dst_test() -> PoHACertificate {
        PoHACertificate {
            block_number: 42,
            data_root: [0xAA; 32],
            shard_count: 8,
            initial_energy: 1000,
            energy: 800,
            half_life: 100,
            created_epoch: 5,
            last_attested_epoch: 5,
            attested_stake: 1_000_000,
            total_stake: 1_000_000,
            re_attestation_count: 0,
            aggregate_signature: vec![0xBB; 96],
            signer_ids: vec![1, 2, 3],
        }
    }

    /// Pre-fix this assertion would have FAILED: omitting
    /// `total_stake` from the hash meant a cert with
    /// `attested=K, total=2K` (not supermajority) hash-matched a
    /// cert with `attested=K, total=K` (supermajority). Post-fix
    /// the same `attested` but different `total` produces a
    /// different hash.
    #[test]
    fn audit_nn3_hash_includes_total_stake() {
        let a = cert_for_dst_test();
        let mut b = a.clone();
        b.total_stake = 2_000_000;
        assert_ne!(
            a.hash(),
            b.hash(),
            "NN3: cert_hash MUST include total_stake (supermajority forgery defense)"
        );
    }

    #[test]
    fn audit_nn3_hash_includes_signer_ids() {
        let a = cert_for_dst_test();
        let mut b = a.clone();
        b.signer_ids = vec![1, 2, 4]; // same length, different members
        assert_ne!(
            a.hash(),
            b.hash(),
            "NN3: cert_hash MUST include signer_ids (accountability)"
        );

        // Different length too.
        let mut c = a.clone();
        c.signer_ids = vec![1, 2, 3, 4];
        assert_ne!(a.hash(), c.hash());
    }

    #[test]
    fn audit_nn3_hash_includes_last_attested_epoch() {
        let a = cert_for_dst_test();
        let mut b = a.clone();
        b.last_attested_epoch = 42;
        assert_ne!(a.hash(), b.hash());
    }

    #[test]
    fn audit_nn3_hash_includes_re_attestation_count() {
        let a = cert_for_dst_test();
        let mut b = a.clone();
        b.re_attestation_count = 7;
        assert_ne!(a.hash(), b.hash());
    }

    /// Hash diverges from the bare blake3 of the legacy field set —
    /// the DST prefix changes the output deterministically.
    #[test]
    fn audit_nn3_hash_uses_dst_prefix() {
        let cert = cert_for_dst_test();

        // Recompute what the pre-fix hash would have been (no DST,
        // 7 fields only).
        let mut pre_fix = blake3::Hasher::new();
        pre_fix.update(&cert.block_number.to_le_bytes());
        pre_fix.update(&cert.data_root);
        pre_fix.update(&cert.shard_count.to_le_bytes());
        pre_fix.update(&cert.initial_energy.to_le_bytes());
        pre_fix.update(&cert.half_life.to_le_bytes());
        pre_fix.update(&cert.created_epoch.to_le_bytes());
        pre_fix.update(&cert.attested_stake.to_le_bytes());
        pre_fix.update(&cert.aggregate_signature);
        let pre_fix_hash: [u8; 32] = pre_fix.finalize().into();

        assert_ne!(
            cert.hash(),
            pre_fix_hash,
            "NN3: post-fix hash must differ from pre-fix (DST + new fields landed)"
        );
    }

    /// `ReAttestation::sign_message` now starts with the
    /// `POHA_REATTEST_DST` prefix.
    #[test]
    fn audit_nn3_reattest_message_uses_dst() {
        let cert_hash = [0xAB; 32];
        let msg = ReAttestation::sign_message(&cert_hash, 100, 7, 64);
        assert!(
            msg.starts_with(POHA_REATTEST_DST),
            "NN3: reattest sign_message must start with POHA_REATTEST_DST"
        );
        // And the trailing bytes are the same shape as pre-fix.
        let suffix = &msg[POHA_REATTEST_DST.len()..];
        assert_eq!(suffix.len(), 52, "32 + 8 + 8 + 4 = 52");
    }

    #[test]
    fn audit_nn3_cert_hash_remains_deterministic_under_fix() {
        let cert = cert_for_dst_test();
        assert_eq!(cert.hash(), cert.hash());
        assert_ne!(cert.hash(), [0u8; 32]);
    }
}
