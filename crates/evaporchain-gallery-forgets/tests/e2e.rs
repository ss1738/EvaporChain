//! End-to-end integration tests for evaporchain-gallery-forgets.
//!
//! Non-trivial fixture: a four-artist Disintegration Show where exhibits
//! of varying half-lives age independently and the gallery tracks its
//! own thermodynamic closing date through all three lifecycle states.
//!
//! Doctrine claim (INVENTION_STACK §A2.3 — The Gallery That Forgets):
//!   "One launch artifact, three primitives:
//!   (1) Provably-Mortal NFTs (Mayflies) — minted with declared
//!       half-life + ZK death certificate. Wallet shows countdown.
//!   (2) Decay-as-Performance-Art — gallery closing date is
//!       *thermodynamic*, not calendrical.
//!   (3) AI-Decay-Art — single f64 seed scalar derived from alive
//!       energy; output literally changes as state evaporates.
//!       Basinski's Disintegration Loops on-chain.
//!   The single sentence: 'It is the first thing humans have made
//!   that is provably going to die.'"
//!
//! Disintegration Show — four exhibits deposited at epoch 0:
//!
//!   DALI_WORK  [initial=4,             half_life=1]
//!     actual death epoch=3  (4 >> 3 = 0)
//!     projected cert epoch=5
//!
//!   CLAUDE_WORK [initial=64,            half_life=2]
//!     actual death epoch=14 (64 >> 7 = 0)
//!     projected cert epoch=18
//!
//!   BASINSKI   [initial=1_000,         half_life=10]
//!     actual death epoch=100 (1000 >> 10 = 0)
//!     projected cert epoch=120
//!
//!   ARTEMIS    [initial=1_000_000_000, half_life=1_000]
//!     actual death epoch=30_000 (1B >> 30 = 0)
//!     projected cert epoch=32_000
//!
//!   Gallery timeline (actual deaths, not projected):
//!     Epoch 2:      all 4 alive (DALI score=1)        → Open,    alive=4
//!     Epoch 3:      DALI dead (4>>3=0)                → Closing, alive=3
//!     Epoch 14:     DALI+CLAUDE dead (64>>7=0)        → Closing, alive=2
//!     Epoch 100:    +BASINSKI dead (1000>>10=0)       → Closing, alive=1
//!     Epoch 30_000: ARTEMIS dead (1B>>30=0)           → Closed,  alive=0
//!
//!   Thermodynamic close = max(projected cert epochs) = 32_000.

use evaporchain_gallery_forgets::{
    runtime_seed, Gallery, GalleryError, GalleryStatus, MayflyError, MayflyToken,
};
use evaporchain_types::AccountAddress;

// ── Fixture constants ─────────────────────────────────────────────────────────

fn addr(b: u8) -> AccountAddress {
    [b; 32]
}
fn tid(b: u8) -> [u8; 32] {
    [b; 32]
}
fn eid(b: u8) -> [u8; 32] {
    [b; 32]
}

const CURATOR: u8 = 0xC0;

// Exhibit parameters.
const DALI_INIT: u64 = 4;
const DALI_HL: u64 = 1;
const CLAUDE_INIT: u64 = 64;
const CLAUDE_HL: u64 = 2;
const BASINSKI_INIT: u64 = 1_000;
const BASINSKI_HL: u64 = 10;
const ARTEMIS_INIT: u64 = 1_000_000_000;
const ARTEMIS_HL: u64 = 1_000;

// Actual death epochs (first epoch where score == 0).
const DALI_ACTUAL: u64 = 3; // 4  >> 3  = 0
const CLAUDE_ACTUAL: u64 = 14; // 64 >> 7  = 0
const BASINSKI_ACTUAL: u64 = 100; // 1000 >> 10 = 0
const ARTEMIS_ACTUAL: u64 = 30_000; // 1B >> 30 = 0

// Certificate (worst-case projected) death epochs — the gallery's
// thermodynamic close is max of these = 32_000.
const THERMO_CLOSE: u64 = 32_000;

