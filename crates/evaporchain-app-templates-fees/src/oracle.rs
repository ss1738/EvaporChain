//! `fee_for(typed)` — the deploy-fee oracle.
//!
//! Each primitive has a base fee plus an optional per-unit-complexity
//! cost. The base fee covers the on-chain instance creation (the
//! handle, the catalogue lookup, the authority record); the
//! per-unit cost covers state proportional to deploy params (ladder
//! rungs, fragment bytes, lease string bytes).
//!
//! Numbers are doctrine choices, not derived from physics. The
//! constants below are the V1 schedule.

use evaporchain_app_templates_engine::TypedInit;

/// The base fee every deploy pays. Covers instance-handle creation,
/// catalogue lookup, authority record, and the chain's per-deploy
/// bookkeeping.
pub const BASE_DEPLOY_FEE: u64 = 1_000;

// ── Per-primitive surcharges (added on top of BASE_DEPLOY_FEE).
//
// Roughly in order of how much state the primitive creates at deploy.
// NFT deploys (single token state) sit near the base; marketplace
// auctions (multi-axis state machines) sit higher; paradigm primitives
// (typing-VM bootstraps) sit highest.

const SURCHARGE_NFT_BASE: u64 = 200;
const SURCHARGE_NFT_RICH: u64 = 400; // Resonance, Posthuma — extra moving parts
const SURCHARGE_MARKETPLACE: u64 = 600;
const SURCHARGE_WALLET_UX: u64 = 300;
const SURCHARGE_CONSUMER: u64 = 250;
const SURCHARGE_CULTURAL: u64 = 250;
const SURCHARGE_PARADIGM: u64 = 800;

// ── Per-unit costs for variable-shape params.

const PER_LADDER_RUNG: u64 = 100;
const PER_FRAGMENT_BYTE: u64 = 4;
const PER_SCL_TARGET_BYTE: u64 = 4; // verb+object combined

/// The base fee for a class without any variable-shape contribution.
/// Useful for UI cost preview ("approximately X"), test fixtures, and
/// computing diffs between two ladder lengths.
pub fn base_fee(typed: &TypedInit) -> u64 {
    let surcharge = match typed {
        TypedInit::SinghSabi(_) | TypedInit::SinghMigrant(_) | TypedInit::Mayfly(_) => {
            SURCHARGE_NFT_BASE
        }
        TypedInit::SinghResonance(_) | TypedInit::SinghPosthuma(_) => SURCHARGE_NFT_RICH,
        TypedInit::Sddc(_)
        | TypedInit::Sfsv(_)
        | TypedInit::Shlm(_)
        | TypedInit::Scl(_)
        | TypedInit::Sap(_) => SURCHARGE_MARKETPLACE,
        TypedInit::SinghTriage(_) | TypedInit::SinghHeartbeat(_) | TypedInit::SinghLineage(_) => {
            SURCHARGE_WALLET_UX
        }
        TypedInit::Childkey(_) | TypedInit::Mnemochain(_) | TypedInit::Witnessfit(_) => {
            SURCHARGE_CONSUMER
        }
        TypedInit::GalleryForgets(_) => SURCHARGE_CULTURAL,
        TypedInit::Sgb(_) | TypedInit::Sbav(_) | TypedInit::Ssm(_) => SURCHARGE_PARADIGM,
    };
    BASE_DEPLOY_FEE.saturating_add(surcharge)
}

