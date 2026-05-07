//! `GraveGraph` — the directed-graph state machine.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct NodeId(pub [u8; 32]);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GraveGraphError {
    #[error("node {0:?} not registered")]
    UnknownNode(NodeId),
    #[error("self-loop edges not allowed")]
    SelfLoop,
    #[error("edge already exists between {from:?} and {to:?} of kind {kind:?}")]
    DuplicateEdge {
        from: NodeId,
        to: NodeId,
        kind: EdgeKind,
    },
    #[error("dedications can only be curated by the living recipient")]
    NotRecipient,
    #[error("only Living source can declare an edge — {0:?} is dead")]
    DeadSource(NodeId),
    #[error("edge {from:?} → {to:?} not found")]
    EdgeNotFound { from: NodeId, to: NodeId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeState {
    Living,
    /// Certified dead at the chain's higher layer; cannot declare
    /// new edges. Existing legacy edges flip to dedications.
    Dead {
        died_at_epoch: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub enum EdgeKind {
    /// Ordinary follow / connection while both alive. Removed
    /// (or marked inert) if either node dies.
    Living,
    /// Legacy edge: declared during life as "this edge should
    /// survive my death". On the source's death, becomes a
    /// dedication FROM the dead TO the survivor.
    Legacy,
    /// Dedication: a Legacy edge whose source has died. The
    /// living recipient can curate (accept/reject/hide).
    Dedication { died_at_epoch: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Curation {
    /// Default — survivor hasn't decided.
    Pending,
    /// Survivor accepts: dedication shows on their profile.
    Accepted,
    /// Survivor rejects: dedication is hidden from their profile
    /// but still visible from the dead's posthumous footprint.
    Rejected,
    /// Survivor hides: not on their profile or aggregate views,
    /// but the on-chain record persists.
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub from: NodeId,
    pub to: NodeId,
    pub kind: EdgeKind,
    /// Only meaningful for Dedication edges; survivor's curation.
    pub curation: Curation,
    pub declared_at_epoch: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraveGraph {
    nodes: BTreeMap<NodeId, NodeState>,
    /// Edges keyed by (from, to). At most one edge per pair.
    edges: BTreeMap<(NodeId, NodeId), Edge>,
}

impl GraveGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_node(&mut self, n: NodeId) {
        self.nodes.entry(n).or_insert(NodeState::Living);
    }

    pub fn node_state(&self, n: &NodeId) -> Option<NodeState> {
        self.nodes.get(n).copied()
    }

    pub fn add_edge(
        &mut self,
        from: NodeId,
        to: NodeId,
        kind: EdgeKind,
        epoch: u64,
    ) -> Result<(), GraveGraphError> {
        if from == to {
            return Err(GraveGraphError::SelfLoop);
        }
        let from_state = self
            .nodes
            .get(&from)
            .copied()
            .ok_or(GraveGraphError::UnknownNode(from))?;
        if !self.nodes.contains_key(&to) {
            return Err(GraveGraphError::UnknownNode(to));
        }
        if matches!(from_state, NodeState::Dead { .. }) {
            return Err(GraveGraphError::DeadSource(from));
        }
        // Cannot create a Dedication directly — only via inversion.
        let kind = match kind {
            EdgeKind::Dedication { .. } => return Err(GraveGraphError::DeadSource(from)),
            other => other,
        };
        if self.edges.contains_key(&(from, to)) {
            return Err(GraveGraphError::DuplicateEdge { from, to, kind });
        }
        self.edges.insert(
            (from, to),
            Edge {
                from,
                to,
                kind,
                curation: Curation::Pending,
                declared_at_epoch: epoch,
            },
        );
        Ok(())
    }

    /// Certify a node dead. Living edges from this node become
    /// inert (cleared). Legacy edges from this node invert to
    /// Dedication.
    pub fn certify_death(
        &mut self,
        n: NodeId,
        died_at_epoch: u64,
    ) -> Result<usize, GraveGraphError> {
        if !self.nodes.contains_key(&n) {
            return Err(GraveGraphError::UnknownNode(n));
        }
        self.nodes.insert(n, NodeState::Dead { died_at_epoch });
        let mut to_remove: Vec<(NodeId, NodeId)> = Vec::new();
        let mut inversions: usize = 0;
        for (key, edge) in self.edges.iter_mut() {
            if edge.from != n {
                continue;
            }
            match edge.kind {
                EdgeKind::Living => {
                    // Living edges from a dead source become inert.
                    to_remove.push(*key);
                }
                EdgeKind::Legacy => {
                    edge.kind = EdgeKind::Dedication { died_at_epoch };
                    edge.curation = Curation::Pending;
                    inversions += 1;
                }
                EdgeKind::Dedication { .. } => {
                    // Already inverted — should not happen for a
                    // freshly-certified death; idempotent.
                }
            }
        }
        for k in to_remove {
            self.edges.remove(&k);
        }
        Ok(inversions)
    }

    /// Curate a dedication. Only the LIVING recipient may curate.
    pub fn curate_dedication(
        &mut self,
        recipient: NodeId,
        from: NodeId,
        choice: Curation,
    ) -> Result<(), GraveGraphError> {
        let edge = self
            .edges
            .get_mut(&(from, recipient))
            .ok_or(GraveGraphError::EdgeNotFound {
                from,
                to: recipient,
            })?;
        if !matches!(edge.kind, EdgeKind::Dedication { .. }) {
            // Curation is meaningless on living/legacy edges.
            return Err(GraveGraphError::NotRecipient);
        }
        // Recipient must be alive to curate.
        let r_state = self
            .nodes
            .get(&recipient)
            .copied()
            .ok_or(GraveGraphError::UnknownNode(recipient))?;
        if matches!(r_state, NodeState::Dead { .. }) {
            return Err(GraveGraphError::NotRecipient);
        }
        edge.curation = choice;
        Ok(())
    }

    /// Edges where `n` is the source.
    pub fn outgoing(&self, n: NodeId) -> impl Iterator<Item = &Edge> {
        self.edges.values().filter(move |e| e.from == n)
    }

    /// Edges where `n` is the target.
    pub fn incoming(&self, n: NodeId) -> impl Iterator<Item = &Edge> {
        self.edges.values().filter(move |e| e.to == n)
    }

    /// All dedications currently directed at `n`. (Filters edges
    /// of kind `Dedication` whose target is `n`.)
    pub fn dedications_for(&self, n: NodeId) -> impl Iterator<Item = &Edge> {
        self.edges
            .values()
            .filter(move |e| e.to == n && matches!(e.kind, EdgeKind::Dedication { .. }))
    }

    /// Posthumous footprint of `n`: dedications from `n` to
    /// living recipients (regardless of curation — the dead's
    /// intent is preserved, the survivor's curation only affects
    /// the survivor's own profile rendering).
    pub fn footprint_of(&self, n: NodeId) -> impl Iterator<Item = &Edge> {
        self.edges
            .values()
            .filter(move |e| e.from == n && matches!(e.kind, EdgeKind::Dedication { .. }))
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(b: u8) -> NodeId {
        NodeId([b; 32])
    }

    fn fresh() -> GraveGraph {
        let mut g = GraveGraph::new();
        for i in 1..=4u8 {
            g.register_node(n(i));
        }
        g
    }

    // ── construction / registration ──────────────────────────────

    #[test]
    fn fresh_graph_has_registered_living_nodes() {
        let g = fresh();
        assert_eq!(g.node_count(), 4);
        for i in 1..=4u8 {
            assert!(matches!(g.node_state(&n(i)).unwrap(), NodeState::Living));
        }
    }

    // ── edge addition ────────────────────────────────────────────

    #[test]
    fn add_living_edge_succeeds() {
        let mut g = fresh();
        g.add_edge(n(1), n(2), EdgeKind::Living, 0).unwrap();
        assert_eq!(g.edge_count(), 1);
    }

    #[test]
    fn add_legacy_edge_succeeds() {
        let mut g = fresh();
        g.add_edge(n(1), n(2), EdgeKind::Legacy, 0).unwrap();
        assert_eq!(g.edge_count(), 1);
    }

    #[test]
    fn self_loop_rejected() {
        let mut g = fresh();
        let err = g.add_edge(n(1), n(1), EdgeKind::Living, 0).unwrap_err();
        assert_eq!(err, GraveGraphError::SelfLoop);
    }

    #[test]
    fn unknown_source_rejected() {
        let mut g = fresh();
        let err = g.add_edge(n(99), n(2), EdgeKind::Living, 0).unwrap_err();
        assert!(matches!(err, GraveGraphError::UnknownNode(_)));
    }

    #[test]
    fn unknown_target_rejected() {
        let mut g = fresh();
        let err = g.add_edge(n(1), n(99), EdgeKind::Living, 0).unwrap_err();
        assert!(matches!(err, GraveGraphError::UnknownNode(_)));
    }

    #[test]
    fn duplicate_edge_rejected() {
        let mut g = fresh();
        g.add_edge(n(1), n(2), EdgeKind::Living, 0).unwrap();
        let err = g.add_edge(n(1), n(2), EdgeKind::Legacy, 1).unwrap_err();
        assert!(matches!(err, GraveGraphError::DuplicateEdge { .. }));
    }

    #[test]
    fn dead_source_cannot_declare_new_edges() {
        let mut g = fresh();
        g.certify_death(n(1), 50).unwrap();
        let err = g.add_edge(n(1), n(2), EdgeKind::Legacy, 60).unwrap_err();
        assert_eq!(err, GraveGraphError::DeadSource(n(1)));
    }

    #[test]
    fn directly_creating_dedication_rejected() {
        // A Dedication can only arise via inversion; direct
        // creation is forbidden.
        let mut g = fresh();
        let err = g
            .add_edge(n(1), n(2), EdgeKind::Dedication { died_at_epoch: 50 }, 60)
            .unwrap_err();
        assert!(matches!(err, GraveGraphError::DeadSource(_)));
    }

    // ── death + edge inversion ───────────────────────────────────

    #[test]
    fn certify_death_clears_living_outgoing() {
        let mut g = fresh();
        g.add_edge(n(1), n(2), EdgeKind::Living, 0).unwrap();
        g.add_edge(n(1), n(3), EdgeKind::Living, 0).unwrap();
        g.certify_death(n(1), 50).unwrap();
        // Living edges from n(1) are gone.
        assert_eq!(g.outgoing(n(1)).count(), 0);
        assert_eq!(g.edge_count(), 0);
    }

    #[test]
    fn certify_death_inverts_legacy_to_dedication() {
        let mut g = fresh();
        g.add_edge(n(1), n(2), EdgeKind::Legacy, 0).unwrap();
        let inverted = g.certify_death(n(1), 50).unwrap();
        assert_eq!(inverted, 1);
        let edge = g.outgoing(n(1)).next().unwrap();
        assert!(matches!(
            edge.kind,
            EdgeKind::Dedication { died_at_epoch: 50 }
        ));
        assert_eq!(edge.curation, Curation::Pending);
    }

    #[test]
    fn certify_death_handles_mixed_edges() {
        let mut g = fresh();
        g.add_edge(n(1), n(2), EdgeKind::Living, 0).unwrap();
        g.add_edge(n(1), n(3), EdgeKind::Legacy, 0).unwrap();
        g.add_edge(n(1), n(4), EdgeKind::Legacy, 0).unwrap();
        let inverted = g.certify_death(n(1), 50).unwrap();
        assert_eq!(inverted, 2);
        // Living edge → cleared. Legacy edges → 2 dedications.
        assert_eq!(g.outgoing(n(1)).count(), 2);
        assert_eq!(g.edge_count(), 2);
    }

    // ── curation ─────────────────────────────────────────────────

    #[test]
    fn curate_dedication_succeeds_for_living_recipient() {
        let mut g = fresh();
        g.add_edge(n(1), n(2), EdgeKind::Legacy, 0).unwrap();
        g.certify_death(n(1), 50).unwrap();
        g.curate_dedication(n(2), n(1), Curation::Accepted).unwrap();
        let edge = g.outgoing(n(1)).next().unwrap();
        assert_eq!(edge.curation, Curation::Accepted);
    }

    #[test]
    fn curate_living_edge_rejected() {
        let mut g = fresh();
        g.add_edge(n(1), n(2), EdgeKind::Living, 0).unwrap();
        let err = g
            .curate_dedication(n(2), n(1), Curation::Accepted)
            .unwrap_err();
        assert_eq!(err, GraveGraphError::NotRecipient);
    }

    #[test]
    fn curate_by_dead_recipient_rejected() {
        let mut g = fresh();
        g.add_edge(n(1), n(2), EdgeKind::Legacy, 0).unwrap();
        g.certify_death(n(1), 50).unwrap();
        // Now the recipient (n(2)) also dies — they cannot curate.
        g.certify_death(n(2), 60).unwrap();
        let err = g
            .curate_dedication(n(2), n(1), Curation::Accepted)
            .unwrap_err();
        assert_eq!(err, GraveGraphError::NotRecipient);
    }

    #[test]
    fn curate_unknown_edge_rejected() {
        let mut g = fresh();
        let err = g
            .curate_dedication(n(2), n(1), Curation::Accepted)
            .unwrap_err();
        assert!(matches!(err, GraveGraphError::EdgeNotFound { .. }));
    }

    // ── footprint + dedications views ────────────────────────────

    #[test]
    fn footprint_of_dead_node_returns_dedications() {
        let mut g = fresh();
        g.add_edge(n(1), n(2), EdgeKind::Legacy, 0).unwrap();
        g.add_edge(n(1), n(3), EdgeKind::Legacy, 0).unwrap();
        g.add_edge(n(1), n(4), EdgeKind::Living, 0).unwrap();
        g.certify_death(n(1), 50).unwrap();
        // Living gets cleared; 2 dedications remain.
        assert_eq!(g.footprint_of(n(1)).count(), 2);
    }

    #[test]
    fn dedications_for_living_recipient() {
        let mut g = fresh();
        g.add_edge(n(1), n(4), EdgeKind::Legacy, 0).unwrap();
        g.add_edge(n(2), n(4), EdgeKind::Legacy, 0).unwrap();
        g.add_edge(n(3), n(4), EdgeKind::Living, 0).unwrap();
        g.certify_death(n(1), 50).unwrap();
        g.certify_death(n(2), 51).unwrap();
        // n(3) still alive → its Living edge is still there but
        // didn't invert. n(4) has 2 incoming dedications.
        assert_eq!(g.dedications_for(n(4)).count(), 2);
    }

    // ── doctrine claim ────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "GraveGraph is the first social structure where
        // death adds connectivity. A user's posthumous footprint
        // is the set of dedications they made (declared as
        // Legacy edges in life, inverted on certified death) +
        // the dedications others made about them. The dead's
        // intent is preserved on chain; survivors curate only
        // their own profile rendering."

        let mut g = fresh();
        // Pre-death: n(1) declares 2 legacy edges (to n(2), n(3)).
        g.add_edge(n(1), n(2), EdgeKind::Legacy, 0).unwrap();
        g.add_edge(n(1), n(3), EdgeKind::Legacy, 0).unwrap();
        // n(1) also has a living edge to n(4).
        g.add_edge(n(1), n(4), EdgeKind::Living, 0).unwrap();
        assert_eq!(g.outgoing(n(1)).count(), 3);

        // n(1) dies.
        g.certify_death(n(1), 100).unwrap();

        // Posthumous: living edge cleared; 2 dedications survive.
        assert_eq!(g.outgoing(n(1)).count(), 2);
        assert_eq!(g.footprint_of(n(1)).count(), 2);

        // Survivors curate their profiles independently.
        g.curate_dedication(n(2), n(1), Curation::Accepted).unwrap();
        g.curate_dedication(n(3), n(1), Curation::Hidden).unwrap();

        // The dead's footprint is unchanged by curation —
        // the dedication still exists on chain. Only rendering
        // is per-survivor.
        let footprint: Vec<&Edge> = g.footprint_of(n(1)).collect();
        assert_eq!(footprint.len(), 2);
        let curations: Vec<Curation> = footprint.iter().map(|e| e.curation).collect();
        assert!(curations.contains(&Curation::Accepted));
        assert!(curations.contains(&Curation::Hidden));
    }

    proptest::proptest! {
        #[test]
        fn property_certified_death_never_leaves_living_outgoing(
            n_legacy in 0u8..6u8,
            n_living in 0u8..6u8,
        ) {
            // For any number of Legacy + Living outgoing edges,
            // after certify_death, the count of Living outgoing
            // from the dead node is exactly 0.
            let mut g = GraveGraph::new();
            for i in 0..=12u8 { g.register_node(n(i)); }
            for i in 0..n_legacy {
                g.add_edge(n(0), n(1 + i), EdgeKind::Legacy, 0).unwrap();
            }
            for i in 0..n_living {
                g.add_edge(n(0), n(1 + n_legacy + i), EdgeKind::Living, 0).unwrap();
            }
            g.certify_death(n(0), 50).unwrap();
            let living_out = g
                .outgoing(n(0))
                .filter(|e| matches!(e.kind, EdgeKind::Living))
                .count();
            proptest::prop_assert_eq!(living_out, 0);
            let dedications = g.outgoing(n(0)).count();
            proptest::prop_assert_eq!(dedications as u8, n_legacy);
        }
    }
}
