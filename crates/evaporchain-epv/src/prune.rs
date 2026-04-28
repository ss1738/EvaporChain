//! `prune_evaporated` — drop versions whose remaining energy is below
//! `e_min`. Run per-block (or per-epoch) by the node binary so any
//! version that's "physically un-runnable" gets *removed* from the
//! registry, not just flagged.

use serde::{Deserialize, Serialize};

use evaporchain_energy_kernel::ChainLambda;
use evaporchain_types::Energy;

use crate::registry::{EpvRegistry, VersionId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PruneOutcome {
    pub pruned: Vec<VersionId>,
}

/// Walk the registry and drop versions whose `remaining_at(t)` is
/// at or below `e_min`. Returns the list of pruned ids.
///
/// The doctrine ("rollback is physically impossible") only holds when
/// the node binary actually invokes this — the kernel can't enforce
/// the prune itself. Production nodes wire this into the per-block
/// state-transition path.
pub fn prune_evaporated(
    registry: &mut EpvRegistry,
    chain_lambda: ChainLambda,
    t: u64,
    e_min: Energy,
) -> PruneOutcome {
    let to_prune: Vec<VersionId> = registry
        .iter()
        .filter(|v| v.remaining_at(chain_lambda, t) <= e_min)
        .map(|v| v.id)
        .collect();
    let map = registry.versions_mut();
    for id in &to_prune {
        map.remove(id);
    }
    PruneOutcome { pruned: to_prune }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ProtocolVersion;
    use evaporchain_energy_kernel::Lambda;

    fn lambda() -> ChainLambda {
        ChainLambda::new(Lambda::from_epochs(100))
    }

    #[test]
    fn prune_empty_registry_no_op() {
        let mut r = EpvRegistry::new();
        let out = prune_evaporated(&mut r, lambda(), 1000, 1);
        assert!(out.pruned.is_empty());
    }

    #[test]
    fn prune_drops_evaporated_versions() {
        let mut r = EpvRegistry::new();
        r.register(ProtocolVersion::new(1, 1000, 0)).unwrap();
        r.register(ProtocolVersion::new(2, 1000, 500)).unwrap();
        // At t=1000, half_life=100:
        //   v1 elapsed=1000 → 10 halvings → 1000>>10 ≈ 0
        //   v2 elapsed=500  →  5 halvings → 1000>>5  ≈ 31
        // With e_min=20: v1 pruned (0 ≤ 20), v2 stays (31 > 20).
        let out = prune_evaporated(&mut r, lambda(), 1000, 20);
        assert_eq!(out.pruned, vec![1]);
        assert!(!r.contains(1));
        assert!(r.contains(2));
    }

    #[test]
    fn after_prune_version_no_longer_runnable() {
        let mut r = EpvRegistry::new();
        r.register(ProtocolVersion::new(1, 1000, 0)).unwrap();
        prune_evaporated(&mut r, lambda(), 5_000, 1);
        // After heavy decay + prune, version 1 is GONE from the registry —
        // the doctrine's "physically un-runnable" property:
        // is_runnable returns false because the lookup itself misses.
        assert!(!r.is_runnable(1, lambda(), 5_000, 0));
        assert!(!r.contains(1));
    }
}