/// Compute the total deploy fee for a typed init. Combines the base
/// fee with any per-unit complexity contribution from variable-shape
/// params (ladder length, fragment bytes, lease string bytes).
pub fn fee_for(typed: &TypedInit) -> u64 {
    let base = base_fee(typed);
    let variable = match typed {
        // ── Variable-shape contributors ─────────────────────────
        TypedInit::SinghLineage(c) => {
            // Ladder cost: per-rung surcharge.
            (c.ladder.len() as u64).saturating_mul(PER_LADDER_RUNG)
        }
        TypedInit::Sgb(c) => (c.fragment.len() as u64).saturating_mul(PER_FRAGMENT_BYTE),
        TypedInit::Ssm(c) => (c.fragment.len() as u64).saturating_mul(PER_FRAGMENT_BYTE),
        TypedInit::Scl(c) => {
            let target_bytes = (c.verb.len() as u64).saturating_add(c.object.len() as u64);
            target_bytes.saturating_mul(PER_SCL_TARGET_BYTE)
        }
        TypedInit::Sfsv(c) => {
            // Predicate string length contributes too; SFSV's
            // predicate is the on-chain release condition.
            (c.predicate.len() as u64).saturating_mul(PER_FRAGMENT_BYTE)
        }
        // ── Fixed-shape primitives ──────────────────────────────
        _ => 0,
    };
    base.saturating_add(variable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_app_templates::{catalogue, class::*};
    use evaporchain_app_templates_deploy::DeployRequest;
    use evaporchain_app_templates_engine::*;
    use evaporchain_app_templates_materialise::materialise_request;

    fn typed_for(class: evaporchain_app_templates::TemplateClass) -> TypedInit {
        let desc = evaporchain_app_templates::find(class).unwrap();
        let req = DeployRequest::new(class, desc.default_params, [0xAB; 32], 0, 1).unwrap();
        let instr = materialise_request(&req).unwrap();
        materialise(&instr).unwrap()
    }

    #[test]
    fn every_catalogue_entry_has_a_quotable_fee() {
        // Anti-regression: every registered template's default
        // params must produce a quote without panicking.
        for desc in catalogue() {
            let typed = typed_for(desc.class);
            let fee = fee_for(&typed);
            assert!(
                fee >= BASE_DEPLOY_FEE,
                "fee for {:#010x} ({}) is {} — below base",
                desc.class.0,
                desc.brand,
                fee
            );
        }
    }

    #[test]
    fn fee_is_deterministic_across_calls() {
        // Validator determinism: same input → same fee, byte-identical.
        let typed = typed_for(SINGH_SABI);
        assert_eq!(fee_for(&typed), fee_for(&typed));
    }

    #[test]
    fn lineage_fee_scales_with_ladder_length() {
        let one_rung = TypedInit::SinghLineage(init_singh_lineage::InitConfig {
            ladder: vec![init_singh_lineage::LadderRung {
                days: 90,
                share_bp: 2500,
            }],
        });
        let three_rungs = TypedInit::SinghLineage(init_singh_lineage::InitConfig {
            ladder: vec![
                init_singh_lineage::LadderRung {
                    days: 90,
                    share_bp: 2500,
                },
                init_singh_lineage::LadderRung {
                    days: 180,
                    share_bp: 5000,
                },
                init_singh_lineage::LadderRung {
                    days: 365,
                    share_bp: 10000,
                },
            ],
        });
        assert!(fee_for(&three_rungs) > fee_for(&one_rung));
        // Difference must be exactly 2 * PER_LADDER_RUNG.
        assert_eq!(
            fee_for(&three_rungs) - fee_for(&one_rung),
            2 * PER_LADDER_RUNG
        );
    }

    #[test]
    fn sgb_fee_scales_with_fragment_length() {
        let short = TypedInit::Sgb(init_sgb::InitConfig {
            fragment: "SLL".into(),
        });
        let long = TypedInit::Sgb(init_sgb::InitConfig {
            fragment: "SLL_extended_with_modal_operators".into(),
        });
        assert!(fee_for(&long) > fee_for(&short));
    }

    #[test]
    fn scl_fee_scales_with_verb_object_length() {
        let short = TypedInit::Scl(init_scl::InitConfig {
            verb: "r".into(),
            object: "a".into(),
            duration_epochs: 1000,
        });
        let long = TypedInit::Scl(init_scl::InitConfig {
            verb: "read_with_audit".into(),
            object: "the_internal_company_finance_ledger".into(),
            duration_epochs: 1000,
        });
        assert!(fee_for(&long) > fee_for(&short));
    }

    #[test]
    fn ssm_fee_scales_with_fragment_length() {
        let short = TypedInit::Ssm(init_ssm::InitConfig {
            fragment: "x".into(),
        });
        let long = TypedInit::Ssm(init_ssm::InitConfig {
            fragment: "x".repeat(100),
        });
        let delta = fee_for(&long) - fee_for(&short);
        assert!(fee_for(&long) > fee_for(&short));
        assert_eq!(delta, (100u64 - 1) * PER_FRAGMENT_BYTE);
    }

    #[test]
    fn sfsv_fee_scales_with_predicate_length() {
        let short = TypedInit::Sfsv(init_sfsv::InitConfig {
            deposit: 1,
            predicate: "p".into(),
            release_epoch: 1,
        });
        let long = TypedInit::Sfsv(init_sfsv::InitConfig {
            deposit: 1,
            predicate: "p".repeat(50),
            release_epoch: 1,
        });
        let delta = fee_for(&long) - fee_for(&short);
        assert!(fee_for(&long) > fee_for(&short));
        assert_eq!(delta, (50u64 - 1) * PER_FRAGMENT_BYTE);
    }

    #[test]
    fn lineage_with_empty_ladder_costs_only_base() {
        let empty = TypedInit::SinghLineage(init_singh_lineage::InitConfig { ladder: vec![] });
        assert_eq!(fee_for(&empty), base_fee(&empty));
    }

    #[test]
    fn fixed_shape_primitives_cost_only_their_base() {
        let mayfly = TypedInit::Mayfly(init_mayfly::InitConfig {
            initial_energy: 1000,
            half_life: 100,
        });
        assert_eq!(fee_for(&mayfly), base_fee(&mayfly));
    }

    #[test]
    fn paradigm_lane_costs_more_than_nft_lane() {
        // Doctrine: paradigm primitives (SGB / SBAV / SSM) bootstrap
        // a typing VM and should be the most expensive deploy.
        let sabi = typed_for(SINGH_SABI);
        let sgb = TypedInit::Sgb(init_sgb::InitConfig {
            fragment: "SLL".into(),
        });
        assert!(fee_for(&sgb) > fee_for(&sabi));
    }

    #[test]
    fn marketplace_costs_more_than_consumer() {
        // SDDC (foundational marketplace) > MnemoChain (consumer card).
        let sddc = typed_for(SDDC_AUCTION);
        let mnemo = typed_for(MNEMOCHAIN_CARD);
        assert!(fee_for(&sddc) > fee_for(&mnemo));
    }

    #[test]
    fn rich_nft_costs_more_than_basic_nft() {
        // Resonance and Posthuma have extra moving parts vs Mayfly.
        let mayfly = typed_for(MAYFLY);
        let resonance = typed_for(SINGH_RESONANCE);
        let posthuma = typed_for(SINGH_POSTHUMA);
        assert!(fee_for(&resonance) > fee_for(&mayfly));
        assert!(fee_for(&posthuma) > fee_for(&mayfly));
    }

    #[test]
    fn fees_never_overflow_on_pathological_input() {
        // Defense-in-depth: even if invariant validation somehow
        // failed and a giant fragment got through, fee_for must
        // not panic — saturating_mul guards.
        let huge = TypedInit::Sgb(init_sgb::InitConfig {
            fragment: "x".repeat(1_000_000),
        });
        let _ = fee_for(&huge);
    }

    #[test]
    fn empty_variable_input_costs_only_base() {
        // SCL with single-char fields should approach the base
        // cost; the per-byte contribution is tiny but non-zero.
        let cfg = TypedInit::Scl(init_scl::InitConfig {
            verb: "r".into(),
            object: "x".into(),
            duration_epochs: 1,
        });
        let total = fee_for(&cfg);
        let base = base_fee(&cfg);
        assert!(total >= base);
        assert!(total - base <= 16); // 2 bytes × 4 byte-cost = 8, with headroom
    }

    proptest::proptest! {
        #[test]
        fn lineage_fee_monotone_in_ladder_length(extra in 0u64..50u64) {
            // Adding rungs never decreases the fee.
            let base_ladder = vec![init_singh_lineage::LadderRung {
                days: 1, share_bp: 100,
            }];
            let mut bigger = base_ladder.clone();
            for i in 0..extra {
                bigger.push(init_singh_lineage::LadderRung {
                    days: 2 + i,
                    share_bp: 200 + i,
                });
            }
            let small = TypedInit::SinghLineage(init_singh_lineage::InitConfig { ladder: base_ladder });
            let big = TypedInit::SinghLineage(init_singh_lineage::InitConfig { ladder: bigger });
            proptest::prop_assert!(fee_for(&big) >= fee_for(&small));
        }

        #[test]
        fn sgb_fee_monotone_in_fragment_length(extra in 0usize..1000usize) {
            let small = TypedInit::Sgb(init_sgb::InitConfig { fragment: "x".into() });
            let big = TypedInit::Sgb(init_sgb::InitConfig {
                fragment: "x".repeat(1 + extra),
            });
            proptest::prop_assert!(fee_for(&big) >= fee_for(&small));
        }
    }
}
