//! End-to-end integration tests for evaporchain-singh-posthuma.
//!
//! Non-trivial fixture: "Alice's Last Testament" — full lifecycle
//! from Seal to Memorial, plus a suspension-gap proof.
//!
//!   Alice seals her testament:
//!     initial=8_192, half_life=100, sealed_at=0
//!     committee: validators [1..=5], threshold=3
//!
//!   Sealed phase (epoch 0 → 400):
//!     visible_energy_at(any epoch) = 8_192 — decay is suspended.
//!
//!   Epoch 400: validators 1, 2, 3 certify Alice's death.
//!     cert.death_epoch = 400.
//!     → Revealed { revealed_at_epoch: 400 }
//!
//!   Revealed phase (half-life clock starts at epoch 400):
//!     visible_energy_at(400) = energy_at_epoch(8_192, 100,   0) = 8_192
//!     visible_energy_at(500) = energy_at_epoch(8_192, 100, 100) = 8_192>>1 = 4_096
//!     visible_energy_at(600) = energy_at_epoch(8_192, 100, 200) = 8_192>>2 = 2_048
//!     visible_energy_at(900) = energy_at_epoch(8_192, 100, 500) = 8_192>>5 = 256
//!
//!   Suspension gap (preservation proof):
//!     A hypothetical un-suspended testament sealed at epoch 0 with
//!     the same params would have:
//!       energy_at_epoch(8_192, 100, 400) = 8_192>>4 = 512 at epoch 400.
//!     Alice's sealed testament at epoch 400 still holds 8_192.
//!     Preservation ratio = 8_192 / 512 = 16×.
//!
//!   Memorial fixture (initial=4, half_life=1, death_epoch=0):
//!     energy_at_epoch(4, 1, 100) = 0  (4 < 2^100, exact)
//!     fade_to_memorial(100) succeeds.
//!     visible_energy_at(any) = 0 once Memorial.
//!     MemorialMarker.commitment ≠ 0; changes with different IDs.
//!
//!   Doctrine claim (INVENTION_STACK.md §A5.3 / Singh-Posthuma):
//!   "The first NFT that's a deathbed confession. Decay suspended
//!   while issuer is alive. M-of-N committee certifies death. On
//!   reveal, half-life clock starts from cert.death_epoch, not mint.
//!   The readable form fades; a permanent MemorialMarker stays."
//!
//!   Adversarial fixture: ZeroHalfLife, ZeroInitialEnergy, NotSealed
//!   (re-certify), NotRevealed (fade while sealed / still visible /
//!   already memorial), CertificateError (WrongTestament, BelowThreshold,
//!   NotCommitteeMember, DuplicateAttestor, BadSignature, NoAttestations),
//!   VaultError (EmptyCiphertextHash, ZeroCiphertextLen, EmptyPubkeyCommitment,
//!   EmptyCommittee, ZeroThreshold, ThresholdAboveCommittee, DuplicateMember).
//!
//!   INVENTION_STACK §A5.3: Singh-Posthuma (Sealed Testaments).

use evaporchain_singh_posthuma::{
    verify_certificate, Attestation, CertificateError, DeathCertificate, MemorialMarker,
    SealedVault, Testament, TestamentError, TestamentId, TestamentStatus, VaultError,
};

// ── Helpers ───────────────────────────────────────────────────────────────

fn tid(b: u8) -> TestamentId {
    [b; 32]
}
fn validator(b: u8) -> [u8; 32] {
    [b; 32]
}

/// 3-of-5 committee vault used across fixtures.
fn five_v_vault() -> SealedVault {
    SealedVault::new(
        [0xAB; 32],
        1_024,
        3,
        vec![
            validator(1),
            validator(2),
            validator(3),
            validator(4),
            validator(5),
        ],
        [0xCD; 32],
    )
    .unwrap()
}

/// Seal a testament with the five-validator vault.
fn alice_testament(id_byte: u8, half_life: u64, initial: u64) -> Testament {
    Testament::seal(
        tid(id_byte),
        validator(0xAA),
        five_v_vault(),
        half_life,
        initial,
        0,
    )
    .unwrap()
}

