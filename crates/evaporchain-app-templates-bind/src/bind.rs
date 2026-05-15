//! `bind` — the per-primitive invariant-validation driver.
//!
//! For each `TypedInit` variant, runs the primitive's known
//! invariant checks. Returns [`Bound`] on success; the chain's
//! `ContractEngine` consumes the `Bound` value with confidence
//! that the per-primitive constructor will not reject it for
//! invariant reasons.
//!
//! Invariants are duplicated here from each primitive's own
//! constructor — a single source of truth would force this crate
//! to depend on all 20 primitive crates (slow + heavy). The
//! cross-check is in tests: each invariant test here mirrors the
//! invariant the primitive's constructor would enforce.

use thiserror::Error;

use evaporchain_app_templates_engine::TypedInit;

/// A `TypedInit` whose per-primitive invariants have been
/// pre-validated. The chain can pass this straight into the
/// per-primitive constructor with confidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bound(pub TypedInit);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BindError {
    #[error("{primitive}: {what}")]
    Invariant {
        primitive: &'static str,
        what: &'static str,
    },
}

fn invariant(primitive: &'static str, what: &'static str) -> BindError {
    BindError::Invariant { primitive, what }
}

/// Validate per-primitive invariants on a `TypedInit`. Returns
/// `Bound(typed)` on success.
pub fn bind(typed: TypedInit) -> Result<Bound, BindError> {
    match &typed {
        // ── NFT lane ────────────────────────────────────────────────
        TypedInit::SinghSabi(c) => {
            if c.initial_energy == 0 {
                return Err(invariant("Singh-Sabi", "initial_energy must be > 0"));
            }
            if c.floor_pct > 100 {
                return Err(invariant("Singh-Sabi", "floor_pct must be in [0, 100]"));
            }
            if c.half_life == 0 {
                return Err(invariant("Singh-Sabi", "half_life must be > 0"));
            }
        }
        TypedInit::SinghMigrant(c) => {
            if c.initial_energy == 0 {
                return Err(invariant("Singh-Migrant", "initial_energy must be > 0"));
            }
            if c.base_half_life == 0 {
                return Err(invariant("Singh-Migrant", "base_half_life must be > 0"));
            }
            if c.resting_threshold_epochs == 0 {
                return Err(invariant(
                    "Singh-Migrant",
                    "resting_threshold_epochs must be > 0",
                ));
            }
        }
        TypedInit::SinghResonance(c) => {
            if c.initial_energy == 0 {
                return Err(invariant("Singh-Resonance", "initial_energy must be > 0"));
            }
            if c.base_half_life == 0 {
                return Err(invariant("Singh-Resonance", "base_half_life must be > 0"));
            }
            if c.saturation == 0 {
                return Err(invariant("Singh-Resonance", "saturation must be > 0"));
            }
            if c.max_scale_bp == 0 {
                return Err(invariant(
                    "Singh-Resonance",
                    "max_scale_bp must be > 0 (anti-Black-Mirror ceiling)",
                ));
            }
        }
        TypedInit::SinghPosthuma(c) => {
            if c.half_life == 0 {
                return Err(invariant("Singh-Posthuma", "half_life must be > 0"));
            }
            if c.initial_visible_energy == 0 {
                return Err(invariant(
                    "Singh-Posthuma",
                    "initial_visible_energy must be > 0",
                ));
            }
            if c.m_threshold == 0 {
                return Err(invariant("Singh-Posthuma", "m_threshold must be ≥ 1"));
            }
            if c.n_committee == 0 {
                return Err(invariant("Singh-Posthuma", "n_committee must be ≥ 1"));
            }
            if c.m_threshold > c.n_committee {
                return Err(invariant(
                    "Singh-Posthuma",
                    "m_threshold must be ≤ n_committee",
                ));
            }
        }
        TypedInit::Mayfly(c) => {
            if c.initial_energy == 0 {
                return Err(invariant("Mayfly", "initial_energy must be > 0"));
            }
            if c.half_life == 0 {
                return Err(invariant("Mayfly", "half_life must be > 0"));
            }
        }
        // ── Marketplace lane ────────────────────────────────────────
        TypedInit::Sddc(c) => {
            if c.ceiling <= c.floor {
                return Err(invariant("SDDC", "ceiling must be > floor"));
            }
            if c.lot_lambda == 0 {
                return Err(invariant("SDDC", "lot_lambda must be > 0"));
            }
            if c.duration_epochs == 0 {
                return Err(invariant("SDDC", "duration_epochs must be > 0"));
            }
        }
        TypedInit::Sfsv(c) => {
            if c.deposit == 0 {
                return Err(invariant("SFSV", "deposit must be > 0"));
            }
            if c.predicate.is_empty() {
                return Err(invariant("SFSV", "predicate must be non-empty"));
            }
            if c.release_epoch == 0 {
                return Err(invariant("SFSV", "release_epoch must be > 0"));
            }
        }
        TypedInit::Shlm(c) => {
            if c.salary_cap == 0 {
                return Err(invariant("SHLM", "salary_cap must be > 0"));
            }
            if c.min_freshness == 0 {
                return Err(invariant("SHLM", "min_freshness must be > 0"));
            }
            if c.min_level == 0 {
                return Err(invariant("SHLM", "min_level must be > 0"));
            }
        }
        TypedInit::Scl(c) => {
            if c.verb.is_empty() {
                return Err(invariant("SCL", "verb must be non-empty"));
            }
            if c.object.is_empty() {
                return Err(invariant("SCL", "object must be non-empty"));
            }
            if c.duration_epochs == 0 {
                return Err(invariant("SCL", "duration_epochs must be > 0"));
            }
        }
        TypedInit::Sap(c) => {
            if c.initial_value == 0 {
                return Err(invariant("SAP", "initial_value must be > 0"));
            }
            if c.half_life == 0 {
                return Err(invariant("SAP", "half_life must be > 0"));
            }
            if c.max_aq_per_window == 0 {
                return Err(invariant("SAP", "max_aq_per_window must be > 0"));
            }
            if c.window_epochs == 0 {
                return Err(invariant("SAP", "window_epochs must be > 0"));
            }
        }
        // ── Wallet UX lane ──────────────────────────────────────────
        TypedInit::SinghTriage(c) => {
            if !(c.horizon_today < c.horizon_tomorrow && c.horizon_tomorrow < c.horizon_week) {
                return Err(invariant(
                    "Singh-Triage",
                    "horizons must be strictly increasing (today < tomorrow < week)",
                ));
            }
        }
        TypedInit::SinghHeartbeat(c) => {
            if c.healthy_bpm >= c.alarmed_bpm {
                return Err(invariant(
                    "Singh-Heartbeat",
                    "healthy_bpm must be < alarmed_bpm",
                ));
            }
            if c.amber_threshold_bp > 10000 || c.red_threshold_bp > 10000 {
                return Err(invariant(
                    "Singh-Heartbeat",
                    "thresholds must be in basis points (≤ 10000)",
                ));
            }
            if c.amber_threshold_bp <= c.red_threshold_bp {
                return Err(invariant(
                    "Singh-Heartbeat",
                    "amber_threshold_bp must be > red_threshold_bp (amber is the warning, red is critical)",
                ));
            }
        }
        TypedInit::SinghLineage(c) => {
            if c.ladder.is_empty() {
                return Err(invariant("Singh-Lineage", "ladder must be non-empty"));
            }
            // days strictly increasing
            for w in c.ladder.windows(2) {
                if w[0].days >= w[1].days {
                    return Err(invariant(
                        "Singh-Lineage",
                        "ladder rungs must be strictly sorted by days",
                    ));
                }
            }
            // share_bp non-decreasing (graduated dormancy is monotonic)
            for w in c.ladder.windows(2) {
                if w[0].share_bp > w[1].share_bp {
                    return Err(invariant(
                        "Singh-Lineage",
                        "ladder share_bp must be non-decreasing",
                    ));
                }
            }
            // each share_bp in basis points
            for r in &c.ladder {
                if r.share_bp > 10000 {
                    return Err(invariant(
                        "Singh-Lineage",
                        "share_bp must be in basis points (≤ 10000)",
                    ));
                }
            }
        }
        // ── Consumer lane ───────────────────────────────────────────
        TypedInit::Childkey(c) => {
            if c.unlock_age_years == 0 {
                return Err(invariant("ChildKey", "unlock_age_years must be > 0"));
            }
            if c.epochs_per_year == 0 {
                return Err(invariant("ChildKey", "epochs_per_year must be > 0"));
            }
            if c.m_threshold == 0 {
                return Err(invariant("ChildKey", "m_threshold must be ≥ 1"));
            }
            if c.n_committee == 0 {
                return Err(invariant("ChildKey", "n_committee must be ≥ 1"));
            }
            if c.m_threshold > c.n_committee {
                return Err(invariant("ChildKey", "m_threshold must be ≤ n_committee"));
            }
        }
        TypedInit::Mnemochain(c) => {
            if c.initial_energy == 0 {
                return Err(invariant("MnemoChain", "initial_energy must be > 0"));
            }
            if c.initial_stability == 0 {
                return Err(invariant("MnemoChain", "initial_stability must be > 0"));
            }
        }
        TypedInit::Witnessfit(c) => {
            if c.half_life == 0 {
                return Err(invariant("WitnessFit", "half_life must be > 0"));
            }
            if c.boost_bp > 10000 {
                return Err(invariant(
                    "WitnessFit",
                    "boost_bp must be in basis points (≤ 10000)",
                ));
            }
        }
        // ── Cultural lane ───────────────────────────────────────────
        TypedInit::GalleryForgets(_) => {
            // opened_at_epoch has no positivity constraint — 0 is a
            // legitimate "opens at genesis" sentinel.
        }
        // ── Paradigm lane ───────────────────────────────────────────
        TypedInit::Sgb(c) => {
            if c.fragment.is_empty() {
                return Err(invariant("SGB", "fragment must be non-empty"));
            }
        }
        TypedInit::Sbav(c) => {
            if c.reg_count == 0 {
                return Err(invariant("SBAV", "reg_count must be > 0"));
            }
        }
        TypedInit::Ssm(c) => {
            if c.fragment.is_empty() {
                return Err(invariant("SSM", "fragment must be non-empty"));
            }
        }
        // RefreshMarket landed in TypedInit but the bind invariant
        // table was never extended — unblocks
        // `cargo build --workspace`. The RefreshMarket init config
        // is owned by `evaporchain-refresh-market`; its on-construct
        // validation already runs in `init_refresh_market`, so the
        // bind step here is a no-op acknowledging the variant. Add
        // primitive-specific checks once the market substrate
        // surfaces config invariants that warrant pre-flight
        // rejection (e.g. zero-half-life market shouldn't deploy).
        TypedInit::RefreshMarket(_) => {}
    }

    Ok(Bound(typed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_app_templates::{catalogue, class::*};
    use evaporchain_app_templates_deploy::DeployRequest;
    use evaporchain_app_templates_engine::*;
    use evaporchain_app_templates_materialise::materialise_request;

    /// Build a `TypedInit` by running default_params through the
    /// full deploy → materialise → engine pipeline.
    fn typed_for(class: evaporchain_app_templates::TemplateClass) -> TypedInit {
        let desc = evaporchain_app_templates::find(class).unwrap();
        let req = DeployRequest::new(class, desc.default_params, [0xAB; 32], 0, 1).unwrap();
        let instr = materialise_request(&req).unwrap();
        materialise(&instr).unwrap()
    }

    #[test]
    fn every_catalogue_default_binds() {
        // Anti-regression: every catalogue entry's `default_params`
        // should pass invariant validation. If a future PR loosens
        // a default to an invariant-violating value, this catches
        // it.
        for desc in catalogue() {
            let typed = typed_for(desc.class);
            bind(typed).unwrap_or_else(|e| {
                panic!(
                    "default_params for {:#010x} ({}) failed bind: {:?}",
                    desc.class.0, desc.brand, e
                );
            });
        }
    }

    #[test]
    fn singh_sabi_zero_initial_rejected() {
        let typed = TypedInit::SinghSabi(init_singh_sabi::InitConfig {
            initial_energy: 0,
            floor_pct: 15,
            half_life: 365,
        });
        let err = bind(typed).unwrap_err();
        assert!(matches!(
            err,
            BindError::Invariant {
                primitive: "Singh-Sabi",
                ..
            }
        ));
    }

    #[test]
    fn singh_sabi_floor_pct_over_100_rejected() {
        let typed = TypedInit::SinghSabi(init_singh_sabi::InitConfig {
            initial_energy: 1000,
            floor_pct: 150,
            half_life: 365,
        });
        assert!(bind(typed).is_err());
    }

    #[test]
    fn singh_posthuma_m_gt_n_rejected() {
        let typed = TypedInit::SinghPosthuma(init_singh_posthuma::InitConfig {
            half_life: 365,
            initial_visible_energy: 1000,
            m_threshold: 4,
            n_committee: 3,
        });
        let err = bind(typed).unwrap_err();
        assert!(matches!(
            err,
            BindError::Invariant {
                primitive: "Singh-Posthuma",
                ..
            }
        ));
    }

    #[test]
    fn sddc_ceiling_le_floor_rejected() {
        let typed = TypedInit::Sddc(init_sddc::InitConfig {
            ceiling: 100,
            floor: 100,
            lot_lambda: 50,
            duration_epochs: 1000,
        });
        assert!(bind(typed).is_err());
    }

    #[test]
    fn singh_triage_non_increasing_horizons_rejected() {
        let typed = TypedInit::SinghTriage(init_singh_triage::InitConfig {
            horizon_today: 5,
            horizon_tomorrow: 5,
            horizon_week: 7,
        });
        assert!(bind(typed).is_err());
    }

    #[test]
    fn singh_heartbeat_healthy_ge_alarmed_rejected() {
        let typed = TypedInit::SinghHeartbeat(init_singh_heartbeat::InitConfig {
            healthy_bpm: 120,
            alarmed_bpm: 60,
            amber_threshold_bp: 75,
            red_threshold_bp: 40,
        });
        assert!(bind(typed).is_err());
    }

    #[test]
    fn singh_lineage_unsorted_ladder_rejected() {
        let typed = TypedInit::SinghLineage(init_singh_lineage::InitConfig {
            ladder: vec![
                init_singh_lineage::LadderRung {
                    days: 90,
                    share_bp: 2500,
                },
                init_singh_lineage::LadderRung {
                    days: 30,
                    share_bp: 5000,
                },
            ],
        });
        assert!(bind(typed).is_err());
    }

    #[test]
    fn singh_lineage_decreasing_share_rejected() {
        let typed = TypedInit::SinghLineage(init_singh_lineage::InitConfig {
            ladder: vec![
                init_singh_lineage::LadderRung {
                    days: 90,
                    share_bp: 5000,
                },
                init_singh_lineage::LadderRung {
                    days: 180,
                    share_bp: 2500,
                },
            ],
        });
        assert!(bind(typed).is_err());
    }

    #[test]
    fn singh_lineage_share_over_10000_rejected() {
        let typed = TypedInit::SinghLineage(init_singh_lineage::InitConfig {
            ladder: vec![init_singh_lineage::LadderRung {
                days: 90,
                share_bp: 12000,
            }],
        });
        assert!(bind(typed).is_err());
    }

    #[test]
    fn childkey_zero_threshold_rejected() {
        let typed = TypedInit::Childkey(init_childkey::InitConfig {
            unlock_age_years: 18,
            epochs_per_year: 365,
            m_threshold: 0,
            n_committee: 5,
        });
        assert!(bind(typed).is_err());
    }

    #[test]
    fn scl_empty_verb_rejected() {
        let typed = TypedInit::Scl(init_scl::InitConfig {
            verb: String::new(),
            object: "0xobj".into(),
            duration_epochs: 1000,
        });
        assert!(bind(typed).is_err());
    }

    #[test]
    fn sgb_empty_fragment_rejected() {
        let typed = TypedInit::Sgb(init_sgb::InitConfig {
            fragment: String::new(),
        });
        assert!(bind(typed).is_err());
    }

    #[test]
    fn gallery_forgets_zero_epoch_accepted() {
        // Cultural lane: opened_at_epoch=0 is a legitimate "opens at
        // genesis" sentinel, not an error.
        let typed =
            TypedInit::GalleryForgets(init_gallery_forgets::InitConfig { opened_at_epoch: 0 });
        bind(typed).unwrap();
    }

    #[test]
    fn witnessfit_boost_over_10000_rejected() {
        let typed = TypedInit::Witnessfit(init_witnessfit::InitConfig {
            half_life: 7,
            boost_bp: 12000,
        });
        assert!(bind(typed).is_err());
    }

    #[test]
    fn bind_is_idempotent() {
        // Pure function — running bind twice on the same input
        // produces the same output.
        let typed = typed_for(SINGH_SABI);
        let a = bind(typed.clone()).unwrap();
        let b = bind(typed).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn bound_preserves_typed_init() {
        // Bound(typed) round-trips: the inner value is the input
        // unchanged. This is what lets the ContractEngine pattern-
        // match on Bound's inner TypedInit directly.
        let typed = typed_for(MAYFLY);
        let bound = bind(typed.clone()).unwrap();
        assert_eq!(bound.0, typed);
    }
}
