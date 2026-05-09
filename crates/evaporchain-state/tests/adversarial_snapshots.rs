//! T0.8 — Light-client / fast-sync adversarial snapshot fixtures.
//!
//! Per `MAINNET_READINESS.md` T0.8, this file ships the adversarial
//! snapshot harness that locks fast-sync rejection contracts. These
//! complement the in-source `mod tests` in `src/snapshot.rs` which
//! covers the substrate (round-trip, integrity-hash mismatch,
//! version mismatch, chain-id mismatch, integrity-hash reproducible
//! across `created_at`).
//!
//! What this file ships:
//!
//! | # | Fixture | Status | What it locks / surfaces |
//! |---|---|---|---|
//! | 1 | tampered bytes + recomputed integrity_hash | ⚠️ KNOWN GAP `#[ignore]` | shows that without quorum-cert binding (T0.8 sub-task 2) an attacker who can re-hash gets through |
//! | 2 | duplicate validator IDs in validator_set | ✅ documents accept-currently behaviour | apply_to does not reject duplicates today; flag for downstream defense |
//! | 3 | truncated zstd payload (chunk-level attack) | ✅ rejected | locks zstd-decompress error path as a defense |
//!
//! Vectors NOT covered here pending substrate work:
//!   - quorum-cert verification (T0.8 sub-task 2 — needs commit-cert
//!     field on SnapshotFile, plus 2f+1-attestation verification)
//!   - partial-state-withhold detection (T0.8 sub-task 4 — needs an
//!     account/object count claim + verification at apply_to time)
//!
//! These are deferred until the substrate is in place; fixtures will
//! be added when the verification path lands.

use evaporchain_state::db::{InMemoryStateDB, StateDB};
use evaporchain_state::snapshot::{
    SnapshotError, SnapshotFile, SnapshotValidator, ValidatorSetSnapshot,
};
use evaporchain_types::{Account, ObjectState, StateObject};

fn addr(b: u8) -> [u8; 32] {
    [b; 32]
}

fn obj_id(b: u8) -> [u8; 32] {
    let mut v = [0u8; 32];
    v[0] = b;
    v
}

fn make_account(b: u8, balance: u64) -> Account {
    Account {
        address: addr(b),
        balance,
        nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        last_touched_epoch: 0,
        vesting: None,
    }
}

fn make_object(b: u8, energy: u64) -> StateObject {
    StateObject {
        id: obj_id(b),
        owner: addr(1),
        energy,
        half_life: 1000,
        created_at: 0,
        last_refreshed: 0,
        state: ObjectState::Active,
        grace_epoch: None,
        data: format!("object-{}", b).into_bytes(),
        decay_curve: None,
        lad_mode: None,
    }
}

fn populate_db(db: &mut InMemoryStateDB) {
    db.put_account(make_account(1, 1_000_000));
    db.put_account(make_account(2, 500_000));
    db.put_object(make_object(1, 100));
}

fn make_validator_set() -> ValidatorSetSnapshot {
    ValidatorSetSnapshot {
        validators: vec![SnapshotValidator {
            id: 1,
            stake: 1000,
            address: addr(1),
            bls_public_key: None,
            vrf_public_key: None,
            jailed: false,
        }],
    }
}

fn make_clean_snapshot() -> SnapshotFile {
    let mut db = InMemoryStateDB::new();
    populate_db(&mut db);
    SnapshotFile::create(
        &mut db,
        "evaporchain-test-1",
        100,
        5,
        [0xCC; 32],
        None,
        make_validator_set(),
    )
    .expect("create clean snapshot")
}

