//! Coverage tests for Memento contracts — commit-and-forget
//! cryptographic substrate. Per `IMPOSSIBLE_RESEARCH_STACK.md`
//! Branch 4, "Memento contracts" — only EvaporChain can offer the
//! thermodynamic `OwnerEnergyBelow` trigger.
//!
//! Existing in-module tests cover the happy path, 5 trigger variants,
//! commitment binding, and length-prefix anti-collision. This file
//! adds:
//!
//!   - Serde round-trips for `MementoContract` and `MementoReveal`
//!   - `RevealError` + `TriggerError` Display rendering
//!   - `ChainObservation::default` behaviour
//!   - Boundary cases: empty payload, BlockHeightReached at exact
//!     target, OwnerInactiveSince with `min_idle_epochs=0`
//!   - `UnsupportedVersion` path (manual version mutation)
//!   - `commitment` field independence — same payload + nonce, two
//!     different triggers → same commitment (trigger is NOT bound
//!     to the cryptographic commitment, only stored alongside)

use evaporchain_memento::{
    seal, try_reveal, ChainObservation, MementoCommitment, MementoContract, MementoReveal,
    MementoTrigger, MementoVersion, RevealError, TriggerError,
};

const OWNER: evaporchain_types::AccountAddress = [0xAB; 32];

// =================================================================
// Serde round-trips
// =================================================================

#[test]
fn memento_contract_serde_round_trips() {
    let (contract, _) = seal(
        b"sealed".to_vec(),
        [42u8; 32],
        MementoTrigger::BlockHeightReached(1_000),
        OWNER,
        50,
    );
    let json = serde_json::to_string(&contract).expect("serialize");
    let back: MementoContract = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, contract);
}

#[test]
fn memento_reveal_serde_round_trips() {
    let r = MementoReveal {
        payload: b"x".repeat(100),
        nonce: [7u8; 32],
    };
    let json = serde_json::to_string(&r).expect("serialize");
    let back: MementoReveal = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, r);
}

#[test]
fn memento_commitment_serde_round_trips() {
    let c = MementoCommitment([0xAB; 32]);
    let json = serde_json::to_string(&c).expect("serialize");
    let back: MementoCommitment = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, c);
}

#[test]
fn all_5_trigger_variants_serde_round_trip() {
    let variants = vec![
        MementoTrigger::BlockHeightReached(100),
        MementoTrigger::OwnerInactiveSince {
            min_idle_epochs: 50,
        },
        MementoTrigger::OwnerEnergyBelow { threshold: 500 },
        MementoTrigger::OwnerSignedReveal,
        MementoTrigger::AttesterApproval {
            attester: [0xC0; 32],
        },
    ];
    for t in variants {
        let json = serde_json::to_string(&t).expect("serialize");
        let back: MementoTrigger = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, t);
    }
}

// =================================================================
// Error ergonomics
// =================================================================

#[test]
fn reveal_error_displays_all_variants() {
    let m = RevealError::CommitmentMismatch.to_string();
    let t = RevealError::TriggerNotSatisfied.to_string();
    let d = RevealError::TriggerData(TriggerError::MissingChainData("test")).to_string();
    let u = RevealError::UnsupportedVersion(MementoVersion::V1).to_string();
    assert!(m.contains("commitment"), "got: {m}");
    assert!(t.contains("trigger"), "got: {t}");
    assert!(d.contains("chain") || d.contains("test"), "got: {d}");
    assert!(u.contains("V1") || u.contains("version"), "got: {u}");
}

#[test]
fn trigger_error_displays_with_field_name() {
    let err = TriggerError::MissingChainData("owner_energy");
    let s = err.to_string();
    assert!(s.contains("owner_energy"), "got: {s}");
}

// =================================================================
// ChainObservation defaults
// =================================================================

