//! `Tx` — a transaction's read-set / write-set declaration.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// 32-byte chain-wide handle for a state slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct StateKey(pub [u8; 32]);

/// Opaque value type. Stored as bytes; the chain's higher layer
/// interprets.
pub type StateValue = Vec<u8>;

/// 32-byte transaction id. Validators agree byte-for-byte on this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct TxId(pub [u8; 32]);

/// One transaction's declared effect.
///
/// V1: read-set is a list of keys (not key-version pairs — version
/// tracking comes in V2 with optimistic-concurrency-control proper).
/// Write-set is a map of `key → new_value`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tx {
    pub id: TxId,
    /// Energy of this transaction. Used for priority. Higher
    /// energy = higher priority. Validator-deterministic ordering.
    pub energy: u64,
    /// Keys this transaction reads. The scheduler validates that
    /// these keys haven't been touched by a committed tx since
    /// snapshot.
    pub read_set: Vec<StateKey>,
    /// Keys this transaction writes (with their new values). All
    /// or nothing — atomic commit.
    pub write_set: BTreeMap<StateKey, StateValue>,
}

impl Tx {
    pub fn new(
        id: TxId,
        energy: u64,
        read_set: Vec<StateKey>,
        write_set: BTreeMap<StateKey, StateValue>,
    ) -> Self {
        Self {
            id,
            energy,
            read_set,
            write_set,
        }
    }
}

/// Result of attempting to commit a transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TxOutcome {
    /// The transaction's write-set was applied to state.
    Committed { tx_id: TxId },
    /// A higher-priority transaction touched our read-set first;
    /// retry against fresh snapshot. Caller decides.
    AbortedConflict {
        tx_id: TxId,
        winner: TxId,
        contended_keys: Vec<StateKey>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(b: u8) -> StateKey {
        StateKey([b; 32])
    }
    fn id(b: u8) -> TxId {
        TxId([b; 32])
    }

    #[test]
    fn tx_components_serde_round_trip() {
        // Tx contains BTreeMap<StateKey, _> where StateKey is
        // [u8; 32]; JSON requires string-typed map keys, so the
        // full Tx doesn't round-trip through JSON. The chain wraps
        // the scheduler with binary durability (RocksDB-keyed,
        // bincode payloads, etc.). Round-trip the key/id types.
        let k = key(1);
        let s = serde_json::to_string(&k).unwrap();
        let back: StateKey = serde_json::from_str(&s).unwrap();
        assert_eq!(k, back);

        let t_id = id(0xAA);
        let s = serde_json::to_string(&t_id).unwrap();
        let back: TxId = serde_json::from_str(&s).unwrap();
        assert_eq!(t_id, back);
    }

    #[test]
    fn outcome_serde_round_trips() {
        let o = TxOutcome::Committed { tx_id: id(1) };
        let s = serde_json::to_string(&o).unwrap();
        let back: TxOutcome = serde_json::from_str(&s).unwrap();
        assert_eq!(o, back);

        let o = TxOutcome::AbortedConflict {
            tx_id: id(1),
            winner: id(2),
            contended_keys: vec![key(0)],
        };
        let s = serde_json::to_string(&o).unwrap();
        let back: TxOutcome = serde_json::from_str(&s).unwrap();
        assert_eq!(o, back);
    }
}