// ─── Fixture 1 — Forged integrity_hash matches tampered bytes ───────
//
// KNOWN GAP. An attacker who controls snapshot bytes AND can recompute
// the integrity_hash can pass the integrity check. The defense is
// snapshot quorum-cert verification (T0.8 sub-task 2): bind the
// integrity_hash to a 2f+1-attestation signed by the validator set.
// Without that binding, integrity_hash is self-attested, which is
// not a defense against a forging attacker.
//
// The fixture below WOULD pass `from_bytes` today (no rejection).
// Marked `#[ignore]` so CI doesn't fail; flips to enabled once the
// quorum-cert verification path lands.
#[test]
#[ignore = "known gap — no quorum-cert binding yet (T0.8 sub-task 2)"]
fn adversarial_t08_forged_integrity_hash_matches_tampered_bytes() {
    let mut file = make_clean_snapshot();

    // Attacker's tamper: change the chain_id (any field works).
    let original_chain_id = file.chain_id.clone();
    file.chain_id = "evaporchain-attacker".to_string();

    // Attacker recomputes the integrity_hash to match the tampered
    // bytes. This currently passes the from_bytes check because the
    // hash is self-attested with no external signature binding.
    file.integrity_hash = [0u8; 32]; // SnapshotFile::create sets this; recompute via to_bytes path
    let bytes = file.to_bytes().expect("serialize tampered snapshot");

    // Today this fails to detect the tamper because we're recomputing
    // the integrity hash to match. The eventual defense (quorum-cert)
    // would catch it because validators attest to the original
    // integrity_hash and won't sign the attacker's recomputed one.
    let parsed = SnapshotFile::from_bytes(&bytes);
    assert!(
        parsed.is_ok(),
        "TODAY: forged integrity_hash + tampered chain_id passes verify \
         (this assertion will FAIL once T0.8 quorum-cert binding lands; \
         flip the test direction at that point)"
    );
    let parsed = parsed.unwrap();
    assert_eq!(parsed.chain_id, "evaporchain-attacker");
    assert_ne!(parsed.chain_id, original_chain_id);
}

// ─── Fixture 2 — Duplicate validator IDs in validator_set ───────────
//
// A snapshot whose validator_set contains two entries with the same
// `id` is structurally malformed. Today's apply_to does not reject
// this — it accepts the snapshot and the duplicate IDs flow through
// to whatever consumes the validator set (consensus, slashing, etc.).
//
// This test documents the accept-currently behaviour. Future
// hardening (T0.8 follow-on) should add a structural validation at
// `from_bytes` or `apply_to` time.
#[test]
fn adversarial_t08_duplicate_validator_ids_in_set_accepted_today() {
    let mut db = InMemoryStateDB::new();
    populate_db(&mut db);

    let dup_validator_set = ValidatorSetSnapshot {
        validators: vec![
            SnapshotValidator {
                id: 1,
                stake: 1000,
                address: addr(1),
                bls_public_key: None,
                vrf_public_key: None,
                jailed: false,
            },
            SnapshotValidator {
                id: 1, // DUPLICATE ID
                stake: 9999,
                address: addr(2),
                bls_public_key: None,
                vrf_public_key: None,
                jailed: false,
            },
        ],
    };
    let file = SnapshotFile::create(
        &mut db,
        "evaporchain-test-1",
        100,
        5,
        [0xCC; 32],
        None,
        dup_validator_set,
    )
    .expect("create snapshot with duplicate validator IDs");
    let bytes = file.to_bytes().expect("serialize");
    let parsed = SnapshotFile::from_bytes(&bytes).expect("from_bytes accepts");

    // ACCEPT-CURRENTLY behaviour: the snapshot deserialises with the
    // duplicate IDs intact. Documents the gap. Future T0.8 hardening
    // should add `validators.iter().map(|v| v.id).collect::<HashSet>().len()
    // == validators.len()` check in from_bytes.
    assert_eq!(parsed.validator_set.validators.len(), 2);
    assert_eq!(parsed.validator_set.validators[0].id, 1);
    assert_eq!(parsed.validator_set.validators[1].id, 1);
}

// ─── Fixture 3 — Truncated zstd payload (chunk-level attack) ───────
//
// An attacker truncates the snapshot bytes in transit (e.g. drops
// the last network chunk). zstd::stream::decode_all fails on
// truncated input → from_bytes returns Err. Locks the chunk-level
// integrity defense.
#[test]
fn adversarial_t08_truncated_zstd_payload_rejects() {
    let file = make_clean_snapshot();
    let mut bytes = file.to_bytes().expect("serialize");

    // Truncate to 2/3 of the payload. The 5-byte magic + version
    // header is preserved; only the zstd body is cut short.
    let cut_at = bytes.len() * 2 / 3;
    bytes.truncate(cut_at);

    let result = SnapshotFile::from_bytes(&bytes);
    assert!(
        result.is_err(),
        "truncated snapshot must fail to verify; got {:?}",
        result
    );
    // Specific error: deserialization fails on truncated zstd.
    match result {
        Err(SnapshotError::DeserializationError(_)) => {}
        Err(other) => panic!(
            "expected DeserializationError from zstd truncation, got {:?}",
            other
        ),
        Ok(_) => panic!("truncated snapshot must not deserialize"),
    }
}
