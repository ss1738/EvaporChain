//! Backwards-compatibility tests for `ConsensusCheckpoint` serde.
//!
//! Guards the `CHECKPOINT_VERSION = 2` bump that added
//! `last_bell_reading: Option<CheckpointedBellReading>` so per-block
//! CHSH Bell-Beacon readings survive node restart. The two scenarios
//! that must work forever:
//!
//! 1. New format round-trips cleanly with the bell reading attached.
//! 2. Old (pre-bump) JSON, written before the `version` and
//!    `last_bell_reading` fields existed, deserializes into the new
//!    struct with `version = 1` and `last_bell_reading = None`.

use evaporchain_consensus::persistence::{
    CheckpointedBellReading, ConsensusCheckpoint, CHECKPOINT_VERSION,
};

#[test]
fn new_format_round_trips_with_bell_reading() {
    // Hand-built checkpoint with the v2 fields populated. We don't go
    // through `from_consensus` here because we want to assert exactly
    // what the wire format contains.
    let cp = ConsensusCheckpoint {
        version: CHECKPOINT_VERSION,
        height: 4242,
        epoch: 17,
        parent_hash: [0xAB; 32],
        weak_subjectivity_checkpoints: vec![(4000, [0xCD; 32])],
        validators: vec![],
        last_bell_reading: Some(CheckpointedBellReading {
            s_value_milli: 2415,
            block_height: 4242,
            epoch: 17,
            certified: true,
        }),
    };

    let json = serde_json::to_string(&cp).expect("serialize v2 checkpoint");
    let back: ConsensusCheckpoint = serde_json::from_str(&json).expect("deserialize v2 checkpoint");

    assert_eq!(back.version, CHECKPOINT_VERSION);
    assert_eq!(back.height, 4242);
    let r = back
        .last_bell_reading
        .expect("bell reading must round-trip");
    assert_eq!(r.s_value_milli, 2415);
    assert_eq!(r.block_height, 4242);
    assert_eq!(r.epoch, 17);
    assert!(r.certified);
}

#[test]
fn old_format_deserializes_without_version_or_bell_reading() {
    // Pre-bump checkpoint format: no `version`, no `last_bell_reading`.
    // Real on-disk files written by nodes before commit `859d15f`'s
    // follow-up look like this. They MUST keep loading after the bump.
    let old_json = r#"{
        "height": 100,
        "epoch": 5,
        "parent_hash": [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
        "weak_subjectivity_checkpoints": [],
        "validators": []
    }"#;

    let cp: ConsensusCheckpoint =
        serde_json::from_str(old_json).expect("old checkpoint must deserialize");

    assert_eq!(cp.version, 1, "missing version defaults to 1");
    assert_eq!(cp.height, 100);
    assert_eq!(cp.epoch, 5);
    assert!(
        cp.last_bell_reading.is_none(),
        "missing last_bell_reading must default to None"
    );
}
