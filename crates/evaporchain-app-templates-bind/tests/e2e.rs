//! §Bind — per-primitive invariant pre-flight gate e2e
//!
//! Scenario: "EvaporChain App Store Launch" — ARJUN is the deployer.
//! He submits a portfolio of primitives across all six lanes: NFT,
//! Marketplace, Wallet UX, Consumer, Cultural, and Paradigm.
//! Some configs are valid; some are adversarial (zero fields, out-of-
//! range values, ordering violations). The bind gate must reject every
//! bad config with a typed BindError before any chain state is touched,
//! and accept every valid config as a Bound that preserves the input.
//!
//! The suite proves: (1) every canonical invariant is enforced at bind
//! time; (2) Bound is a transparent passthrough of valid TypedInit;
//! (3) all six lanes participate; (4) BindContext pairs correctly.

use evaporchain_app_templates_bind::{bind, BindContext, BindError};
use evaporchain_app_templates_engine::{
    init_childkey, init_gallery_forgets, init_mayfly, init_mnemochain,
    init_sddc, init_sfsv, init_sgb, init_singh_heartbeat, init_singh_lineage,
    init_singh_posthuma, init_singh_resonance, init_singh_sabi, init_singh_triage,
    TypedInit,
};

// ── Deployer ──────────────────────────────────────────────────────────────
fn arjun() -> [u8; 32] { [0xA4; 32] }
fn instance(n: u8) -> [u8; 32] { [n; 32] }
fn ctx(n: u8) -> BindContext { BindContext::new(arjun(), instance(n), 1_000) }

// ── Valid-init helpers ────────────────────────────────────────────────────

fn valid_sabi() -> TypedInit {
    TypedInit::SinghSabi(init_singh_sabi::InitConfig {
        initial_energy: 10_000,
        floor_pct: 10,
        half_life: 365,
    })
}

fn valid_posthuma() -> TypedInit {
    TypedInit::SinghPosthuma(init_singh_posthuma::InitConfig {
        half_life: 365,
        initial_visible_energy: 5_000,
        m_threshold: 3,
        n_committee: 5,
    })
}

fn valid_mayfly() -> TypedInit {
    TypedInit::Mayfly(init_mayfly::InitConfig {
        initial_energy: 1_000,
        half_life: 30,
    })
}

fn valid_sddc() -> TypedInit {
    TypedInit::Sddc(init_sddc::InitConfig {
        ceiling: 1_000,
        floor: 100,
        lot_lambda: 50,
        duration_epochs: 500,
    })
}

fn valid_sfsv() -> TypedInit {
    TypedInit::Sfsv(init_sfsv::InitConfig {
        deposit_amount: 500,
        predicate_type: 0,
        release_param: 200,
        future_self: "0xfuture".into(),
    })
}

fn valid_triage() -> TypedInit {
    TypedInit::SinghTriage(init_singh_triage::InitConfig {
        horizon_today: 1,
        horizon_tomorrow: 2,
        horizon_week: 7,
    })
}

fn valid_heartbeat() -> TypedInit {
    TypedInit::SinghHeartbeat(init_singh_heartbeat::InitConfig {
        healthy_bpm: 60,
        alarmed_bpm: 120,
        amber_threshold_bp: 7500,
        red_threshold_bp: 5000,
    })
}

fn valid_lineage() -> TypedInit {
    TypedInit::SinghLineage(init_singh_lineage::InitConfig {
        ladder: vec![
            init_singh_lineage::LadderRung { days: 30,  share_bp: 1000 },
            init_singh_lineage::LadderRung { days: 90,  share_bp: 2500 },
            init_singh_lineage::LadderRung { days: 180, share_bp: 5000 },
        ],
    })
}

fn valid_childkey() -> TypedInit {
    TypedInit::Childkey(init_childkey::InitConfig {
        unlock_age_years: 18,
        epochs_per_year: 365,
        m_threshold: 2,
        n_committee: 3,
    })
}