fn disintegration_gallery() -> Gallery {
    let mut g = Gallery::open(tid(0xFF), "The Disintegration Show", addr(CURATOR), 0);
    let c = addr(CURATOR);
    g.deposit(
        c,
        eid(0xD1),
        addr(0xA1),
        MayflyToken::mint(tid(0xD1), addr(0xA1), DALI_INIT, DALI_HL, 0).unwrap(),
        0,
    )
    .unwrap();
    g.deposit(
        c,
        eid(0xC2),
        addr(0xA2),
        MayflyToken::mint(tid(0xC2), addr(0xA2), CLAUDE_INIT, CLAUDE_HL, 0).unwrap(),
        0,
    )
    .unwrap();
    g.deposit(
        c,
        eid(0xB3),
        addr(0xA3),
        MayflyToken::mint(tid(0xB3), addr(0xA3), BASINSKI_INIT, BASINSKI_HL, 0).unwrap(),
        0,
    )
    .unwrap();
    g.deposit(
        c,
        eid(0xA4),
        addr(0xA4),
        MayflyToken::mint(tid(0xA4), addr(0xA4), ARTEMIS_INIT, ARTEMIS_HL, 0).unwrap(),
        0,
    )
    .unwrap();
    g
}

// ── Main fixture: Disintegration Show full lifecycle ──────────────────────────

#[test]
fn disintegration_show_open_closing_closed_lifecycle() {
    let g = disintegration_gallery();

    // Epoch 0: gallery just opened, all 4 alive.
    assert_eq!(g.status_at(0), GalleryStatus::Open);
    assert_eq!(g.alive_count(0), 4);
    assert_eq!(g.exhibits.len(), 4);

    // Epoch 2: DALI still alive (4 >> 2 = 1), gallery still Open.
    assert_eq!(
        g.status_at(2),
        GalleryStatus::Open,
        "epoch 2: DALI barely alive, still Open"
    );
    assert_eq!(g.alive_count(2), 4);

    // Epoch 3: DALI dead (4 >> 3 = 0) — first exhibit falls.
    assert_eq!(
        g.status_at(DALI_ACTUAL),
        GalleryStatus::Closing,
        "epoch {DALI_ACTUAL}: DALI crosses 0, gallery transitions Open → Closing"
    );
    assert_eq!(g.alive_count(DALI_ACTUAL), 3);

    // Epoch 14: CLAUDE_WORK dead (64 >> 7 = 0), BASINSKI + ARTEMIS survive.
    assert_eq!(g.status_at(CLAUDE_ACTUAL), GalleryStatus::Closing);
    assert_eq!(g.alive_count(CLAUDE_ACTUAL), 2);

    // Epoch 100: BASINSKI dead (1000 >> 10 = 0), only ARTEMIS survives.
    assert_eq!(g.status_at(BASINSKI_ACTUAL), GalleryStatus::Closing);
    assert_eq!(g.alive_count(BASINSKI_ACTUAL), 1);

    // Epoch 30_000: ARTEMIS finally dies (1B >> 30 = 0) — gallery fully Closed.
    assert_eq!(
        g.status_at(ARTEMIS_ACTUAL),
        GalleryStatus::Closed,
        "epoch {ARTEMIS_ACTUAL}: ARTEMIS crosses 0, gallery is Closed"
    );
    assert_eq!(g.alive_count(ARTEMIS_ACTUAL), 0);

    // Thermodynamic close: gallery is guaranteed Closed by epoch 32_000.
    assert_eq!(g.thermodynamic_close_epoch().unwrap(), THERMO_CLOSE);
    assert_eq!(g.status_at(THERMO_CLOSE), GalleryStatus::Closed);
}

// ── Thermodynamic close: certificate is always a valid upper bound ────────────

#[test]
fn thermodynamic_close_epoch_upper_bounds_actual_closure_for_every_exhibit() {
    let g = disintegration_gallery();

    let close = g.thermodynamic_close_epoch().unwrap();
    assert_eq!(
        close, THERMO_CLOSE,
        "thermodynamic close = max projected cert epoch"
    );

    // The gallery is definitely Closed at the thermodynamic close.
    assert_eq!(g.status_at(close), GalleryStatus::Closed);

    // Each exhibit is dead by its own projected cert epoch (upper bound guarantee).
    for ex in &g.exhibits {
        let proj = ex.mayfly.certificate.projected_death_epoch;
        assert!(
            ex.mayfly.is_dead(proj),
            "exhibit {:?} must be dead by its cert epoch {proj}",
            ex.id
        );
    }
}

