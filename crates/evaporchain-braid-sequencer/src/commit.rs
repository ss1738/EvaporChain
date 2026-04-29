//! `commit_braid` — domain-separated blake3 over the reduced word.

use crate::reduce::reduce_canonical;
use crate::word::BraidWord;

const COMMIT_TAG: &[u8] = b"evaporchain-braid-commit";

/// Commit to `word`. First reduces to substrate-canonical form, then
/// hashes (length || generators in i32 little-endian).
pub fn commit_braid(word: &BraidWord) -> [u8; 32] {
    let reduced = reduce_canonical(word);
    let mut h = blake3::Hasher::new();
    h.update(COMMIT_TAG);
    h.update(&(reduced.generators.len() as u64).to_le_bytes());
    for &g in &reduced.generators {
        h.update(&g.to_le_bytes());
    }
    *h.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(gens: Vec<i32>) -> BraidWord {
        BraidWord::new(gens, 10).unwrap()
    }

    #[test]
    fn identity_has_a_well_defined_commitment() {
        let _ = commit_braid(&BraidWord::identity());
    }

    #[test]
    fn commitment_is_deterministic() {
        let c1 = commit_braid(&w(vec![1, 2, 3]));
        let c2 = commit_braid(&w(vec![1, 2, 3]));
        assert_eq!(c1, c2);
    }

    #[test]
    fn equivalent_words_share_commitment() {
        // After reduction these collapse to identity → same commitment.
        let c_id = commit_braid(&BraidWord::identity());
        let c_cancel = commit_braid(&w(vec![1, -1]));
        assert_eq!(c_id, c_cancel);
    }

    #[test]
    fn commuting_rearrangement_shares_commitment() {
        // σ_1 σ_3 ≡ σ_3 σ_1 under commuting (|3-1|=2).
        let a = commit_braid(&w(vec![1, 3]));
        let b = commit_braid(&w(vec![3, 1]));
        assert_eq!(a, b);
    }

    #[test]
    fn distinct_braids_distinct_commitments() {
        let a = commit_braid(&w(vec![1, 2]));
        let b = commit_braid(&w(vec![2, 1]));
        // Non-commuting (|2-1|=1) → substrate leaves them in order.
        assert_ne!(a, b);
    }
}
