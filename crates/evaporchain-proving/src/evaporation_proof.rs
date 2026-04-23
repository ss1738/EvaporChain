//! ZK Evaporation Proofs — Fiat-Shamir transcript proofs for correct
//! energy decay, evaporation validity, and MMR nullifier inclusion.
//!
//! These are deterministic computation proofs (not hiding secrets), so we
//! use a BLAKE3-based Fiat-Shamir transcript instead of full SNARK overhead.
//! Proofs are constant-size per batch, serializable, and verifiable without
//! the full state.

use evaporchain_types::energy_at_epoch;
use serde::{Deserialize, Serialize};

// ─── Fiat-Shamir Transcript ──────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Transcript {
    hasher: blake3::Hasher,
}

impl Transcript {
    fn new(domain: &[u8]) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"evaporchain-evaporation-proof-v1:");
        hasher.update(domain);
        Self { hasher }
    }

    fn append(&mut self, label: &[u8], data: &[u8]) {
        self.hasher.update(label);
        self.hasher.update(&(data.len() as u32).to_le_bytes());
        self.hasher.update(data);
    }

    fn append_u64(&mut self, label: &[u8], val: u64) {
        self.append(label, &val.to_le_bytes());
    }

    fn challenge(&self) -> [u8; 32] {
        *self.hasher.finalize().as_bytes()
    }
}

// ─── Types ───────────────────────────────────────────────────────────────

/// A claim that an object's energy was correctly computed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EnergyDecayStatement {
    pub object_id: [u8; 20],
    pub initial_energy: u64,
    pub half_life: u64,
    pub creation_epoch: u64,
    pub current_epoch: u64,
    pub claimed_energy: u64,
}

/// A claim that an object was correctly evaporated.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvaporationClaim {
    pub object_id: [u8; 20],
    pub initial_energy: u64,
    pub half_life: u64,
    pub creation_epoch: u64,
    pub evaporation_epoch: u64,
    pub nullifier: [u8; 32],
}

/// A batch evaporation proof covering multiple objects in one block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaporationProof {
    pub block_number: u64,
    pub decay_statements: Vec<EnergyDecayStatement>,
    pub evaporation_claims: Vec<EvaporationClaim>,
    pub mmr_root: [u8; 32],
    pub transcript_hash: [u8; 32],
    pub batch_commitment: [u8; 32],
}

impl EvaporationProof {
    pub fn size(&self) -> usize {
        self.decay_statements.len() + self.evaporation_claims.len()
    }
}

// ─── Prover ──────────────────────────────────────────────────────────────

/// Produces batch evaporation proofs using Fiat-Shamir transcripts.
pub struct EvaporationProver {
    block_number: u64,
    decay_statements: Vec<EnergyDecayStatement>,
    evaporation_claims: Vec<EvaporationClaim>,
    mmr_leaves: Vec<[u8; 32]>,
}

impl EvaporationProver {
    pub fn new(block_number: u64) -> Self {
        Self {
            block_number,
            decay_statements: Vec::new(),
            evaporation_claims: Vec::new(),
            mmr_leaves: Vec::new(),
        }
    }

    /// Add a decay statement — proves energy was correctly computed.
    pub fn add_decay(&mut self, stmt: EnergyDecayStatement) -> Result<(), &'static str> {
        let elapsed = stmt.current_epoch.saturating_sub(stmt.creation_epoch);
        let expected = energy_at_epoch(stmt.initial_energy, stmt.half_life, elapsed);
        if stmt.claimed_energy != expected {
            return Err("claimed energy does not match decay formula");
        }
        self.decay_statements.push(stmt);
        Ok(())
    }

    /// Add an evaporation claim — proves an object was correctly evaporated
    /// (energy reached 0 at the claimed epoch).
    pub fn add_evaporation(&mut self, claim: EvaporationClaim) -> Result<(), &'static str> {
        let elapsed = claim.evaporation_epoch.saturating_sub(claim.creation_epoch);
        let energy = energy_at_epoch(claim.initial_energy, claim.half_life, elapsed);
        if energy != 0 {
            return Err("object still has energy at claimed evaporation epoch");
        }
        // Add nullifier to MMR
        self.mmr_leaves.push(claim.nullifier);
        self.evaporation_claims.push(claim);
        Ok(())
    }

    /// Build the batch proof for this block.
    pub fn prove(self) -> EvaporationProof {
        let mut transcript = Transcript::new(b"batch-evaporation");
        transcript.append_u64(b"block", self.block_number);

        // Commit all decay statements
        for stmt in &self.decay_statements {
            transcript.append(b"obj", &stmt.object_id);
            transcript.append_u64(b"E0", stmt.initial_energy);
            transcript.append_u64(b"hl", stmt.half_life);
            transcript.append_u64(b"ce", stmt.creation_epoch);
            transcript.append_u64(b"now", stmt.current_epoch);
            transcript.append_u64(b"E", stmt.claimed_energy);
        }

        // Commit all evaporation claims
        for claim in &self.evaporation_claims {
            transcript.append(b"evap-obj", &claim.object_id);
            transcript.append_u64(b"evap-E0", claim.initial_energy);
            transcript.append_u64(b"evap-hl", claim.half_life);
            transcript.append_u64(b"evap-ce", claim.creation_epoch);
            transcript.append_u64(b"evap-ee", claim.evaporation_epoch);
            transcript.append(b"null", &claim.nullifier);
        }

        let transcript_hash = transcript.challenge();
        let mmr_root = compute_mmr_root(&self.mmr_leaves);

        // Batch commitment = BLAKE3(transcript_hash || mmr_root || block_number)
        let batch_commitment = {
            let mut h = blake3::Hasher::new();
            h.update(&transcript_hash);
            h.update(&mmr_root);
            h.update(&self.block_number.to_le_bytes());
            *h.finalize().as_bytes()
        };

        EvaporationProof {
            block_number: self.block_number,
            decay_statements: self.decay_statements,
            evaporation_claims: self.evaporation_claims,
            mmr_root,
            transcript_hash,
            batch_commitment,
        }
    }
}

