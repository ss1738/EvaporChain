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

    /// T1.20 — `Tx::new` populates every field directly from its
    /// arguments. Trivial pin, but prevents a future "convenience"
    /// constructor that silently drops or reorders fields.
    #[test]
    fn t1_20_tx_new_populates_all_fields() {
        let mut ws = BTreeMap::new();
        ws.insert(key(7), vec![0xAA, 0xBB]);
        let tx = Tx::new(id(42), 1000, vec![key(1), key(2)], ws.clone());
        assert_eq!(tx.id, id(42));
        assert_eq!(tx.energy, 1000);
        assert_eq!(tx.read_set, vec![key(1), key(2)]);
        assert_eq!(tx.write_set, ws);
    }

    /// T1.20 — `StateKey` ordering is lexicographic on the underlying
    /// `[u8; 32]`. The scheduler relies on this for validator-
    /// deterministic conflict resolution; an `Ord` derive change that
    /// silently inverted or broke this would re-order commits across
    /// validators and split consensus.
    #[test]
    fn t1_20_state_key_ord_is_lexicographic() {
        let a = StateKey([0x00; 32]);
        let mut b_bytes = [0u8; 32];
        b_bytes[0] = 0x01;
        let b = StateKey(b_bytes);
        let mut c_bytes = [0u8; 32];
        c_bytes[31] = 0x01;
        let c = StateKey(c_bytes);
        // a < c (differs at last byte) < b (differs at first byte).
        // Lexicographic order on big-endian byte arrays: leftmost-
        // byte difference dominates.
        assert!(a < c, "a < c (a is all zero, c differs at last byte)");
        assert!(
            c < b,
            "c < b (b differs at first byte ≫ last-byte diff in c)"
        );
        assert!(a < b);
    }

    /// T1.20 — `TxOutcome::AbortedConflict` equality is field-wise.
    /// Distinct `winner` ids produce distinct outcomes even when
    /// `tx_id` and `contended_keys` match. Pins that the scheduler's
    /// outcome-equality predicate doesn't accidentally drop fields.
    #[test]
    fn t1_20_tx_outcome_aborted_conflict_distinguishes_each_field() {
        let base = TxOutcome::AbortedConflict {
            tx_id: id(1),
            winner: id(2),
            contended_keys: vec![key(0)],
        };
        // Different winner → different outcome.
        let other_winner = TxOutcome::AbortedConflict {
            tx_id: id(1),
            winner: id(99),
            contended_keys: vec![key(0)],
        };
        assert_ne!(base, other_winner);
        // Different tx_id → different.
        let other_tx = TxOutcome::AbortedConflict {
            tx_id: id(77),
            winner: id(2),
            contended_keys: vec![key(0)],
        };
        assert_ne!(base, other_tx);
        // Different contended_keys → different.
        let other_keys = TxOutcome::AbortedConflict {
            tx_id: id(1),
            winner: id(2),
            contended_keys: vec![key(0), key(1)],
        };
        assert_ne!(base, other_keys);
        // Committed and AbortedConflict are different variants.
        let committed = TxOutcome::Committed { tx_id: id(1) };
        assert_ne!(base, committed);
    }
}
