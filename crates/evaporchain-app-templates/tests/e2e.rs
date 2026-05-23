//! §App-Templates catalogue e2e
//!
//! Scenario: "EvaporChain wallet UI enumeration" — ZARA is building a
//! wallet that renders a deploy form for every registered primitive.
//! She enumerates the full catalogue, groups by lane, spot-checks
//! several entries by class id, verifies that default_params are
//! non-empty, and confirms the catalogue is deterministic.
//!
//! The suite proves: every entry is in-range; find() is total over
//! the catalogue; default_params is a non-empty JSON object; the
//! catalogue is non-empty, sorted, deduplicated.

use evaporchain_app_templates::{
    catalogue, find, CatalogueError, TemplateClass,
    APP_TEMPLATE_RANGE_END, APP_TEMPLATE_RANGE_START,
};
use evaporchain_app_templates::class::{
    GALLERY_FORGETS, MAYFLY, MNEMOCHAIN_CARD, SGB_TYPE_SYSTEM,
    SDDC_AUCTION, SINGH_SABI, SINGH_POSTHUMA,
};
use std::collections::HashSet;

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn catalogue_is_non_empty() {
    assert!(!catalogue().is_empty(), "catalogue must contain at least one template");
}

#[test]
fn all_entries_in_app_template_range() {
    for t in catalogue() {
        assert!(
            (APP_TEMPLATE_RANGE_START..=APP_TEMPLATE_RANGE_END).contains(&t.class.0),
            "class {:#010x} ({}) is outside the reserved range",
            t.class.0, t.brand
        );
    }
}

#[test]
fn find_is_total_over_catalogue() {
    // Every catalogued template must be findable by its own class id.
    for t in catalogue() {
        find(t.class).unwrap_or_else(|_| panic!("find({:#010x}) must succeed", t.class.0));
    }
}

#[test]
fn find_unknown_class_returns_not_found() {
    let ghost = TemplateClass(0x0001_FFFE);
    let err = find(ghost).unwrap_err();
    assert_eq!(err, CatalogueError::NotFound(ghost.0));
}

#[test]
fn catalogue_is_deterministic() {
    // Two calls must produce byte-identical results.
    let a = catalogue();
    let b = catalogue();
    assert_eq!(a, b, "catalogue must be deterministic across calls");
}

#[test]
fn catalogue_has_no_duplicate_class_ids() {
    let ids: Vec<_> = catalogue().iter().map(|t| t.class.0).collect();
    let unique: HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len(), "catalogue must have no duplicate class ids");
}

#[test]
fn catalogue_is_sorted_by_class_id() {
    let ids: Vec<u32> = catalogue().iter().map(|t| t.class.0).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "catalogue must be sorted by class id");
}

#[test]
fn every_entry_has_non_empty_brand_and_lane() {
    for t in catalogue() {
        assert!(!t.brand.is_empty(), "class {:#010x} must have a non-empty brand", t.class.0);
        assert!(!t.lane.is_empty(),  "class {:#010x} must have a non-empty lane",  t.class.0);
    }
}

#[test]
fn every_entry_default_params_is_non_empty_object() {
    for t in catalogue() {
        assert!(t.default_params.is_object(),
            "{} default_params must be a JSON object", t.brand);
        assert!(!t.default_params.as_object().unwrap().is_empty(),
            "{} default_params must not be empty", t.brand);
    }
}

#[test]
fn six_lanes_all_represented() {
    // The catalogue must cover all six primitive lanes.
    let lanes: HashSet<String> = catalogue().iter().map(|t| t.lane.clone()).collect();
    for expected_lane in &["NFT", "Marketplace", "Wallet UX", "Consumer", "Cultural", "Paradigm"] {
        assert!(lanes.contains(*expected_lane),
            "lane '{}' must be represented in the catalogue", expected_lane);
    }
}

#[test]
fn spot_check_nft_lane_entries() {
    // SinghSabi, SinghPosthuma, and Mayfly must all be in the catalogue.
    for class in [SINGH_SABI, SINGH_POSTHUMA, MAYFLY] {
        let desc = find(class).unwrap();
        assert_eq!(desc.lane, "NFT",
            "{} must be in the NFT lane", desc.brand);
        assert!(!desc.crate_name.is_empty(),
            "{} must have a crate_name", desc.brand);
    }
}

#[test]
fn spot_check_marketplace_lane() {
    let desc = find(SDDC_AUCTION).unwrap();
    assert_eq!(desc.lane, "Marketplace");
    assert!(desc.default_params.get("ceiling").is_some(),
        "SDDC default_params must include 'ceiling'");
    assert!(desc.default_params.get("floor").is_some(),
        "SDDC default_params must include 'floor'");
}

#[test]
fn spot_check_consumer_lane() {
    let desc = find(MNEMOCHAIN_CARD).unwrap();
    assert_eq!(desc.lane, "Consumer");
    assert!(desc.default_params.get("initial_energy").is_some(),
        "MnemoChain default_params must include 'initial_energy'");
}

#[test]
fn spot_check_paradigm_lane() {
    let desc = find(SGB_TYPE_SYSTEM).unwrap();
    assert_eq!(desc.lane, "Paradigm");
}

#[test]
fn spot_check_cultural_lane() {
    let desc = find(GALLERY_FORGETS).unwrap();
    assert_eq!(desc.lane, "Cultural");
}

#[test]
fn zara_wallet_enumeration_full_arc() {
    // Full arc: ZARA enumerates all templates, groups by lane, and
    // verifies she can build a deploy form for each.
    let cat = catalogue();
    let mut by_lane: std::collections::BTreeMap<&str, Vec<&str>> = Default::default();
    for t in &cat {
        by_lane.entry(t.lane.as_str()).or_default().push(t.brand.as_str());
    }

    // Every lane has at least one template.
    for lane in ["NFT", "Marketplace", "Wallet UX", "Consumer", "Cultural", "Paradigm"] {
        assert!(by_lane.contains_key(lane),
            "lane '{}' must appear in Zara's wallet UI", lane);
    }

    // Total count matches catalogue.
    let total: usize = by_lane.values().map(|v| v.len()).sum();
    assert_eq!(total, cat.len(), "lane groups must cover all catalogue entries");

    // Every entry round-trips through serde (wallet persists to JSON).
    for t in &cat {
        let json = serde_json::to_string(t).unwrap();
        let back: evaporchain_app_templates::descriptor::TemplateDescriptor =
            serde_json::from_str(&json).unwrap();
        assert_eq!(back.class, t.class,
            "{} must survive JSON round-trip", t.brand);
    }
}