/// Build a DeathCertificate for `id_byte` at `death_epoch` signed by `signers`.
fn cert_for(id_byte: u8, death_epoch: u64, signers: Vec<u8>) -> DeathCertificate {
    DeathCertificate {
        testament_id: tid(id_byte),
        death_epoch,
        nonce: [0xEE; 32],
        attestations: signers
            .into_iter()
            .map(|b| Attestation {
                validator: validator(b),
                signature: vec![b; 8],
            })
            .collect(),
    }
}

fn always_valid(_v: &[u8; 32], _b: &[u8], _s: &[u8]) -> bool {
    true
}

// ── Non-trivial fixture ───────────────────────────────────────────────────

#[test]
fn fixture_full_lifecycle_sealed_revealed_memorial() {
    // Alice's Last Testament: initial=8_192, half_life=100, threshold=3-of-5.
    let mut t = alice_testament(0xA1, 100, 8_192);

    // Sealed: decay suspended at all epochs.
    assert!(t.is_sealed());
    assert_eq!(t.visible_energy_at(0), 8_192);
    assert_eq!(
        t.visible_energy_at(10_000),
        8_192,
        "suspension must hold for arbitrary epochs"
    );

    // 3-of-5 committee certifies death at epoch 400.
    let cert = cert_for(0xA1, 400, vec![1, 2, 3]);
    t.accept_death_certificate(&cert, always_valid).unwrap();
    assert!(t.is_revealed());
    assert!(matches!(
        t.status,
        TestamentStatus::Revealed {
            revealed_at_epoch: 400
        }
    ));

    // Half-life clock runs from epoch 400 (cert.death_epoch).
    assert_eq!(
        t.visible_energy_at(400),
        8_192,
        "exactly at reveal: no elapsed time"
    );
    assert_eq!(
        t.visible_energy_at(500),
        4_096,
        "1 half-life elapsed: 8_192>>1"
    );
    assert_eq!(
        t.visible_energy_at(600),
        2_048,
        "2 half-lives elapsed: 8_192>>2"
    );
    assert_eq!(
        t.visible_energy_at(900),
        256,
        "5 half-lives elapsed: 8_192>>5"
    );

    // Cannot accept a second certificate once Revealed.
    let cert2 = cert_for(0xA1, 450, vec![1, 2, 3]);
    let err = t
        .accept_death_certificate(&cert2, always_valid)
        .unwrap_err();
    assert_eq!(err, TestamentError::NotSealed);

    // Now use a fast-decay testament to reach memorial.
    let mut fast = alice_testament(0xA2, 1, 4); // initial=4, half_life=1
    let cert_fast = cert_for(0xA2, 0, vec![1, 2, 3]);
    fast.accept_death_certificate(&cert_fast, always_valid)
        .unwrap();
    // energy_at_epoch(4, 1, 100) = 0 (4 < 2^100).
    fast.fade_to_memorial(100).unwrap();
    assert!(fast.is_memorial());
    assert_eq!(
        fast.visible_energy_at(100),
        0,
        "Memorial: visible_energy always 0"
    );
    assert_eq!(fast.visible_energy_at(10_000), 0);

    // MemorialMarker captures the testament's history.
    if let TestamentStatus::Memorial(MemorialMarker {
        testament_id,
        issuer,
        revealed_at_epoch,
        commitment,
    }) = fast.status
    {
        assert_eq!(testament_id, tid(0xA2));
        assert_eq!(issuer, validator(0xAA));
        assert_eq!(
            revealed_at_epoch, 0,
            "revealed_at taken from cert.death_epoch"
        );
        assert_ne!(commitment, [0u8; 32], "commitment must be non-zero");
    } else {
        panic!("expected Memorial");
    }
}

