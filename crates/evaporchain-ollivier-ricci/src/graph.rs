//! Unweighted graph + BFS distance + lazy-walk distribution.

use std::collections::{BTreeSet, HashMap, VecDeque};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct NodeId(pub u32);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GraphError {
    #[error("node {0:?} not in graph")]
    UnknownNode(NodeId),
    #[error("self-loop edges not supported")]
    SelfLoop,
    #[error("nodes {0:?} and {1:?} are not connected — distance undefined")]
    Disconnected(NodeId, NodeId),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Graph {
    /// Adjacency: node -> sorted set of neighbors. BTreeSet so
    /// validators agree byte-for-byte on neighbor enumeration.
    adj: HashMap<NodeId, BTreeSet<NodeId>>,
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(&mut self, n: NodeId) {
        self.adj.entry(n).or_insert_with(BTreeSet::new);
    }

    pub fn add_edge(&mut self, u: NodeId, v: NodeId) -> Result<(), GraphError> {
        if u == v {
            return Err(GraphError::SelfLoop);
        }
        self.adj.entry(u).or_insert_with(BTreeSet::new).insert(v);
        self.adj.entry(v).or_insert_with(BTreeSet::new).insert(u);
        Ok(())
    }

    pub fn nodes(&self) -> impl Iterator<Item = &NodeId> {
        self.adj.keys()
    }

    pub fn neighbors(&self, n: &NodeId) -> Option<&BTreeSet<NodeId>> {
        self.adj.get(n)
    }

    pub fn degree(&self, n: &NodeId) -> usize {
        self.adj.get(n).map(|s| s.len()).unwrap_or(0)
    }

    /// Hop distance between two nodes. BFS.
    pub fn distance(&self, from: NodeId, to: NodeId) -> Result<u64, GraphError> {
        if !self.adj.contains_key(&from) {
            return Err(GraphError::UnknownNode(from));
        }
        if !self.adj.contains_key(&to) {
            return Err(GraphError::UnknownNode(to));
        }
        if from == to {
            return Ok(0);
        }
        let mut q = VecDeque::new();
        q.push_back((from, 0u64));
        let mut visited: BTreeSet<NodeId> = BTreeSet::new();
        visited.insert(from);
        while let Some((node, dist)) = q.pop_front() {
            if let Some(neighs) = self.adj.get(&node) {
                for &nb in neighs {
                    if nb == to {
                        return Ok(dist + 1);
                    }
                    if visited.insert(nb) {
                        q.push_back((nb, dist + 1));
                    }
                }
            }
        }
        Err(GraphError::Disconnected(from, to))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(i: u32) -> NodeId {
        NodeId(i)
    }

    #[test]
    fn add_edge_creates_undirected_link() {
        let mut g = Graph::new();
        g.add_edge(n(1), n(2)).unwrap();
        assert!(g.neighbors(&n(1)).unwrap().contains(&n(2)));
        assert!(g.neighbors(&n(2)).unwrap().contains(&n(1)));
    }

    #[test]
    fn self_loop_rejected() {
        let mut g = Graph::new();
        let err = g.add_edge(n(1), n(1)).unwrap_err();
        assert_eq!(err, GraphError::SelfLoop);
    }

    #[test]
    fn distance_zero_for_self() {
        let mut g = Graph::new();
        g.add_node(n(1));
        assert_eq!(g.distance(n(1), n(1)).unwrap(), 0);
    }

    #[test]
    fn distance_one_for_neighbors() {
        let mut g = Graph::new();
        g.add_edge(n(1), n(2)).unwrap();
        assert_eq!(g.distance(n(1), n(2)).unwrap(), 1);
    }

    #[test]
    fn distance_through_path() {
        let mut g = Graph::new();
        for i in 1..=5 {
            g.add_edge(n(i), n(i + 1)).unwrap();
        }
        assert_eq!(g.distance(n(1), n(6)).unwrap(), 5);
    }

    #[test]
    fn distance_disconnected_errors() {
        let mut g = Graph::new();
        g.add_node(n(1));
        g.add_node(n(2));
        let err = g.distance(n(1), n(2)).unwrap_err();
        assert_eq!(err, GraphError::Disconnected(n(1), n(2)));
    }

    #[test]
    fn distance_unknown_node_errors() {
        let g = Graph::new();
        let err = g.distance(n(1), n(2)).unwrap_err();
        assert!(matches!(err, GraphError::UnknownNode(_)));
    }

    #[test]
    fn round_trip_serde() {
        let mut g = Graph::new();
        g.add_edge(n(1), n(2)).unwrap();
        g.add_edge(n(2), n(3)).unwrap();
        let s = serde_json::to_string(&g).unwrap();
        let back: Graph = serde_json::from_str(&s).unwrap();
        assert_eq!(back.distance(n(1), n(3)).unwrap(), 2);
    }
}