// ── Pre-certified death: certificate exists before any decay ──────────────────

#[test]
fn mayfly_death_certificate_pre_issued_before_any_decay() {
    let m = MayflyToken::mint(tid(0x01), addr(0xAA), BASINSKI_INIT, BASINSKI_HL, 0).unwrap();

    // At epoch 0: alive, full energy — but the cert already promises a death epoch.
    assert!(!m.is_dead(0), "token must be alive at mint");
    assert_eq!(m.score_at(0), BASINSKI_INIT);
    let promised = m.certificate.projected_death_epoch;
    assert!(
        promised > 0,
        "projected death epoch must be strictly in the future"
    );

    // Certificate verifies cryptographically from the token's stored fields alone.
    assert!(
        m.verify_certificate(),
        "certificate must verify against mint inputs before any decay"
    );

    // And the token is provably dead by the promised epoch.
    assert!(
        m.is_dead(promised),
        "token must be dead by cert epoch {promised} — the certificate is honored"
    );
}

// ── AI seed: full monotone lifecycle from 1.0 to 0.0 ────────────────────────

#[test]
fn ai_seed_full_lifecycle_one_to_zero() {
    // Single exhibit: initial=1000, half_life=10.
    //   epoch  0: score=1000, seed=1.0
    //   epoch 10: score= 500, seed=0.5
    //   epoch 20: score= 250, seed=0.25
    //   epoch 100: score=0,   seed=0.0  (1000 >> 10 = 0)
    let mut g = Gallery::open(tid(0xEE), "Seed Gallery", addr(CURATOR), 0);
    let m = MayflyToken::mint(tid(0x10), addr(0xAA), 1_000, 10, 0).unwrap();
    g.deposit(addr(CURATOR), eid(0x10), addr(0xAA), m, 0)
        .unwrap();

    let s0 = runtime_seed(&g, 0);
    assert_eq!(s0.seed, 1.0, "at deposit epoch, seed = 1.0 (full energy)");

    let s1 = runtime_seed(&g, 10);
    assert_eq!(s1.seed, 0.5, "after one half-life (epoch 10), seed = 0.5");

    let s2 = runtime_seed(&g, 20);
    assert_eq!(
        s2.seed, 0.25,
        "after two half-lives (epoch 20), seed = 0.25"
    );

    let s_dead = runtime_seed(&g, 100);
    assert_eq!(
        s_dead.seed, 0.0,
        "after all energy gone, seed = 0.0 (silence)"
    );

    // Monotone non-increasing.
    let checkpoints = [s0, s1, s2, s_dead];
    for w in checkpoints.windows(2) {
        assert!(
            w[1].seed <= w[0].seed,
            "AI seed must be monotone non-increasing: {} → {}",
            w[0].seed,
            w[1].seed
        );
    }
}

// ── Dead Mayfly blocks transfer ───────────────────────────────────────────────

#[test]
fn dead_mayfly_blocks_transfer_alive_permits() {
    // Alive Mayfly: transfers succeed.
    let mut live = MayflyToken::mint(tid(0x01), addr(0xAA), 1_000, 100, 0).unwrap();
    live.transfer(addr(0xAA), addr(0xBB), 0).unwrap();
    assert_eq!(live.owner, addr(0xBB));

    // Dead Mayfly: transfer rejected.
    let mut dead = MayflyToken::mint(tid(0x02), addr(0xAA), DALI_INIT, DALI_HL, 0).unwrap();
    let err = dead
        .transfer(addr(0xAA), addr(0xBB), DALI_ACTUAL)
        .unwrap_err();
    assert!(
        matches!(err, MayflyError::Died),
        "transfer of a dead Mayfly must yield Died error"
    );

    // Owner is unchanged after the failed transfer.
    assert_eq!(
        dead.owner,
        addr(0xAA),
        "failed transfer must leave owner unchanged"
    );
}

// ── Tampered certificate rejected at gallery deposit ──────────────────────────