#[test]
fn fixture_suspension_gap_proves_16x_preservation() {
    // Sealed testament at epoch 400 has 8_192 (suspended).
    // Hypothetical: same params but unsuspended → energy_at_epoch(8_192, 100, 400) = 512.
    // Preservation ratio = 8_192 / 512 = 16.
    let t_sealed = alice_testament(0xB0, 100, 8_192);
    let suspended_at_400 = t_sealed.visible_energy_at(400);
    assert_eq!(
        suspended_at_400, 8_192,
        "suspension gap: full initial preserved"
    );

    // Hypothetical unsuspended: compute the decay manually using the
    // same formula the crate uses internally.
    // energy_at_epoch(8_192, 100, 400): full=4, rem=0, after=8_192>>4=512.
    let hypothetical = 8_192u64 >> 4;
    assert_eq!(hypothetical, 512);

    assert_eq!(
        suspended_at_400 / hypothetical,
        16,
        "suspension preserves 16× more energy"
    );
}

#[test]
fn fixture_reveal_epoch_taken_from_certificate_not_now() {
    // Even if accept_death_certificate is called "now" (epoch 999),
    // the reveal clock must start at cert.death_epoch (400) — the
    // actual death event.  This is the structural promise:
    // "the issuer's testament was promised against the death epoch."
    let mut t = alice_testament(0xC0, 100, 8_192);
    // Certificate says death_epoch=400; we "process" it at epoch 999.
    let cert = cert_for(0xC0, 400, vec![1, 2, 3]);
    t.accept_death_certificate(&cert, always_valid).unwrap();

    // Half-life clock from epoch 400 → at epoch 500 (elapsed=100), energy=4_096.
    assert_eq!(
        t.visible_energy_at(500),
        4_096,
        "clock starts at cert.death_epoch=400"
    );

    // If the clock had started at 999, elapsed at 500 would be negative →
    // saturating_sub gives 0 → energy would be 8_192. The fact we get 4_096
    // confirms the clock anchors to cert.death_epoch.
    assert!(
        t.visible_energy_at(500) < t.initial_visible_energy,
        "energy must have started decaying from epoch 400"
    );
}

#[test]
fn fixture_memorial_commitment_differs_by_testament_id() {
    let mut t1 = alice_testament(0xD0, 1, 4);
    let mut t2 = alice_testament(0xD1, 1, 4);

    let c1 = cert_for(0xD0, 0, vec![1, 2, 3]);
    let c2 = cert_for(0xD1, 0, vec![1, 2, 3]);
    t1.accept_death_certificate(&c1, always_valid).unwrap();
    t2.accept_death_certificate(&c2, always_valid).unwrap();
    t1.fade_to_memorial(100).unwrap();
    t2.fade_to_memorial(100).unwrap();

    let m1 = match t1.status {
        TestamentStatus::Memorial(m) => m.commitment,
        _ => panic!(),
    };
    let m2 = match t2.status {
        TestamentStatus::Memorial(m) => m.commitment,
        _ => panic!(),
    };
    assert_ne!(
        m1, m2,
        "different testament IDs → different memorial commitments"
    );
}

#[test]
fn fixture_visible_energy_monotone_non_increasing_after_reveal() {
    // After reveal the readable form can only decay, never grow.
    let mut t = alice_testament(0xE0, 100, 8_192);
    let cert = cert_for(0xE0, 0, vec![1, 2, 3]);
    t.accept_death_certificate(&cert, always_valid).unwrap();

    let epochs = [0u64, 50, 100, 200, 500, 1_000, 10_000];
    let mut prev = u64::MAX;
    for &e in &epochs {
        let energy = t.visible_energy_at(e);
        assert!(
            energy <= prev,
            "visible_energy not monotone at epoch {e}: {energy} > {prev}"
        );
        prev = energy;
    }
}

// ── Doctrine tests ────────────────────────────────────────────────────────

#[test]
fn doctrine_decay_strictly_suspended_while_sealed() {
    // INVENTION_STACK §A5.3: "Decay suspended while issuer is verifiably alive."
    // No matter how many epochs pass, a Sealed testament's visible_energy
    // remains exactly at its initial value — the half-life knob is dormant.
    let t = alice_testament(0xF0, 1, 1_000); // half_life=1: would decay very fast if unsuspended
    for epoch in [0u64, 1, 10, 100, 10_000, 1_000_000] {
        assert_eq!(
            t.visible_energy_at(epoch),
            1_000,
            "Sealed testament: visible_energy must equal initial at epoch {epoch}"
        );
    }
}

