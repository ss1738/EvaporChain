//! Coverage tests for the per-primitive bind / invariant layer.

use evaporchain_app_templates_bind::bind::{bind, BindError, Bound};
use evaporchain_app_templates_engine::dispatch::TypedInit;
use evaporchain_app_templates_engine::{init_sddc, init_sfsv, init_shlm};

// =================================================================
// SDDC invariants
// =================================================================

fn sddc(ceiling: u64, floor: u64, lambda: u64, dur: u64) -> TypedInit {
    TypedInit::Sddc(init_sddc::InitConfig {
        ceiling,
        floor,
        lot_lambda: lambda,
        duration_epochs: dur,
    })
}

#[test]
fn sddc_happy_path_binds() {
    let b = bind(sddc(1_000, 100, 10, 500)).expect("valid SDDC");
    assert!(matches!(b.0, TypedInit::Sddc(_)));
}

#[test]
fn sddc_rejects_ceiling_equal_floor() {
    let err = bind(sddc(100, 100, 10, 500)).unwrap_err();
    match err {
        BindError::Invariant { primitive, what } => {
            assert_eq!(primitive, "SDDC");
            assert!(what.contains("ceiling"));
        }
    }
}

#[test]
fn sddc_rejects_ceiling_below_floor() {
    let err = bind(sddc(50, 100, 10, 500)).unwrap_err();
    assert!(matches!(err, BindError::Invariant { primitive: "SDDC", .. }));
}

#[test]
fn sddc_rejects_zero_lambda() {
    let err = bind(sddc(1_000, 100, 0, 500)).unwrap_err();
    match err {
        BindError::Invariant { primitive, what } => {
            assert_eq!(primitive, "SDDC");
            assert!(what.contains("lot_lambda"));
        }
    }
}

#[test]
fn sddc_rejects_zero_duration() {
    let err = bind(sddc(1_000, 100, 10, 0)).unwrap_err();
    match err {
        BindError::Invariant { what, .. } => assert!(what.contains("duration_epochs")),
    }
}

// =================================================================
// SFSV invariants
// =================================================================

fn sfsv(deposit: u64, predicate: u64, release: u64, future_self: impl Into<String>) -> TypedInit {
    let future_self = future_self.into();
    TypedInit::Sfsv(init_sfsv::InitConfig {
        future_self,
        predicate_type: predicate,
        release_param: release,
        deposit_amount: deposit,
    })
}

#[test]
fn sfsv_happy_path_predicate_zero() {
    let b = bind(sfsv(1_000, 0, 100, "0x01".repeat(32))).expect("valid SFSV");
    assert!(matches!(b.0, TypedInit::Sfsv(_)));
}

#[test]
fn sfsv_happy_path_predicate_one() {
    let b = bind(sfsv(1_000, 1, 500, "0x02".repeat(32))).expect("valid SFSV");
    assert!(matches!(b.0, TypedInit::Sfsv(_)));
}

#[test]
fn sfsv_rejects_zero_deposit() {
    let err = bind(sfsv(0, 0, 100, "0x01".repeat(32))).unwrap_err();
    match err {
        BindError::Invariant { primitive, what } => {
            assert_eq!(primitive, "SFSV");
            assert!(what.contains("deposit_amount"));
        }
    }
}

#[test]
fn sfsv_rejects_predicate_type_two() {
    let err = bind(sfsv(1_000, 2, 100, "0x01".repeat(32))).unwrap_err();
    match err {
        BindError::Invariant { what, .. } => assert!(what.contains("predicate_type")),
    }
}

#[test]
fn sfsv_rejects_zero_release_param() {
    let err = bind(sfsv(1_000, 0, 0, "0x01".repeat(32))).unwrap_err();
    match err {
        BindError::Invariant { what, .. } => assert!(what.contains("release_param")),
    }
}

#[test]
fn sfsv_rejects_empty_future_self() {
    let err = bind(sfsv(1_000, 0, 100, "")).unwrap_err();
    match err {
        BindError::Invariant { what, .. } => assert!(what.contains("future_self")),
    }
}

// =================================================================
// SHLM invariants
// =================================================================

#[test]
fn shlm_rejects_zero_salary_cap() {
    let init = init_shlm::InitConfig {
        salary_cap: 0,
        min_freshness: 100,
        min_level: 1,
    };
    let err = bind(TypedInit::Shlm(init)).unwrap_err();
    match err {
        BindError::Invariant { primitive, what } => {
            assert_eq!(primitive, "SHLM");
            assert!(what.contains("salary_cap"));
        }
    }
}

#[test]
fn shlm_rejects_zero_min_freshness() {
    let init = init_shlm::InitConfig {
        salary_cap: 1_000,
        min_freshness: 0,
        min_level: 1,
    };
    let err = bind(TypedInit::Shlm(init)).unwrap_err();
    match err {
        BindError::Invariant { primitive, what } => {
            assert_eq!(primitive, "SHLM");
            assert!(what.contains("min_freshness"));
        }
    }
}

// =================================================================
// BindError ergonomics
// =================================================================

#[test]
fn bind_error_displays_primitive_and_what() {
    let e = BindError::Invariant { primitive: "SDDC", what: "ceiling must be > floor" };
    let s = e.to_string();
    assert!(s.contains("SDDC"));
    assert!(s.contains("ceiling"));
}

#[test]
fn bind_error_eq_discriminates() {
    let a = BindError::Invariant { primitive: "SDDC", what: "x" };
    let b = BindError::Invariant { primitive: "SDDC", what: "x" };
    let c = BindError::Invariant { primitive: "SFSV", what: "x" };
    let d = BindError::Invariant { primitive: "SDDC", what: "y" };
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_ne!(a, d);
}

// =================================================================
// Bound newtype
// =================================================================

#[test]
fn bound_carries_the_typed_init() {
    let inner = init_sddc::InitConfig {
        ceiling: 1_000,
        floor: 100,
        lot_lambda: 10,
        duration_epochs: 500,
    };
    let b = bind(TypedInit::Sddc(inner.clone())).expect("valid");
    let Bound(typed) = b;
    if let TypedInit::Sddc(c) = typed {
        assert_eq!(c, inner);
    } else {
        panic!("expected Sddc variant");
    }
}