#[test]
fn tampered_certificate_rejected_at_gallery_deposit() {
    let mut g = Gallery::open(tid(0xEE), "Gallery", addr(CURATOR), 0);
    let mut m = MayflyToken::mint(tid(0x01), addr(0xAA), 1_000, 100, 0).unwrap();

    // Flip one byte of the Blake3 commitment.
    m.certificate.commitment[0] ^= 0xFF;
    assert!(
        !m.verify_certificate(),
        "tampered cert must fail verification"
    );

    let err = g
        .deposit(addr(CURATOR), eid(0x01), addr(0xAA), m, 0)
        .unwrap_err();
    assert!(
        matches!(err, GalleryError::InvalidCertificate),
        "gallery must reject exhibit with tampered certificate"
    );
}

// ── Non-curator deposit rejected ──────────────────────────────────────────────

#[test]
fn non_curator_deposit_access_control() {
    let mut g = Gallery::open(tid(0xEE), "Gallery", addr(CURATOR), 0);
    let m = MayflyToken::mint(tid(0x01), addr(0xAA), 1_000, 100, 0).unwrap();

    let err = g
        .deposit(addr(0xBB), eid(0x01), addr(0xAA), m, 0)
        .unwrap_err();
    assert!(
        matches!(err, GalleryError::NotCurator { .. }),
        "only the gallery curator may deposit exhibits"
    );
}

// ── AI seed is energy-weighted, not exhibit-count-weighted ───────────────────

#[test]
fn ai_seed_energy_weighted_big_exhibit_dominates() {
    // Giant exhibit (1B energy, very long half-life) + tiny one (4 energy, hl=1).
    // When the tiny one dies (epoch DALI_ACTUAL=3), the seed barely moves
    // because the giant contributes >99.9999% of total initial energy.
    let mut g = Gallery::open(tid(0xEE), "Gallery", addr(CURATOR), 0);
    let giant = MayflyToken::mint(tid(0x01), addr(0xAA), ARTEMIS_INIT, 1_000_000, 0).unwrap();
    let tiny = MayflyToken::mint(tid(0x02), addr(0xBB), DALI_INIT, DALI_HL, 0).unwrap();
    g.deposit(addr(CURATOR), eid(0x01), addr(0xAA), giant, 0)
        .unwrap();
    g.deposit(addr(CURATOR), eid(0x02), addr(0xBB), tiny, 0)
        .unwrap();

    // At epoch 3: tiny is dead (4>>3=0), giant untouched (0 halvings in 3 epochs).
    let snap = runtime_seed(&g, DALI_ACTUAL);
    assert!(snap.seed > 0.999,
        "energy-weighted seed must be ≈1.0 when 1B-energy giant dominates over dead 4-energy tiny; got {}",
        snap.seed);
}

// ── Two galleries have independent thermodynamic clocks ──────────────────────

#[test]
fn two_galleries_have_independent_thermodynamic_clocks() {
    let mut ga = Gallery::open(tid(0xA0), "Gallery A", addr(0xCA), 0);
    ga.deposit(
        addr(0xCA),
        eid(0xA1),
        addr(0xAA),
        MayflyToken::mint(tid(0xA1), addr(0xAA), DALI_INIT, DALI_HL, 0).unwrap(),
        0,
    )
    .unwrap();

    let mut gb = Gallery::open(tid(0xB0), "Gallery B", addr(0xCB), 0);
    gb.deposit(
        addr(0xCB),
        eid(0xB1),
        addr(0xBB),
        MayflyToken::mint(tid(0xB1), addr(0xBB), ARTEMIS_INIT, ARTEMIS_HL, 0).unwrap(),
        0,
    )
    .unwrap();

    let close_a = ga.thermodynamic_close_epoch().unwrap();
    let close_b = gb.thermodynamic_close_epoch().unwrap();
    assert!(
        close_a < close_b,
        "Gallery A (short-lived) must close thermodynamically before Gallery B"
    );

    // At A's close: A closed, B fully open.
    assert_eq!(ga.status_at(close_a), GalleryStatus::Closed);
    assert_eq!(
        gb.status_at(close_a),
        GalleryStatus::Open,
        "Gallery B must be Open — its exhibit is unaffected by Gallery A's closing"
    );

    // At B's close: both closed.
    assert_eq!(gb.status_at(close_b), GalleryStatus::Closed);
    assert_eq!(
        ga.status_at(close_b),
        GalleryStatus::Closed,
        "Gallery A was already Closed long before B's close"
    );
}

// ── Alive count tracks surviving exhibits exactly ─────────────────────────────