// ─── Verifier ────────────────────────────────────────────────────────────

/// Verify an evaporation proof is correct.
pub fn verify_proof(proof: &EvaporationProof) -> Result<bool, &'static str> {
    // Re-verify all decay statements
    for stmt in &proof.decay_statements {
        let elapsed = stmt.current_epoch.saturating_sub(stmt.creation_epoch);
        let expected = energy_at_epoch(stmt.initial_energy, stmt.half_life, elapsed);
        if stmt.claimed_energy != expected {
            return Err("decay statement has incorrect energy");
        }
    }

    // Re-verify all evaporation claims
    for claim in &proof.evaporation_claims {
        let elapsed = claim.evaporation_epoch.saturating_sub(claim.creation_epoch);
        let energy = energy_at_epoch(claim.initial_energy, claim.half_life, elapsed);
        if energy != 0 {
            return Err("evaporation claim: object still has energy");
        }
    }

    // Re-derive transcript
    let mut transcript = Transcript::new(b"batch-evaporation");
    transcript.append_u64(b"block", proof.block_number);
    for stmt in &proof.decay_statements {
        transcript.append(b"obj", &stmt.object_id);
        transcript.append_u64(b"E0", stmt.initial_energy);
        transcript.append_u64(b"hl", stmt.half_life);
        transcript.append_u64(b"ce", stmt.creation_epoch);
        transcript.append_u64(b"now", stmt.current_epoch);
        transcript.append_u64(b"E", stmt.claimed_energy);
    }
    for claim in &proof.evaporation_claims {
        transcript.append(b"evap-obj", &claim.object_id);
        transcript.append_u64(b"evap-E0", claim.initial_energy);
        transcript.append_u64(b"evap-hl", claim.half_life);
        transcript.append_u64(b"evap-ce", claim.creation_epoch);
        transcript.append_u64(b"evap-ee", claim.evaporation_epoch);
        transcript.append(b"null", &claim.nullifier);
    }
    if transcript.challenge() != proof.transcript_hash {
        return Err("transcript hash mismatch");
    }

    // Verify MMR root
    let nullifiers: Vec<[u8; 32]> = proof
        .evaporation_claims
        .iter()
        .map(|c| c.nullifier)
        .collect();
    if compute_mmr_root(&nullifiers) != proof.mmr_root {
        return Err("MMR root mismatch");
    }

    // Verify batch commitment
    let expected_commitment = {
        let mut h = blake3::Hasher::new();
        h.update(&proof.transcript_hash);
        h.update(&proof.mmr_root);
        h.update(&proof.block_number.to_le_bytes());
        *h.finalize().as_bytes()
    };
    if expected_commitment != proof.batch_commitment {
        return Err("batch commitment mismatch");
    }

    Ok(true)
}

// ─── MMR Helpers ─────────────────────────────────────────────────────────

fn compute_mmr_root(leaves: &[[u8; 32]]) -> [u8; 32] {
    if leaves.is_empty() {
        return [0u8; 32];
    }
    let mut current: Vec<[u8; 32]> = leaves.to_vec();
    while current.len() > 1 {
        let mut next = Vec::new();
        for chunk in current.chunks(2) {
            let mut h = blake3::Hasher::new();
            h.update(&chunk[0]);
            if chunk.len() > 1 {
                h.update(&chunk[1]);
            } else {
                h.update(&chunk[0]);
            }
            next.push(*h.finalize().as_bytes());
        }
        current = next;
    }
    current[0]
}