fn valid_mnemochain() -> TypedInit {
    TypedInit::Mnemochain(init_mnemochain::InitConfig {
        initial_energy: 1_000,
        initial_stability: 10,
    })
}

fn valid_gallery() -> TypedInit {
    TypedInit::GalleryForgets(init_gallery_forgets::InitConfig { opened_at_epoch: 0 })
}

fn valid_sgb() -> TypedInit {
    TypedInit::Sgb(init_sgb::InitConfig {
        fragment: "ipfs://QmTest".into(),
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn nft_lane_all_three_primitives_bind() {
    // SinghSabi, SinghPosthuma, Mayfly — NFT lane valid configs all pass.
    bind(valid_sabi()).expect("SinghSabi must bind");
    bind(valid_posthuma()).expect("SinghPosthuma must bind");
    bind(valid_mayfly()).expect("Mayfly must bind");
}

#[test]
fn marketplace_lane_both_primitives_bind() {
    // SDDC and SFSV — Marketplace lane valid configs pass.
    bind(valid_sddc()).expect("SDDC must bind");
    bind(valid_sfsv()).expect("SFSV must bind");
}

#[test]
fn wallet_ux_lane_three_primitives_bind() {
    // SinghTriage, SinghHeartbeat, SinghLineage — Wallet UX lane valid.
    bind(valid_triage()).expect("SinghTriage must bind");
    bind(valid_heartbeat()).expect("SinghHeartbeat must bind");
    bind(valid_lineage()).expect("SinghLineage must bind");
}

#[test]
fn consumer_lane_both_primitives_bind() {
    // Childkey and Mnemochain — Consumer lane valid.
    bind(valid_childkey()).expect("Childkey must bind");
    bind(valid_mnemochain()).expect("Mnemochain must bind");
}

#[test]
fn cultural_and_paradigm_lanes_bind() {
    // GalleryForgets (epoch=0 valid), SGB with non-empty fragment.
    bind(valid_gallery()).expect("GalleryForgets must bind");
    bind(valid_sgb()).expect("SGB must bind");
}

#[test]
fn nft_lane_zero_energy_rejects_across_three_primitives() {
    // Zero initial_energy must be caught at bind for SinghSabi, SinghResonance, Mayfly.
    let bad_sabi = TypedInit::SinghSabi(init_singh_sabi::InitConfig {
        initial_energy: 0, floor_pct: 10, half_life: 365,
    });
    let err = bind(bad_sabi).unwrap_err();
    assert!(matches!(err, BindError::Invariant { primitive: "Singh-Sabi", .. }),
        "zero energy: {:?}", err);

    let bad_resonance = TypedInit::SinghResonance(init_singh_resonance::InitConfig {
        initial_energy: 0, base_half_life: 100, saturation: 50, max_scale_bp: 200,
    });
    assert!(bind(bad_resonance).is_err(), "SinghResonance zero energy must reject");

    let bad_mayfly = TypedInit::Mayfly(init_mayfly::InitConfig {
        initial_energy: 0, half_life: 30,
    });
    assert!(bind(bad_mayfly).is_err(), "Mayfly zero energy must reject");
}

#[test]
fn posthuma_quorum_invariant_m_le_n() {
    // m > n must reject; m == n (unanimous) must accept; m == 1 must accept.
    let too_strict = TypedInit::SinghPosthuma(init_singh_posthuma::InitConfig {
        half_life: 365, initial_visible_energy: 5_000, m_threshold: 6, n_committee: 5,
    });
    let err = bind(too_strict).unwrap_err();
    assert!(matches!(err, BindError::Invariant { primitive: "Singh-Posthuma", .. }),
        "m=6 > n=5 must reject: {:?}", err);

    // Unanimous (m == n): valid.
    let unanimous = TypedInit::SinghPosthuma(init_singh_posthuma::InitConfig {
        half_life: 365, initial_visible_energy: 5_000, m_threshold: 5, n_committee: 5,
    });
    bind(unanimous).expect("m=n (unanimous) must bind");

    // Sole guardian (m=1, n=1): valid.
    let sole = TypedInit::SinghPosthuma(init_singh_posthuma::InitConfig {
        half_life: 365, initial_visible_energy: 1_000, m_threshold: 1, n_committee: 1,
    });
    bind(sole).expect("m=1,n=1 (sole guardian) must bind");
}

#[test]
fn marketplace_sddc_ceiling_floor_boundary() {
    // ceiling == floor must reject; ceiling == floor + 1 must accept.
    let equal = TypedInit::Sddc(init_sddc::InitConfig {
        ceiling: 100, floor: 100, lot_lambda: 10, duration_epochs: 100,
    });
    assert!(bind(equal).is_err(), "ceiling==floor must reject");

    let one_above = TypedInit::Sddc(init_sddc::InitConfig {
        ceiling: 101, floor: 100, lot_lambda: 10, duration_epochs: 100,
    });
    bind(one_above).expect("ceiling=floor+1 must bind (strict boundary)");
}

#[test]
fn wallet_ux_triage_horizons_must_be_strictly_increasing() {
    // today == tomorrow must reject; today > week must reject.
    let flat = TypedInit::SinghTriage(init_singh_triage::InitConfig {
        horizon_today: 2, horizon_tomorrow: 2, horizon_week: 7,
    });
    assert!(bind(flat).is_err(), "today==tomorrow must reject");

    let reversed = TypedInit::SinghTriage(init_singh_triage::InitConfig {
        horizon_today: 7, horizon_tomorrow: 5, horizon_week: 3,
    });
    assert!(bind(reversed).is_err(), "reversed horizons must reject");
}

#[test]
fn wallet_ux_heartbeat_amber_must_exceed_red() {
    // amber_threshold_bp <= red_threshold_bp must reject.
    let inverted = TypedInit::SinghHeartbeat(init_singh_heartbeat::InitConfig {
        healthy_bpm: 60,
        alarmed_bpm: 120,
        amber_threshold_bp: 40,
        red_threshold_bp: 75,
    });
    assert!(bind(inverted).is_err(), "amber<=red threshold must reject");

    // amber == red: also invalid (not strictly greater).
    let equal_bp = TypedInit::SinghHeartbeat(init_singh_heartbeat::InitConfig {
        healthy_bpm: 60,
        alarmed_bpm: 120,
        amber_threshold_bp: 50,
        red_threshold_bp: 50,
    });
    assert!(bind(equal_bp).is_err(), "amber==red must reject");
}

#[test]
fn wallet_ux_lineage_ladder_ordering_and_bounds() {
    // Days non-monotone: reject.
    let unsorted = TypedInit::SinghLineage(init_singh_lineage::InitConfig {
        ladder: vec![
            init_singh_lineage::LadderRung { days: 90, share_bp: 2500 },
            init_singh_lineage::LadderRung { days: 30, share_bp: 5000 },
        ],
    });
    assert!(bind(unsorted).is_err(), "unsorted days must reject");

    // Share_bp decreasing: reject (graduated dormancy must be monotonic).
    let decreasing = TypedInit::SinghLineage(init_singh_lineage::InitConfig {
        ladder: vec![
            init_singh_lineage::LadderRung { days: 30,  share_bp: 8000 },
            init_singh_lineage::LadderRung { days: 90,  share_bp: 3000 },
        ],
    });
    assert!(bind(decreasing).is_err(), "decreasing share_bp must reject");

    // share_bp > 10000 (>100%): reject.
    let oob = TypedInit::SinghLineage(init_singh_lineage::InitConfig {
        ladder: vec![
            init_singh_lineage::LadderRung { days: 30, share_bp: 12000 },
        ],
    });
    assert!(bind(oob).is_err(), "share_bp > 10000 must reject");

    // Flat share_bp (equal rungs, not decreasing): accept.
    let flat_share = TypedInit::SinghLineage(init_singh_lineage::InitConfig {
        ladder: vec![
            init_singh_lineage::LadderRung { days: 30, share_bp: 5000 },
            init_singh_lineage::LadderRung { days: 90, share_bp: 5000 },
        ],
    });
    bind(flat_share).expect("non-decreasing flat share_bp must bind");
}

#[test]
fn consumer_childkey_quorum_boundary() {
    // m_threshold == 0 rejects; m == n (unanimous) accepts.
    let zero_m = TypedInit::Childkey(init_childkey::InitConfig {
        unlock_age_years: 18, epochs_per_year: 365, m_threshold: 0, n_committee: 3,
    });
    assert!(bind(zero_m).is_err(), "m=0 must reject");

    let m_gt_n = TypedInit::Childkey(init_childkey::InitConfig {
        unlock_age_years: 18, epochs_per_year: 365, m_threshold: 4, n_committee: 3,
    });
    assert!(bind(m_gt_n).is_err(), "m>n must reject");

    let unanimous = TypedInit::Childkey(init_childkey::InitConfig {
        unlock_age_years: 18, epochs_per_year: 365, m_threshold: 3, n_committee: 3,
    });
    bind(unanimous).expect("m=n unanimous must bind");
}

#[test]
fn bound_is_transparent_passthrough() {
    // Bound(typed).0 must equal the input — no mutation, no wrapping.
    let typed = valid_sabi();
    let bound = bind(typed.clone()).unwrap();
    assert_eq!(bound.0, typed,
        "Bound must preserve the exact TypedInit value");
}

#[test]
fn bind_is_pure_function() {
    // Two calls with identical input produce identical output — no hidden state.
    let typed = valid_sddc();
    let a = bind(typed.clone()).unwrap();
    let b = bind(typed).unwrap();
    assert_eq!(a, b, "bind must be deterministic and idempotent");
}

#[test]
fn bind_context_pairs_with_typed_init() {
    // BindContext carries deployer + instance_id + epoch.
    // with_init() returns (ctx, typed) unchanged.
    let ctx = ctx(7);
    let typed = valid_mnemochain();
    let (c, t) = ctx.clone().with_init(typed.clone());
    assert_eq!(c, ctx);
    assert_eq!(t, typed);
    assert_eq!(c.deployer, arjun());
    assert_eq!(c.current_epoch, 1_000);
}

#[test]
fn arjun_app_store_full_deploy_arc() {
    // Full arc: Arjun submits 4 valid + 2 adversarial deploys.
    // The bind gate accepts the four valid ones and stops the two
    // bad ones before any chain state is touched.

    // Valid deploys: SinghSabi, SDDC, Mnemochain, GalleryForgets
    let deploys: Vec<(&str, TypedInit)> = vec![
        ("SinghSabi",     valid_sabi()),
        ("SDDC",          valid_sddc()),
        ("Mnemochain",    valid_mnemochain()),
        ("GalleryForgets",valid_gallery()),
    ];
    for (name, typed) in deploys {
        bind(typed).unwrap_or_else(|e| panic!("{name} must bind: {:?}", e));
    }

    // Adversarial: Mayfly with zero half_life → must be stopped here.
    let bad_mayfly = TypedInit::Mayfly(init_mayfly::InitConfig {
        initial_energy: 1_000, half_life: 0,
    });
    let err = bind(bad_mayfly).unwrap_err();
    assert!(matches!(err, BindError::Invariant { primitive: "Mayfly", .. }),
        "zero half_life: {:?}", err);

    // Adversarial: SFSV with predicate_type=2 → must be stopped here.
    let bad_sfsv = TypedInit::Sfsv(init_sfsv::InitConfig {
        deposit_amount: 500,
        predicate_type: 2,  // only 0 and 1 are valid
        release_param: 200,
        future_self: "0xbob".into(),
    });
    let err2 = bind(bad_sfsv).unwrap_err();
    assert!(matches!(err2, BindError::Invariant { primitive: "SFSV", .. }),
        "invalid predicate_type: {:?}", err2);
}
