//! §Deploy-fee oracle e2e
//!
//! Scenario: "EvaporChain App Store gas quotes" — CAMILLE is a dApp
//! developer comparing deploy costs before committing to a launch. She
//! queries the fee oracle for primitives across all six lanes. The
//! oracle must be deterministic, monotone in complexity, and correctly
//! order lanes by their deployment cost.
//!
//! The suite proves: integer-only math produces validator-identical
//! fees; every primitive's fee ≥ BASE_DEPLOY_FEE; paradigm > NFT;
//! complexity surcharges are proportional and monotone.

use evaporchain_app_templates_engine::{
    init_childkey, init_gallery_forgets, init_mayfly, init_mnemochain, init_sbav, init_sddc,
    init_sfsv, init_sgb, init_singh_heartbeat, init_singh_lineage, init_singh_posthuma,
    init_singh_sabi, init_ssm, init_witnessfit, TypedInit,
};
use evaporchain_app_templates_fees::{base_fee, fee_for, BASE_DEPLOY_FEE};

// ── TypedInit helpers ─────────────────────────────────────────────────────

fn mayfly() -> TypedInit {
    TypedInit::Mayfly(init_mayfly::InitConfig {
        initial_energy: 1_000,
        half_life: 30,
    })
}
fn sabi() -> TypedInit {
    TypedInit::SinghSabi(init_singh_sabi::InitConfig {
        initial_energy: 5_000,
        floor_pct: 15,
        half_life: 365,
    })
}
fn posthuma() -> TypedInit {
    TypedInit::SinghPosthuma(init_singh_posthuma::InitConfig {
        half_life: 365,
        initial_visible_energy: 5_000,
        m_threshold: 3,
        n_committee: 5,
    })
}
fn sddc() -> TypedInit {
    TypedInit::Sddc(init_sddc::InitConfig {
        ceiling: 1_000,
        floor: 100,
        lot_lambda: 50,
        duration_epochs: 500,
    })
}
fn mnemo() -> TypedInit {
    TypedInit::Mnemochain(init_mnemochain::InitConfig {
        initial_energy: 1_000,
        initial_stability: 10,
    })
}
fn gallery() -> TypedInit {
    TypedInit::GalleryForgets(init_gallery_forgets::InitConfig { opened_at_epoch: 0 })
}
fn heartbeat() -> TypedInit {
    TypedInit::SinghHeartbeat(init_singh_heartbeat::InitConfig {
        healthy_bpm: 60,
        alarmed_bpm: 120,
        amber_threshold_bp: 7500,
        red_threshold_bp: 5000,
    })
}
fn witnessfit() -> TypedInit {
    TypedInit::Witnessfit(init_witnessfit::InitConfig {
        half_life: 7,
        boost_bp: 500,
    })
}
fn sbav() -> TypedInit {
    TypedInit::Sbav(init_sbav::InitConfig { reg_count: 4 })
}

fn lineage(n: usize) -> TypedInit {
    let rungs: Vec<_> = (0..n as u64)
        .map(|i| init_singh_lineage::LadderRung {
            days: 30 * (i + 1),
            share_bp: 1000 * (i + 1),
        })
        .collect();
    TypedInit::SinghLineage(init_singh_lineage::InitConfig { ladder: rungs })
}

fn sgb(fragment: &str) -> TypedInit {
    TypedInit::Sgb(init_sgb::InitConfig {
        fragment: fragment.into(),
    })
}

fn ssm(fragment: &str) -> TypedInit {
    TypedInit::Ssm(init_ssm::InitConfig {
        fragment: fragment.into(),
    })
}

fn childkey() -> TypedInit {
    TypedInit::Childkey(init_childkey::InitConfig {
        unlock_age_years: 18,
        epochs_per_year: 365,
        m_threshold: 2,
        n_committee: 3,
    })
}