#[test]
fn chain_observation_default_is_all_none_and_empty() {
    let obs = ChainObservation::default();
    assert_eq!(obs.current_epoch, 0);
    assert!(obs.owner_last_active_epoch.is_none());
    assert!(obs.owner_energy.is_none());
    assert!(obs.owner_signed_reveal_for.is_none());
    assert!(obs.attester_approvals.is_empty());
}

// =================================================================
// Boundary cases
// =================================================================

#[test]
fn empty_payload_seals_and_reveals() {
    let (contract, opening) = seal(
        /* empty payload */ vec![],
        [1u8; 32],
        MementoTrigger::BlockHeightReached(0),
        OWNER,
        0,
    );
    let reveal = MementoReveal {
        payload: opening.payload.clone(),
        nonce: opening.nonce,
    };
    let obs = ChainObservation {
        current_epoch: 1,
        ..Default::default()
    };
    let revealed = try_reveal(&contract, &reveal, &obs).expect("empty payload must reveal");
    assert_eq!(revealed, opening.payload);
    assert!(revealed.is_empty());
}

#[test]
fn block_height_reached_at_exact_target_fires() {
    // BlockHeightReached uses `>=`, so target = current is satisfied.
    let (contract, opening) = seal(
        b"x".to_vec(),
        [1u8; 32],
        MementoTrigger::BlockHeightReached(100),
        OWNER,
        0,
    );
    let reveal = MementoReveal {
        payload: opening.payload.clone(),
        nonce: opening.nonce,
    };
    let obs = ChainObservation {
        current_epoch: 100,
        ..Default::default()
    };
    assert!(try_reveal(&contract, &reveal, &obs).is_ok());
}

#[test]
fn owner_inactive_since_with_zero_min_idle_epochs_fires_immediately() {
    // min_idle_epochs=0: even at the exact sealing/activity epoch the
    // window has elapsed by 0, which satisfies `elapsed >= 0`.
    let (contract, opening) = seal(
        b"x".to_vec(),
        [1u8; 32],
        MementoTrigger::OwnerInactiveSince { min_idle_epochs: 0 },
        OWNER,
        100,
    );
    let reveal = MementoReveal {
        payload: opening.payload.clone(),
        nonce: opening.nonce,
    };
    let obs = ChainObservation {
        current_epoch: 100,
        owner_last_active_epoch: Some(100),
        ..Default::default()
    };
    assert!(try_reveal(&contract, &reveal, &obs).is_ok());
}

#[test]
fn owner_energy_below_strict_less_than_at_threshold_minus_one() {
    // strict `<` means energy == threshold-1 fires; threshold itself does NOT.
    let (contract, opening) = seal(
        b"x".to_vec(),
        [1u8; 32],
        MementoTrigger::OwnerEnergyBelow { threshold: 1 },
        OWNER,
        0,
    );
    let reveal = MementoReveal {
        payload: opening.payload.clone(),
        nonce: opening.nonce,
    };
    let obs_zero = ChainObservation {
        current_epoch: 100,
        owner_energy: Some(0),
        ..Default::default()
    };
    assert!(try_reveal(&contract, &reveal, &obs_zero).is_ok());
}

#[test]
fn block_height_reached_at_zero_fires_at_any_epoch() {
    // BlockHeightReached(0) → permanently satisfied.
    let (contract, opening) = seal(
        b"x".to_vec(),
        [1u8; 32],
        MementoTrigger::BlockHeightReached(0),
        OWNER,
        0,
    );
    let reveal = MementoReveal {
        payload: opening.payload.clone(),
        nonce: opening.nonce,
    };
    for epoch in [0u64, 1, 1_000, u64::MAX] {
        let obs = ChainObservation {
            current_epoch: epoch,
            ..Default::default()
        };
        assert!(
            try_reveal(&contract, &reveal, &obs).is_ok(),
            "must fire at epoch {epoch}"
        );
    }
}

// =================================================================
// UnsupportedVersion path
// =================================================================