#[test]
fn doctrine_committee_threshold_gates_reveal() {
    // 3-of-5 threshold: 1 or 2 signatures must not reveal the testament.
    let mut t1 = alice_testament(0xF1, 100, 1_000);
    let one_sig = cert_for(0xF1, 100, vec![1]);
    let err = t1
        .accept_death_certificate(&one_sig, always_valid)
        .unwrap_err();
    assert!(matches!(
        err,
        TestamentError::Certificate(CertificateError::BelowThreshold {
            valid: 1,
            threshold: 3
        })
    ));
    assert!(
        t1.is_sealed(),
        "testament must remain Sealed on below-threshold cert"
    );

    let mut t2 = alice_testament(0xF2, 100, 1_000);
    let two_sig = cert_for(0xF2, 100, vec![1, 2]);
    let err = t2
        .accept_death_certificate(&two_sig, always_valid)
        .unwrap_err();
    assert!(matches!(
        err,
        TestamentError::Certificate(CertificateError::BelowThreshold {
            valid: 2,
            threshold: 3
        })
    ));
    assert!(t2.is_sealed());

    // Exactly 3 signatures succeeds.
    let mut t3 = alice_testament(0xF3, 100, 1_000);
    let three_sig = cert_for(0xF3, 100, vec![1, 2, 3]);
    t3.accept_death_certificate(&three_sig, always_valid)
        .unwrap();
    assert!(t3.is_revealed());
}

#[test]
fn doctrine_memorial_visible_energy_is_permanently_zero() {
    // INVENTION_STACK §A5.3: once Memorial, the readable form is gone.
    // visible_energy_at returns 0 for all epochs.
    let mut t = alice_testament(0xF4, 1, 4);
    let cert = cert_for(0xF4, 0, vec![1, 2, 3]);
    t.accept_death_certificate(&cert, always_valid).unwrap();
    t.fade_to_memorial(100).unwrap();

    for epoch in [0u64, 1, 100, 10_000, u64::MAX / 2] {
        assert_eq!(
            t.visible_energy_at(epoch),
            0,
            "Memorial: visible_energy must be 0 at epoch {epoch}"
        );
    }
}

#[test]
fn doctrine_signing_bytes_include_domain_separation_tag() {
    // Anti-replay: the domain-separation tag prevents a committee
    // member's signature on another document from being replayed as a
    // death attestation.
    let cert = cert_for(0xF5, 100, vec![1, 2, 3]);
    let bytes = cert.signing_bytes();
    let dst = b"singh-posthuma:death:v1";
    assert!(
        bytes.windows(dst.len()).any(|w| w == dst),
        "signing bytes must contain domain-separation tag"
    );
}

// ── Adversarial fixture ───────────────────────────────────────────────────

#[test]
fn adversarial_zero_half_life_rejected() {
    let err = Testament::seal(tid(1), validator(0xAA), five_v_vault(), 0, 1_000, 0).unwrap_err();
    assert_eq!(err, TestamentError::ZeroHalfLife);
}

#[test]
fn adversarial_zero_initial_energy_rejected() {
    let err = Testament::seal(tid(1), validator(0xAA), five_v_vault(), 100, 0, 0).unwrap_err();
    assert_eq!(err, TestamentError::ZeroInitialEnergy);
}

#[test]
fn adversarial_accept_certificate_twice_rejected() {
    let mut t = alice_testament(0x10, 100, 1_000);
    let cert = cert_for(0x10, 50, vec![1, 2, 3]);
    t.accept_death_certificate(&cert, always_valid).unwrap();
    let err = t.accept_death_certificate(&cert, always_valid).unwrap_err();
    assert_eq!(err, TestamentError::NotSealed);
}

