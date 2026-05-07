//! Fiat-Shamir transcript + query-position derivation.

use evaporchain_dfri::{EnergyCodeword, FieldElem};

pub const TRANSCRIPT_TAG: &[u8] = b"evaporchain:dfri-fs:transcript:v1\0";

/// FS transcript: a running BLAKE3 hasher seeded with the
/// domain-separation tag plus prover commitments. Calling
/// `challenge()` advances the state and returns 32 fresh bytes.
#[derive(Debug, Clone)]
pub struct FsTranscript {
    state: blake3::Hasher,
    counter: u64,
}

impl FsTranscript {
    /// Initialise from `(input_root, folded_root, domain_size,
    /// num_queries)`. Two verifiers that observe the same prover
    /// commitments derive identical challenges.
    pub fn new(
        input_root: &[u8; 32],
        folded_root: &[u8; 32],
        domain_size: u64,
        num_queries: u32,
    ) -> Self {
        let mut h = blake3::Hasher::new();
        h.update(TRANSCRIPT_TAG);
        h.update(input_root);
        h.update(folded_root);
        h.update(&domain_size.to_le_bytes());
        h.update(&(num_queries as u64).to_le_bytes());
        Self {
            state: h,
            counter: 0,
        }
    }

    /// Advance and return 32 fresh challenge bytes.
    pub fn challenge(&mut self) -> [u8; 32] {
        let mut local = self.state.clone();
        local.update(b"challenge");
        local.update(&self.counter.to_le_bytes());
        self.counter = self.counter.saturating_add(1);
        let bytes: [u8; 32] = *local.finalize().as_bytes();
        // Mix the issued challenge back into the running state so
        // subsequent challenges depend on the full transcript.
        self.state.update(&bytes);
        bytes
    }
}

/// Hash an `EnergyCodeword` into a 32-byte commitment used as
/// the FS transcript's input. V1 uses a simple sequential
/// hash; production would use a Merkle root.
pub fn codeword_root(cw: &EnergyCodeword) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"evaporchain:dfri-fs:codeword-root:v1\0");
    h.update(&(cw.positions.len() as u64).to_le_bytes());
    for p in &cw.positions {
        h.update(&p.x.to_le_bytes());
        h.update(&p.fx.to_le_bytes());
        h.update(&p.energy.to_le_bytes());
    }
    *h.finalize().as_bytes()
}

/// Derive `n` distinct query positions from the input codeword's
/// domain, using the transcript as the randomness source.
///
/// Validator-deterministic: same transcript + same codeword →
/// same positions across runs.
pub fn derive_query_positions(
    input: &EnergyCodeword,
    transcript: &mut FsTranscript,
    n: usize,
) -> Vec<FieldElem> {
    if input.positions.is_empty() {
        return vec![];
    }
    let domain_size = input.positions.len();
    let mut chosen: Vec<usize> = Vec::with_capacity(n);
    let mut chosen_set: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    let max_distinct = domain_size;
    while chosen.len() < n.min(max_distinct) {
        let bytes = transcript.challenge();
        let raw = u64::from_le_bytes(bytes[..8].try_into().unwrap());
        let idx = (raw as usize) % domain_size;
        if chosen_set.insert(idx) {
            chosen.push(idx);
        }
    }
    chosen.into_iter().map(|i| input.positions[i].x).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_dfri::CodewordPosition;

    fn cw_4(energy: u64) -> EnergyCodeword {
        // f(x) = x² over domain {1, 2, MOD_P-1, MOD_P-2}.
        EnergyCodeword::new(vec![
            CodewordPosition::new(1, 1, energy),
            CodewordPosition::new(2, 4, energy),
            CodewordPosition::new(MOD_P - 1, 1, energy),
            CodewordPosition::new(MOD_P - 2, 4, energy),
        ])
    }

    #[test]
    fn transcript_is_deterministic() {
        let mut a = FsTranscript::new(&[0xAA; 32], &[0xBB; 32], 4, 8);
        let mut b = FsTranscript::new(&[0xAA; 32], &[0xBB; 32], 4, 8);
        for _ in 0..10 {
            assert_eq!(a.challenge(), b.challenge());
        }
    }

    #[test]
    fn transcript_diverges_on_input_change() {
        let mut a = FsTranscript::new(&[0xAA; 32], &[0xBB; 32], 4, 8);
        let mut b = FsTranscript::new(&[0xAB; 32], &[0xBB; 32], 4, 8);
        let ca = a.challenge();
        let cb = b.challenge();
        assert_ne!(ca, cb);
    }

    #[test]
    fn challenges_advance_under_counter() {
        let mut t = FsTranscript::new(&[0; 32], &[0; 32], 4, 8);
        let c0 = t.challenge();
        let c1 = t.challenge();
        let c2 = t.challenge();
        assert_ne!(c0, c1);
        assert_ne!(c1, c2);
        assert_ne!(c0, c2);
    }

    #[test]
    fn codeword_root_is_deterministic() {
        let cw1 = cw_4(1000);
        let cw2 = cw_4(1000);
        assert_eq!(codeword_root(&cw1), codeword_root(&cw2));
    }

    #[test]
    fn codeword_root_changes_with_energy() {
        let a = cw_4(1000);
        let b = cw_4(500);
        assert_ne!(codeword_root(&a), codeword_root(&b));
    }

    #[test]
    fn derive_positions_gives_distinct_indices() {
        let cw = cw_4(1000);
        let mut t = FsTranscript::new(&codeword_root(&cw), &[0; 32], 4, 4);
        let positions = derive_query_positions(&cw, &mut t, 4);
        assert_eq!(positions.len(), 4);
        let mut sorted = positions.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 4); // all distinct
    }

    #[test]
    fn derive_positions_caps_at_domain_size() {
        let cw = cw_4(1000);
        let mut t = FsTranscript::new(&codeword_root(&cw), &[0; 32], 4, 100);
        // Asking for 100 positions in a domain of size 4 returns 4.
        let positions = derive_query_positions(&cw, &mut t, 100);
        assert_eq!(positions.len(), 4);
    }

    #[test]
    fn derive_positions_validator_deterministic() {
        let cw = cw_4(1000);
        let mut a = FsTranscript::new(&codeword_root(&cw), &[0xCC; 32], 4, 3);
        let mut b = FsTranscript::new(&codeword_root(&cw), &[0xCC; 32], 4, 3);
        let pa = derive_query_positions(&cw, &mut a, 3);
        let pb = derive_query_positions(&cw, &mut b, 3);
        assert_eq!(pa, pb);
    }
}