fn sfsv(addr: &str) -> TypedInit {
    TypedInit::Sfsv(init_sfsv::InitConfig {
        deposit_amount: 500,
        predicate_type: 0,
        release_param: 200,
        future_self: addr.into(),
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn all_primitives_fee_at_or_above_base() {
    // Every primitive pays at least BASE_DEPLOY_FEE. No zero fees.
    for typed in [
        mayfly(),
        sabi(),
        posthuma(),
        sddc(),
        mnemo(),
        gallery(),
        heartbeat(),
        witnessfit(),
        sbav(),
        childkey(),
        lineage(1),
        sgb("SLL"),
        ssm("game"),
    ] {
        let fee = fee_for(&typed);
        assert!(
            fee >= BASE_DEPLOY_FEE,
            "fee must be ≥ BASE_DEPLOY_FEE; got {fee} for {:?}",
            typed
        );
        assert!(fee > 0, "fee must never be zero");
    }
}

#[test]
fn fee_is_deterministic_per_primitive() {
    // Same TypedInit → same fee on every call (validator-deterministic).
    for typed in [mayfly(), sabi(), sddc(), mnemo(), sbav()] {
        assert_eq!(
            fee_for(&typed),
            fee_for(&typed),
            "fee must be deterministic for {:?}",
            typed
        );
    }
}

#[test]
fn paradigm_lane_costs_more_than_nft_lane() {
    // Doctrine: paradigm VM bootstrap > simple NFT deploy.
    let nft = fee_for(&mayfly());
    let paradigm = fee_for(&sgb("SLL"));
    assert!(
        paradigm > nft,
        "paradigm ({paradigm}) must cost more than NFT ({nft})"
    );
}

#[test]
fn marketplace_lane_costs_more_than_consumer_lane() {
    // SDDC (multi-axis state machine) > MnemoChain (consumer card).
    let market = fee_for(&sddc());
    let consumer = fee_for(&mnemo());
    assert!(
        market > consumer,
        "marketplace ({market}) must cost more than consumer ({consumer})"
    );
}

#[test]
fn rich_nft_costs_more_than_basic_nft() {
    // SinghPosthuma (m-of-n testament) > Mayfly (single expiry).
    let basic = fee_for(&mayfly());
    let rich = fee_for(&posthuma());
    assert!(
        rich > basic,
        "rich NFT ({rich}) must cost more than basic NFT ({basic})"
    );
}

#[test]
fn lineage_fee_scales_linearly_with_ladder_rungs() {
    // Each additional rung adds exactly PER_LADDER_RUNG = 100.
    let f1 = fee_for(&lineage(1));
    let f3 = fee_for(&lineage(3));
    let f5 = fee_for(&lineage(5));
    assert_eq!(f3 - f1, 2 * 100, "2 extra rungs must add 2×100");
    assert_eq!(f5 - f3, 2 * 100, "2 more extra rungs must add 2×100");
    // Monotone.
    assert!(
        f1 < f3 && f3 < f5,
        "lineage fee must be strictly increasing in ladder length"
    );
}

#[test]
fn sgb_and_ssm_fee_scale_with_fragment_length() {
    let short_sgb = fee_for(&sgb("SLL"));
    let long_sgb = fee_for(&sgb("SLL-with-modal-operators-and-recursive-types"));
    assert!(long_sgb > short_sgb, "longer SGB fragment must cost more");

    let short_ssm = fee_for(&ssm("game"));
    let long_ssm = fee_for(&ssm(
        "game-semantics-with-visibility-modality-and-composition",
    ));
    assert!(long_ssm > short_ssm, "longer SSM fragment must cost more");
}

#[test]
fn sfsv_fee_scales_with_future_self_length() {
    let short = fee_for(&sfsv("0x01"));
    let long = fee_for(&sfsv(
        "0x0102030405060708090a0b0c0d0e0f10111213141516171819",
    ));
    assert!(long > short, "longer future_self address must cost more");
}

#[test]
fn fixed_shape_primitives_fee_equals_base_fee() {
    // Fixed-shape: no variable contribution, so fee == base_fee.
    let typed = mayfly();
    assert_eq!(
        fee_for(&typed),
        base_fee(&typed),
        "Mayfly has no variable component — fee must equal base_fee"
    );

    let typed = heartbeat();
    assert_eq!(
        fee_for(&typed),
        base_fee(&typed),
        "SinghHeartbeat has no variable component"
    );

    let typed = mnemo();
    assert_eq!(
        fee_for(&typed),
        base_fee(&typed),
        "MnemoChain has no variable component"
    );
}

#[test]
fn saturating_arithmetic_on_huge_fragment() {
    // Defence-in-depth: even a gigantic fragment must not panic.
    let huge = sgb(&"x".repeat(4 * 1024)); // MAX_FRAGMENT_LEN
    let fee = fee_for(&huge);
    assert!(
        fee >= BASE_DEPLOY_FEE,
        "huge fragment must still return a sane fee"
    );
}

#[test]
fn camille_cost_comparison_full_arc() {
    // CAMILLE queries all six lanes before deciding what to deploy.
    // She expects: paradigm > marketplace > rich-NFT > wallet-ux > consumer ≈ cultural.
    let nft = fee_for(&sabi());
    let rich_nft = fee_for(&posthuma());
    let market = fee_for(&sddc());
    let wallet = fee_for(&heartbeat());
    let consumer = fee_for(&childkey());
    let cultural = fee_for(&gallery());
    let paradigm = fee_for(&sbav());

    assert!(paradigm > market, "paradigm > marketplace");
    assert!(market > rich_nft, "marketplace > rich-NFT");
    assert!(rich_nft > nft, "rich-NFT > basic-NFT");
    assert!(wallet > consumer, "wallet-ux > consumer");
    // Cultural is at consumer-class surcharge — not below base.
    assert!(cultural >= BASE_DEPLOY_FEE);
}