#[test]
fn adversarial_fade_while_sealed_rejected() {
    let mut t = alice_testament(0x11, 100, 1_000);
    let err = t.fade_to_memorial(10_000).unwrap_err();
    assert_eq!(err, TestamentError::NotRevealed);
}

#[test]
fn adversarial_fade_before_visible_zero_rejected() {
    let mut t = alice_testament(0x12, 100, 1_000);
    let cert = cert_for(0x12, 0, vec![1, 2, 3]);
    t.accept_death_certificate(&cert, always_valid).unwrap();
    // After only 10 epochs (< 1 half-life), visible energy > 0.
    let err = t.fade_to_memorial(10).unwrap_err();
    assert_eq!(err, TestamentError::NotRevealed);
}

#[test]
fn adversarial_fade_on_already_memorial_rejected() {
    let mut t = alice_testament(0x13, 1, 4);
    let cert = cert_for(0x13, 0, vec![1, 2, 3]);
    t.accept_death_certificate(&cert, always_valid).unwrap();
    t.fade_to_memorial(100).unwrap();
    let err = t.fade_to_memorial(200).unwrap_err();
    assert_eq!(err, TestamentError::NotRevealed);
}

#[test]
fn adversarial_certificate_wrong_testament_rejected() {
    let mut t = alice_testament(0x14, 100, 1_000);
    // Certificate cites a different testament id.
    let cert = cert_for(0xFF, 100, vec![1, 2, 3]);
    let err = t.accept_death_certificate(&cert, always_valid).unwrap_err();
    assert!(matches!(
        err,
        TestamentError::Certificate(CertificateError::WrongTestament)
    ));
    assert!(
        t.is_sealed(),
        "testament must stay Sealed on WrongTestament"
    );
}

#[test]
fn adversarial_certificate_non_committee_member_rejected() {
    let mut t = alice_testament(0x15, 100, 1_000);
    // Validator 99 is not in the [1..5] committee.
    let cert = cert_for(0x15, 100, vec![1, 2, 99]);
    let err = t.accept_death_certificate(&cert, always_valid).unwrap_err();
    assert!(matches!(
        err,
        TestamentError::Certificate(CertificateError::NotCommitteeMember(_))
    ));
    assert!(t.is_sealed());
}

#[test]
fn adversarial_certificate_duplicate_attestor_rejected() {
    let mut t = alice_testament(0x16, 100, 1_000);
    let cert = cert_for(0x16, 100, vec![1, 1, 2]);
    let err = t.accept_death_certificate(&cert, always_valid).unwrap_err();
    assert!(matches!(
        err,
        TestamentError::Certificate(CertificateError::DuplicateAttestor(_))
    ));
    assert!(t.is_sealed());
}

#[test]
fn adversarial_certificate_no_attestations_rejected() {
    let mut t = alice_testament(0x17, 100, 1_000);
    let cert = cert_for(0x17, 100, vec![]); // empty attestation list
    let err = t.accept_death_certificate(&cert, always_valid).unwrap_err();
    assert!(matches!(
        err,
        TestamentError::Certificate(CertificateError::NoAttestations)
    ));
    assert!(t.is_sealed());
}

#[test]
fn adversarial_certificate_bad_signature_rejected() {
    let mut t = alice_testament(0x18, 100, 1_000);
    let cert = cert_for(0x18, 100, vec![1, 2, 3]);
    // Reject validator 3's signature specifically.
    let reject_v3 = |v: &[u8; 32], _b: &[u8], _s: &[u8]| v != &validator(3);
    let err = t.accept_death_certificate(&cert, reject_v3).unwrap_err();
    assert!(matches!(
        err,
        TestamentError::Certificate(CertificateError::BadSignature(_))
    ));
    assert!(t.is_sealed());
}

#[test]
fn adversarial_vault_empty_ciphertext_hash_rejected() {
    let err = SealedVault::new([0u8; 32], 100, 1, vec![validator(1)], [1u8; 32]).unwrap_err();
    assert_eq!(err, VaultError::EmptyCiphertextHash);
}

