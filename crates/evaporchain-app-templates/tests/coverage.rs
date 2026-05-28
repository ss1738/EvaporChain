//! Coverage tests for app-templates registry.

use evaporchain_app_templates::catalogue::{catalogue, find, CatalogueError};
use evaporchain_app_templates::class::{
    TemplateClass, APP_TEMPLATE_RANGE_END, APP_TEMPLATE_RANGE_START, SDDC_AUCTION, SFSV_VAULT,
    SHLM_BOUNTY,
};
use evaporchain_app_templates::descriptor::{DescriptorError, TemplateDescriptor};

// =================================================================
// Class range
// =================================================================

#[test]
fn class_range_constants_pinned() {
    assert_eq!(APP_TEMPLATE_RANGE_START, 0x0001_0000);
    assert_eq!(APP_TEMPLATE_RANGE_END, 0x0001_FFFF);
}

#[test]
fn is_in_app_range_for_known_classes() {
    assert!(SDDC_AUCTION.is_in_app_range());
    assert!(SFSV_VAULT.is_in_app_range());
    assert!(SHLM_BOUNTY.is_in_app_range());
}

#[test]
fn is_in_app_range_rejects_below_range() {
    let below = TemplateClass(APP_TEMPLATE_RANGE_START - 1);
    assert!(!below.is_in_app_range());
}

#[test]
fn is_in_app_range_rejects_above_range() {
    let above = TemplateClass(APP_TEMPLATE_RANGE_END + 1);
    assert!(!above.is_in_app_range());
}

#[test]
fn is_in_app_range_inclusive_at_endpoints() {
    assert!(TemplateClass(APP_TEMPLATE_RANGE_START).is_in_app_range());
    assert!(TemplateClass(APP_TEMPLATE_RANGE_END).is_in_app_range());
}

#[test]
fn template_class_serde_round_trips() {
    let c = SFSV_VAULT;
    let json = serde_json::to_string(&c).unwrap();
    let back: TemplateClass = serde_json::from_str(&json).unwrap();
    assert_eq!(back, c);
}

// =================================================================
// TemplateDescriptor::new validation
// =================================================================

#[test]
fn descriptor_new_rejects_empty_brand() {
    let err = TemplateDescriptor::new(
        SDDC_AUCTION,
        "",
        "marketplace",
        serde_json::json!({}),
        "notes",
        "evaporchain-sddc",
    )
    .unwrap_err();
    assert_eq!(err, DescriptorError::EmptyBrand);
}

#[test]
fn descriptor_new_rejects_empty_lane() {
    let err = TemplateDescriptor::new(
        SDDC_AUCTION,
        "SDDC",
        "",
        serde_json::json!({}),
        "notes",
        "evaporchain-sddc",
    )
    .unwrap_err();
    assert_eq!(err, DescriptorError::EmptyLane);
}

#[test]
fn descriptor_new_rejects_empty_crate_name() {
    let err = TemplateDescriptor::new(
        SDDC_AUCTION,
        "SDDC",
        "marketplace",
        serde_json::json!({}),
        "notes",
        "",
    )
    .unwrap_err();
    assert_eq!(err, DescriptorError::EmptyCrate);
}

#[test]
fn descriptor_new_rejects_out_of_range_class() {
    let outside = TemplateClass(0xFFFF_FFFF);
    let err = TemplateDescriptor::new(
        outside,
        "X",
        "lane",
        serde_json::json!({}),
        "notes",
        "crate",
    )
    .unwrap_err();
    match err {
        DescriptorError::OutOfRange(c) => assert_eq!(c, 0xFFFF_FFFF),
        other => panic!("expected OutOfRange, got {other:?}"),
    }
}

#[test]
fn descriptor_new_accepts_valid_inputs() {
    let d = TemplateDescriptor::new(
        SDDC_AUCTION,
        "SDDC",
        "marketplace",
        serde_json::json!({}),
        "notes",
        "evaporchain-sddc",
    )
    .expect("valid descriptor");
    assert_eq!(d.class, SDDC_AUCTION);
    assert_eq!(d.brand, "SDDC");
}

// =================================================================
// catalogue + find
// =================================================================

#[test]
fn catalogue_is_non_empty() {
    let entries = catalogue();
    assert!(
        entries.len() >= 21,
        "catalogue must register ≥ 21 templates"
    );
}

#[test]
fn catalogue_classes_all_in_app_range() {
    for d in catalogue() {
        assert!(
            d.class.is_in_app_range(),
            "{:?} must be in app-template range",
            d.class
        );
    }
}

#[test]
fn catalogue_class_ids_are_unique() {
    let entries = catalogue();
    let mut ids: Vec<u32> = entries.iter().map(|d| d.class.0).collect();
    ids.sort();
    let before = ids.len();
    ids.dedup();
    assert_eq!(
        ids.len(),
        before,
        "duplicate template class ids in catalogue"
    );
}

#[test]
fn find_resolves_known_class() {
    let d = find(SDDC_AUCTION).expect("SDDC must be registered");
    assert_eq!(d.class, SDDC_AUCTION);
    assert!(!d.brand.is_empty());
    assert!(!d.lane.is_empty());
    assert!(!d.crate_name.is_empty());
}

#[test]
fn find_unknown_class_errors() {
    let unknown = TemplateClass(0x0001_FFFE); // in range, but not registered
    let err = find(unknown).unwrap_err();
    assert!(matches!(err, CatalogueError::NotFound(_)));
}

#[test]
fn catalogue_error_displays() {
    let s = CatalogueError::NotFound(0xDEAD_BEEF).to_string();
    assert!(s.contains("deadbeef") || s.contains("DEADBEEF") || s.contains("0xdeadbeef"));
}

// =================================================================
// Serde
// =================================================================

#[test]
fn descriptor_serde_round_trips() {
    let d = TemplateDescriptor::new(
        SFSV_VAULT,
        "SFSV",
        "marketplace",
        serde_json::json!({"k": 1}),
        "x",
        "evaporchain-sfsv",
    )
    .unwrap();
    let json = serde_json::to_string(&d).unwrap();
    let back: TemplateDescriptor = serde_json::from_str(&json).unwrap();
    assert_eq!(back, d);
}