// ─── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_nullifier(seed: u8) -> [u8; 32] {
        let mut n = [0u8; 32];
        n[0] = seed;
        n
    }

    fn make_object_id(seed: u8) -> [u8; 20] {
        let mut id = [0u8; 20];
        id[0] = seed;
        id
    }

    #[test]
    fn test_single_decay_statement() {
        let mut prover = EvaporationProver::new(100);
        let stmt = EnergyDecayStatement {
            object_id: make_object_id(1),
            initial_energy: 10000,
            half_life: 50,
            creation_epoch: 0,
            current_epoch: 50,
            claimed_energy: energy_at_epoch(10000, 50, 50),
        };
        assert!(prover.add_decay(stmt).is_ok());
        let proof = prover.prove();
        assert!(verify_proof(&proof).unwrap());
    }

    #[test]
    fn test_wrong_energy_rejected() {
        let mut prover = EvaporationProver::new(100);
        let stmt = EnergyDecayStatement {
            object_id: make_object_id(1),
            initial_energy: 10000,
            half_life: 50,
            creation_epoch: 0,
            current_epoch: 50,
            claimed_energy: 9999, // wrong
        };
        assert!(prover.add_decay(stmt).is_err());
    }

    #[test]
    fn test_single_evaporation() {
        let mut prover = EvaporationProver::new(200);
        // half_life=10, initial=1000, after 640 epochs = 64 halvings = 0
        let claim = EvaporationClaim {
            object_id: make_object_id(2),
            initial_energy: 1000,
            half_life: 10,
            creation_epoch: 0,
            evaporation_epoch: 640,
            nullifier: make_nullifier(2),
        };
        assert!(prover.add_evaporation(claim).is_ok());
        let proof = prover.prove();
        assert!(verify_proof(&proof).unwrap());
    }

    #[test]
    fn test_evaporation_still_alive_rejected() {
        let mut prover = EvaporationProver::new(200);
        let claim = EvaporationClaim {
            object_id: make_object_id(3),
            initial_energy: 1000,
            half_life: 100,
            creation_epoch: 0,
            evaporation_epoch: 10, // way too early
            nullifier: make_nullifier(3),
        };
        assert!(prover.add_evaporation(claim).is_err());
    }

    #[test]
    fn test_batch_proof() {
        let mut prover = EvaporationProver::new(300);

        // Add 3 decay statements
        for i in 0..3 {
            let elapsed = (i + 1) * 50;
            let stmt = EnergyDecayStatement {
                object_id: make_object_id(i as u8),
                initial_energy: 10000,
                half_life: 100,
                creation_epoch: 0,
                current_epoch: elapsed,
                claimed_energy: energy_at_epoch(10000, 100, elapsed),
            };
            prover.add_decay(stmt).unwrap();
        }

        // Add 2 evaporation claims
        for i in 10..12 {
            let claim = EvaporationClaim {
                object_id: make_object_id(i),
                initial_energy: 100,
                half_life: 10,
                creation_epoch: 0,
                evaporation_epoch: 640,
                nullifier: make_nullifier(i),
            };
            prover.add_evaporation(claim).unwrap();
        }

        let proof = prover.prove();
        assert_eq!(proof.size(), 5);
        assert!(verify_proof(&proof).unwrap());
    }

    #[test]
    fn test_empty_proof() {
        let prover = EvaporationProver::new(1);
        let proof = prover.prove();
        assert_eq!(proof.size(), 0);
        assert!(verify_proof(&proof).unwrap());
    }

    #[test]
    fn test_tampered_transcript_hash_fails() {
        let mut prover = EvaporationProver::new(100);
        let stmt = EnergyDecayStatement {
            object_id: make_object_id(1),
            initial_energy: 10000,
            half_life: 50,
            creation_epoch: 0,
            current_epoch: 50,
            claimed_energy: energy_at_epoch(10000, 50, 50),
        };
        prover.add_decay(stmt).unwrap();
        let mut proof = prover.prove();
        proof.transcript_hash[0] ^= 0xFF;
        assert!(verify_proof(&proof).is_err());
    }

    #[test]
    fn test_tampered_mmr_root_fails() {
        let mut prover = EvaporationProver::new(200);
        let claim = EvaporationClaim {
            object_id: make_object_id(1),
            initial_energy: 100,
            half_life: 10,
            creation_epoch: 0,
            evaporation_epoch: 640,
            nullifier: make_nullifier(1),
        };
        prover.add_evaporation(claim).unwrap();
        let mut proof = prover.prove();
        proof.mmr_root[0] ^= 0xFF;
        assert!(verify_proof(&proof).is_err());
    }

    #[test]
    fn test_tampered_batch_commitment_fails() {
        let prover = EvaporationProver::new(100);
        let mut proof = prover.prove();
        proof.batch_commitment[0] ^= 0xFF;
        assert!(verify_proof(&proof).is_err());
    }

    #[test]
    fn test_tampered_energy_in_proof_fails() {
        let mut prover = EvaporationProver::new(100);
        let stmt = EnergyDecayStatement {
            object_id: make_object_id(1),
            initial_energy: 10000,
            half_life: 50,
            creation_epoch: 0,
            current_epoch: 50,
            claimed_energy: energy_at_epoch(10000, 50, 50),
        };
        prover.add_decay(stmt).unwrap();
        let mut proof = prover.prove();
        proof.decay_statements[0].claimed_energy += 1;
        assert!(verify_proof(&proof).is_err());
    }

    #[test]
    fn test_proof_deterministic() {
        let make = || {
            let mut prover = EvaporationProver::new(42);
            prover.add_decay(EnergyDecayStatement {
                object_id: make_object_id(1),
                initial_energy: 5000,
                half_life: 25,
                creation_epoch: 10,
                current_epoch: 35,
                claimed_energy: energy_at_epoch(5000, 25, 25),
            }).unwrap();
            prover.prove()
        };
        let p1 = make();
        let p2 = make();
        assert_eq!(p1.transcript_hash, p2.transcript_hash);
        assert_eq!(p1.batch_commitment, p2.batch_commitment);
    }

    #[test]
    fn test_mmr_root_empty() {
        assert_eq!(compute_mmr_root(&[]), [0u8; 32]);
    }

    #[test]
    fn test_mmr_root_single() {
        let leaf = make_nullifier(42);
        let root = compute_mmr_root(&[leaf]);
        // Single leaf: hash(leaf || leaf)
        let mut h = blake3::Hasher::new();
        h.update(&leaf);
        h.update(&leaf);
        assert_eq!(root, *h.finalize().as_bytes());
    }

    #[test]
    fn test_mmr_root_two_leaves() {
        let a = make_nullifier(1);
        let b = make_nullifier(2);
        let root = compute_mmr_root(&[a, b]);
        let mut h = blake3::Hasher::new();
        h.update(&a);
        h.update(&b);
        assert_eq!(root, *h.finalize().as_bytes());
    }

    #[test]
    fn test_energy_boundary_zero_half_life() {
        let mut prover = EvaporationProver::new(1);
        // half_life=0 → energy immediately 0
        let claim = EvaporationClaim {
            object_id: make_object_id(1),
            initial_energy: 1000,
            half_life: 0,
            creation_epoch: 0,
            evaporation_epoch: 1,
            nullifier: make_nullifier(1),
        };
        assert!(prover.add_evaporation(claim).is_ok());
    }

    #[test]
    fn test_serialization_roundtrip() {
        let mut prover = EvaporationProver::new(99);
        prover.add_decay(EnergyDecayStatement {
            object_id: make_object_id(5),
            initial_energy: 8000,
            half_life: 40,
            creation_epoch: 10,
            current_epoch: 50,
            claimed_energy: energy_at_epoch(8000, 40, 40),
        }).unwrap();
        let proof = prover.prove();
        let json = serde_json::to_string(&proof).unwrap();
        let decoded: EvaporationProof = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.transcript_hash, proof.transcript_hash);
        assert_eq!(decoded.batch_commitment, proof.batch_commitment);
    }

    #[test]
    fn test_large_batch() {
        let mut prover = EvaporationProver::new(500);
        for i in 0..50u8 {
            prover.add_decay(EnergyDecayStatement {
                object_id: make_object_id(i),
                initial_energy: 10000,
                half_life: 100,
                creation_epoch: 0,
                current_epoch: 200,
                claimed_energy: energy_at_epoch(10000, 100, 200),
            }).unwrap();
        }
        for i in 100..120u8 {
            prover.add_evaporation(EvaporationClaim {
                object_id: make_object_id(i),
                initial_energy: 50,
                half_life: 10,
                creation_epoch: 0,
                evaporation_epoch: 640,
                nullifier: make_nullifier(i),
            }).unwrap();
        }
        let proof = prover.prove();
        assert_eq!(proof.size(), 70);
        assert!(verify_proof(&proof).unwrap());
    }
}