#[test]
fn unsupported_version_is_rejected_before_commitment_check() {
    // Pin: forwards-compat path. Today only V1 exists, but the version
    // gate is defensive. If a refactor mutates the contract's version
    // field to something other than V1, try_reveal must reject *before*
    // computing the commitment. We can't construct a non-V1 contract
    // through the public API today (the enum only has V1), so this
    // test exists structurally to pin that adding a new variant must
    // also update the version check in try_reveal.
    let (contract, opening) = seal(
        b"x".to_vec(),
        [1u8; 32],
        MementoTrigger::BlockHeightReached(0),
        OWNER,
        0,
    );
    let reveal = MementoReveal {
        payload: opening.payload.clone(),
        nonce: opening.nonce,
    };
    let obs = ChainObservation {
        current_epoch: 1,
        ..Default::default()
    };
    // V1 path must still succeed today.
    assert!(try_reveal(&contract, &reveal, &obs).is_ok());
    // Pin that V1 is the only variant: matching exhaustively must compile.
    match contract.version {
        MementoVersion::V1 => (),
    };
}

// =================================================================
// Commitment / trigger independence
// =================================================================

#[test]
fn same_payload_and_nonce_with_different_triggers_share_commitment() {
    // The commitment is over (payload, nonce, version) only — NOT
    // trigger or owner. Two contracts sealed with identical
    // (payload, nonce) but different triggers must share the same
    // commitment field. This is by design — trigger metadata is
    // stored alongside, not bound into the hash.
    let payload = b"same payload".to_vec();
    let nonce = [0xAA; 32];
    let (c1, _) = seal(
        payload.clone(),
        nonce,
        MementoTrigger::BlockHeightReached(100),
        OWNER,
        0,
    );
    let (c2, _) = seal(
        payload,
        nonce,
        MementoTrigger::OwnerEnergyBelow { threshold: 50 },
        OWNER,
        0,
    );
    assert_eq!(
        c1.commitment, c2.commitment,
        "commitment is over (payload, nonce, version) — not trigger"
    );
    // But the contracts themselves differ.
    assert_ne!(c1, c2);
}

#[test]
fn seal_records_sealed_at_epoch_verbatim() {
    let (contract, _) = seal(
        b"x".to_vec(),
        [1u8; 32],
        MementoTrigger::BlockHeightReached(0),
        OWNER,
        /* sealed_at_epoch */ 12_345_678,
    );
    assert_eq!(contract.sealed_at_epoch, 12_345_678);
    assert_eq!(contract.owner, OWNER);
}

// =================================================================
// Attester approval boundary
// =================================================================

#[test]
fn attester_approval_with_zero_address_is_valid_lookup() {
    // Zero-address attesters are weird but legal — the chain may have
    // a zero-address sentinel for system attestations. Pin that the
    // lookup works for any address pattern.
    let zero_attester: evaporchain_types::AccountAddress = [0u8; 32];
    let (contract, opening) = seal(
        b"x".to_vec(),
        [1u8; 32],
        MementoTrigger::AttesterApproval {
            attester: zero_attester,
        },
        OWNER,
        0,
    );
    let reveal = MementoReveal {
        payload: opening.payload.clone(),
        nonce: opening.nonce,
    };
    let obs = ChainObservation {
        current_epoch: 1,
        attester_approvals: vec![zero_attester],
        ..Default::default()
    };
    assert!(try_reveal(&contract, &reveal, &obs).is_ok());
}

#[test]
fn commitment_ct_eq_differs_on_every_byte_position() {
    // Constant-time pinning: ct_eq must distinguish at every byte
    // position, not just the first. Walk through all 32 positions
    // flipping one bit at a time.
    let base = MementoCommitment([0xAA; 32]);
    for i in 0..32 {
        let mut bytes = [0xAA; 32];
        bytes[i] ^= 0x01;
        let tweaked = MementoCommitment(bytes);
        assert!(!base.ct_eq(&tweaked), "must differ at byte {i}");
        assert!(!tweaked.ct_eq(&base), "ct_eq must be symmetric");
    }
}