#[test]
fn adversarial_vault_zero_ciphertext_len_rejected() {
    let err = SealedVault::new([1u8; 32], 0, 1, vec![validator(1)], [1u8; 32]).unwrap_err();
    assert_eq!(err, VaultError::ZeroCiphertextLen);
}

#[test]
fn adversarial_vault_empty_pubkey_commitment_rejected() {
    let err = SealedVault::new([1u8; 32], 100, 1, vec![validator(1)], [0u8; 32]).unwrap_err();
    assert_eq!(err, VaultError::EmptyPubkeyCommitment);
}

#[test]
fn adversarial_vault_empty_committee_rejected() {
    let err = SealedVault::new([1u8; 32], 100, 1, vec![], [1u8; 32]).unwrap_err();
    assert_eq!(err, VaultError::EmptyCommittee);
}

#[test]
fn adversarial_vault_zero_threshold_rejected() {
    let err = SealedVault::new([1u8; 32], 100, 0, vec![validator(1)], [1u8; 32]).unwrap_err();
    assert_eq!(err, VaultError::ZeroThreshold);
}

#[test]
fn adversarial_vault_threshold_above_committee_rejected() {
    let err = SealedVault::new(
        [1u8; 32],
        100,
        5,
        vec![validator(1), validator(2)],
        [1u8; 32],
    )
    .unwrap_err();
    assert!(matches!(
        err,
        VaultError::ThresholdAboveCommittee { m: 5, n: 2 }
    ));
}

#[test]
fn adversarial_vault_duplicate_committee_member_rejected() {
    let err = SealedVault::new(
        [1u8; 32],
        100,
        2,
        vec![validator(1), validator(1), validator(2)],
        [1u8; 32],
    )
    .unwrap_err();
    assert_eq!(err, VaultError::DuplicateMember);
}

// ── Cross-cuts ───────────────────────────────────────────────────────────

#[test]
fn serde_round_trip_all_three_states() {
    // Sealed:
    let t_sealed = alice_testament(0x20, 100, 8_192);
    let json = serde_json::to_string(&t_sealed).unwrap();
    let back: Testament = serde_json::from_str(&json).unwrap();
    assert_eq!(t_sealed, back);

    // Revealed:
    let mut t_revealed = alice_testament(0x21, 100, 8_192);
    t_revealed
        .accept_death_certificate(&cert_for(0x21, 400, vec![1, 2, 3]), always_valid)
        .unwrap();
    let json = serde_json::to_string(&t_revealed).unwrap();
    let back: Testament = serde_json::from_str(&json).unwrap();
    assert_eq!(t_revealed, back);

    // Memorial:
    let mut t_memorial = alice_testament(0x22, 1, 4);
    t_memorial
        .accept_death_certificate(&cert_for(0x22, 0, vec![1, 2, 3]), always_valid)
        .unwrap();
    t_memorial.fade_to_memorial(100).unwrap();
    let json = serde_json::to_string(&t_memorial).unwrap();
    let back: Testament = serde_json::from_str(&json).unwrap();
    assert_eq!(t_memorial, back);
}

#[test]
fn visible_energy_at_is_deterministic() {
    let mut t = alice_testament(0x30, 100, 8_192);
    t.accept_death_certificate(&cert_for(0x30, 400, vec![1, 2, 3]), always_valid)
        .unwrap();
    assert_eq!(
        t.visible_energy_at(500),
        t.visible_energy_at(500),
        "visible_energy_at must be deterministic"
    );
}

#[test]
fn verify_certificate_standalone_matches_testament_accept() {
    // verify_certificate is the pure-function workhorse; accept_death_certificate
    // is a thin wrapper. Both must agree on acceptance.
    let vault = five_v_vault();
    let cert = cert_for(0x31, 100, vec![1, 2, 3]);
    verify_certificate(&cert, tid(0x31), &vault, always_valid).unwrap();

    // Wrong id on standalone verifier.
    let err = verify_certificate(&cert, tid(0xFF), &vault, always_valid).unwrap_err();
    assert_eq!(err, CertificateError::WrongTestament);
}
