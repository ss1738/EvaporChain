//! Substrate-quality canonical reduction.
//!
//! Two simplifications are applied repeatedly until fixpoint:
//!
//! 1. **Trivial cancellation**: adjacent `+i, -i` (or `-i, +i`)
//!    collapses to identity. This is the `σ_i σ_i^{-1} = e` relation.
//! 2. **Commuting normalisation**: adjacent generators that commute
//!    (`|i − j| ≥ 2`) are sorted into ascending order of `|generator|`.
//!    This collapses every commuting permutation into one canonical
//!    representative.
//!
//! **NOT implemented:** the braid relation
//! `σ_i σ_{i+1} σ_i = σ_{i+1} σ_i σ_{i+1}`. That requires full
//! Garside normal form. Two braid words equivalent under this
//! relation will produce DIFFERENT commitments in the substrate;
//! production `evaporchain-braid-garside` (future) closes that gap.

use crate::word::BraidWord;

/// Apply repeated trivial-cancel + commuting-sort until fixpoint.
pub fn reduce_canonical(word: &BraidWord) -> BraidWord {
    let mut g = word.generators.clone();
    loop {
        let before = g.clone();
        cancel_inverses(&mut g);
        sort_commuting(&mut g);
        if g == before {
            break;
        }
    }
    BraidWord { generators: g }
}

fn cancel_inverses(g: &mut Vec<i32>) {
    let mut i = 0;
    while i + 1 < g.len() {
        if g[i] == -g[i + 1] {
            g.remove(i);
            g.remove(i);
            // Step back one to re-check the new neighbour pair.
            i = i.saturating_sub(1);
        } else {
            i += 1;
        }
    }
}

fn sort_commuting(g: &mut [i32]) {
    // Bubble-sort adjacent commuting pairs (|abs diff| >= 2). Stop at
    // non-commuting pairs (which we don't reorder).
    loop {
        let mut swapped = false;
        for i in 0..g.len().saturating_sub(1) {
            let a = g[i];
            let b = g[i + 1];
            if (a.abs() - b.abs()).abs() >= 2 && a.abs() > b.abs() {
                g.swap(i, i + 1);
                swapped = true;
            }
        }
        if !swapped {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(gens: Vec<i32>) -> BraidWord {
        BraidWord::new(gens, 10).unwrap()
    }

    #[test]
    fn identity_reduces_to_identity() {
        let r = reduce_canonical(&BraidWord::identity());
        assert_eq!(r, BraidWord::identity());
    }

    #[test]
    fn trivial_cancel_collapses_to_identity() {
        let r = reduce_canonical(&w(vec![1, -1]));
        assert!(r.is_empty());
    }

    #[test]
    fn nested_cancel_collapses() {
        // σ_1 σ_2 σ_2^{-1} σ_1^{-1} → identity.
        let r = reduce_canonical(&w(vec![1, 2, -2, -1]));
        assert!(r.is_empty());
    }

    #[test]
    fn commuting_sort_canonicalises() {
        // σ_3 σ_1 → σ_1 σ_3 (|3-1|=2, commute).
        let r = reduce_canonical(&w(vec![3, 1]));
        assert_eq!(r.generators, vec![1, 3]);
    }

    #[test]
    fn non_commuting_left_alone() {
        // σ_2 σ_1 do NOT commute (|2-1|=1) — substrate leaves them.
        let r = reduce_canonical(&w(vec![2, 1]));
        assert_eq!(r.generators, vec![2, 1]);
    }

    #[test]
    fn cancel_then_sort_reaches_fixpoint() {
        // σ_3 σ_1 σ_1^{-1} → σ_3 → already sorted.
        let r = reduce_canonical(&w(vec![3, 1, -1]));
        assert_eq!(r.generators, vec![3]);
    }
}