#[test]
fn alive_count_tracks_surviving_exhibits_exactly() {
    let g = disintegration_gallery();

    assert_eq!(
        g.alive_count(2),
        4,
        "epoch 2: DALI barely alive (4>>2=1) — all 4 exhibits alive"
    );
    assert_eq!(
        g.alive_count(DALI_ACTUAL),
        3,
        "epoch {DALI_ACTUAL}: DALI dead (4>>3=0), 3 remaining"
    );
    assert_eq!(
        g.alive_count(CLAUDE_ACTUAL),
        2,
        "epoch {CLAUDE_ACTUAL}: CLAUDE dead (64>>7=0), 2 remaining"
    );
    assert_eq!(
        g.alive_count(BASINSKI_ACTUAL),
        1,
        "epoch {BASINSKI_ACTUAL}: BASINSKI dead (1000>>10=0), 1 remaining"
    );
    assert_eq!(
        g.alive_count(ARTEMIS_ACTUAL),
        0,
        "epoch {ARTEMIS_ACTUAL}: ARTEMIS dead (1B>>30=0), gallery fully silent"
    );
}

// ── Gallery status is deterministic for the same epoch ───────────────────────

#[test]
fn gallery_status_is_deterministic_for_same_epoch() {
    let g = disintegration_gallery();
    for epoch in [
        0u64,
        2,
        DALI_ACTUAL,
        CLAUDE_ACTUAL,
        BASINSKI_ACTUAL,
        ARTEMIS_ACTUAL,
        THERMO_CLOSE,
    ] {
        let a = g.status_at(epoch);
        let b = g.status_at(epoch);
        assert_eq!(a, b, "status_at({epoch}) must be deterministic");
    }
}

// ── "Provably going to die" — full doctrine sentence operationalised ──────────

#[test]
fn provably_mortal_single_sentence_doctrine() {
    // "It is the first thing humans have made that is provably going to die."
    //
    // Operationalised:
    // 1. Certificate exists at mint time (epoch 0) — before any decay.
    // 2. Certificate is cryptographically verifiable by any node.
    // 3. The token is provably dead at the certified epoch.
    // 4. Dead tokens cannot be transferred — the chain enforces it,
    //    with no admin key or extension mechanism.
    let mut m = MayflyToken::mint(tid(0x01), addr(0xAA), BASINSKI_INIT, BASINSKI_HL, 0).unwrap();
    let death_epoch = m.certificate.projected_death_epoch;

    // 1+2: pre-death cert is live and verifiable.
    assert!(!m.is_dead(0), "alive at mint");
    assert_eq!(m.score_at(0), BASINSKI_INIT);
    assert!(
        m.verify_certificate(),
        "certificate verifiable at mint time"
    );
    assert!(death_epoch > 0);

    // 3: provably dead at certified epoch.
    assert!(
        m.is_dead(death_epoch),
        "token provably dead at cert epoch {death_epoch}"
    );

    // 4: chain refuses transfer.
    let err = m.transfer(addr(0xAA), addr(0xBB), death_epoch).unwrap_err();
    assert!(
        matches!(err, MayflyError::Died),
        "chain refuses all transfers after provable death"
    );
}

// ── Open → Closing → Closed is the only possible status progression ──────────

#[test]
fn status_progression_open_then_closing_then_closed_never_reverses() {
    let g = disintegration_gallery();

    // Sample status at increasing epochs; verify it only goes
    // Open → Closing → Closed (never backwards).
    let epochs = [0u64, 1, 2, 3, 14, 50, 100, 10_000, 30_000, 32_000];
    let statuses: Vec<GalleryStatus> = epochs.iter().map(|&e| g.status_at(e)).collect();

    // Once a transition happens, the previous state doesn't recur.
    let mut seen_closing = false;
    let mut seen_closed = false;
    for &s in &statuses {
        match s {
            GalleryStatus::Open => {
                assert!(
                    !seen_closing && !seen_closed,
                    "Open must not appear after Closing or Closed"
                );
            }
            GalleryStatus::Closing => {
                seen_closing = true;
                assert!(!seen_closed, "Closing must not appear after Closed");
            }
            GalleryStatus::Closed => {
                seen_closed = true;
            }
        }
    }
    assert!(seen_closing, "gallery must pass through Closing");
    assert!(seen_closed, "gallery must eventually reach Closed");
}
