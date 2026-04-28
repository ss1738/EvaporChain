//! `PersistenceDiagram` — the (birth, death) pairs of a filtration.
//!
//! `Filtration::Sublevel` (low → high) is the conventional choice;
//! the alternative `Superlevel` is exposed for callers that want to
//! sweep high → low (e.g. "high-energy first" for the EvaporChain
//! interpretation).

use serde::{Deserialize, Serialize};

use evaporchain_types::Energy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Filtration {
    /// Sweep parameter low → high; features appear at low values.
    Sublevel,
    /// Sweep parameter high → low.
    Superlevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PersistenceDiagram {
    /// `(birth, death)` pairs. `death = u64::MAX` for "essential"
    /// features that never die in a finite filtration.
    pub pairs: Vec<(Energy, Energy)>,
    pub filtration: Filtration,
}

impl Default for Filtration {
    fn default() -> Self {
        Filtration::Sublevel
    }
}

impl PersistenceDiagram {
    pub fn new(pairs: Vec<(Energy, Energy)>, filtration: Filtration) -> Self {
        Self { pairs, filtration }
    }

    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// Persistence (death - birth) of each pair. Essential features
    /// (death = u64::MAX) get persistence = u64::MAX.
    pub fn persistences(&self) -> Vec<Energy> {
        self.pairs
            .iter()
            .map(|(b, d)| {
                if *d == Energy::MAX {
                    Energy::MAX
                } else {
                    d.saturating_sub(*b)
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_diagram() {
        let d = PersistenceDiagram::default();
        assert_eq!(d.len(), 0);
        assert!(d.is_empty());
    }

    #[test]
    fn persistences_sub_birth() {
        let d = PersistenceDiagram::new(
            vec![(0, 5), (2, 7), (3, 3)],
            Filtration::Sublevel,
        );
        assert_eq!(d.persistences(), vec![5, 5, 0]);
    }

    #[test]
    fn essential_feature_max_persistence() {
        let d = PersistenceDiagram::new(
            vec![(0, Energy::MAX)],
            Filtration::Sublevel,
        );
        assert_eq!(d.persistences(), vec![Energy::MAX]);
    }
}
