//! `compute_h0` — 0-dimensional persistence over a sequence of energy
//! values.
//!
//! For `H_0`, every value is born at its own filtration level and
//! dies when it merges with a larger one. With a *sublevel*
//! filtration over scalars, this reduces to: sort the values
//! ascending; each value is born at itself and dies when it joins a
//! larger value's component (i.e. at the next *strictly larger*
//! value). The single largest value is essential — it never dies —
//! and we record it with `death = Energy::MAX`.
//!
//! This is the simplest case of persistent homology and is exact for
//! the `H_0` dimension at substrate scope.

use crate::diagram::{Filtration, PersistenceDiagram};
use evaporchain_types::Energy;

pub fn compute_h0(values: &[Energy]) -> PersistenceDiagram {
    if values.is_empty() {
        return PersistenceDiagram::default();
    }
    let mut sorted: Vec<Energy> = values.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    let mut pairs: Vec<(Energy, Energy)> = Vec::with_capacity(n);
    for i in 0..n {
        let b = sorted[i];
        let d = if i + 1 < n {
            sorted[i + 1]
        } else {
            Energy::MAX
        };
        pairs.push((b, d));
    }
    PersistenceDiagram::new(pairs, Filtration::Sublevel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_empty_diagram() {
        assert!(compute_h0(&[]).is_empty());
    }

    #[test]
    fn singleton_is_essential_feature() {
        let d = compute_h0(&[42]);
        assert_eq!(d.pairs, vec![(42, Energy::MAX)]);
    }

    #[test]
    fn three_distinct_values_two_finite_one_essential() {
        let d = compute_h0(&[1, 5, 3]);
        // sorted: [1, 3, 5]. Pairs: (1,3), (3,5), (5,MAX).
        assert_eq!(d.pairs, vec![(1, 3), (3, 5), (5, Energy::MAX)]);
    }

    #[test]
    fn duplicate_values_yield_zero_persistence_pairs() {
        let d = compute_h0(&[5, 5, 5]);
        // sorted: [5,5,5] → pairs: (5,5), (5,5), (5,MAX).
        // First two have persistence 0 (born and die at same level).
        let pers = d.persistences();
        assert_eq!(pers, vec![0, 0, Energy::MAX]);
    }
}
