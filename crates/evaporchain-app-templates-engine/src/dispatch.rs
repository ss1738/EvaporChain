//! `materialise` driver — consumes a [`MaterialiseInstruction`],
//! dispatches to the per-template handler, and returns the typed
//! [`TypedInit`] enum the chain's `ContractEngine` pattern-matches on.
//!
//! Three structural decisions:
//!
//! 1. **One enum, all 20 variants.** A `Box<dyn Trait>` would let
//!    each variant have its own type, but every consumer would still
//!    need to downcast. An enum makes the dispatch exhaustive at
//!    compile time — adding a 21st template forces every consumer's
//!    `match` to be updated.
//!
//! 2. **Type-only validation, no business rules.** This layer
//!    confirms `initial_energy` is a u64, not that it's > 0. Each
//!    typed contract enforces its own invariants. Two layers of
//!    validation (this + per-contract) keeps the engine honest:
//!    type errors surface here, semantic errors surface in the
//!    contract.
//!
//! 3. **Unknown class is a hard error, not a fallback.** If the
//!    materialise layer has produced an envelope for an unknown
//!    class, the catalogue and engine drifted. Fail loudly rather
//!    than swallow.

use thiserror::Error;

use evaporchain_app_templates::class::*;
use evaporchain_app_templates_materialise::MaterialiseInstruction;

use crate::{
    init_bell_oracle, init_childkey, init_deadman, init_decay_access_pass, init_gallery_forgets,
    init_mayfly, init_mnemochain, init_mortal_dao, init_mortal_message, init_mortal_nft,
    init_multisig, init_open_bounty, init_refresh_market, init_sap, init_sbav, init_scl, init_sddc,
    init_sfsv, init_sgb, init_shlm, init_singh_heartbeat, init_singh_lineage, init_singh_migrant,
    init_singh_posthuma, init_singh_resonance, init_singh_sabi, init_singh_triage, init_ssm,
    init_subscription, init_time_lock, init_vesting_schedule, init_witnessfit,
};

/// Typed init configs — one variant per registered template. The
/// chain's `ContractEngine` pattern-matches on this to construct the
/// actual on-chain instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypedInit {
    SinghSabi(init_singh_sabi::InitConfig),
    SinghMigrant(init_singh_migrant::InitConfig),
    SinghResonance(init_singh_resonance::InitConfig),
    SinghPosthuma(init_singh_posthuma::InitConfig),
    Mayfly(init_mayfly::InitConfig),
    Sddc(init_sddc::InitConfig),
    Sfsv(init_sfsv::InitConfig),
    Shlm(init_shlm::InitConfig),
    Scl(init_scl::InitConfig),
    Sap(init_sap::InitConfig),
    SinghTriage(init_singh_triage::InitConfig),
    SinghHeartbeat(init_singh_heartbeat::InitConfig),
    SinghLineage(init_singh_lineage::InitConfig),
    Childkey(init_childkey::InitConfig),
    Mnemochain(init_mnemochain::InitConfig),
    Witnessfit(init_witnessfit::InitConfig),
    GalleryForgets(init_gallery_forgets::InitConfig),
    Sgb(init_sgb::InitConfig),
    Sbav(init_sbav::InitConfig),
    Ssm(init_ssm::InitConfig),
    RefreshMarket(init_refresh_market::InitConfig),
    Deadman(init_deadman::InitConfig),
    DecayAccessPass(init_decay_access_pass::InitConfig),
    BellOracle(init_bell_oracle::InitConfig),
    MortalDao(init_mortal_dao::InitConfig),
    Subscription(init_subscription::InitConfig),
    MortalNft(init_mortal_nft::InitConfig),
    MortalMessage(init_mortal_message::InitConfig),
    OpenBounty(init_open_bounty::InitConfig),
    Multisig(init_multisig::InitConfig),
    TimeLock(init_time_lock::InitConfig),
    VestingSchedule(init_vesting_schedule::InitConfig),
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EngineError {
    #[error("template class {0:#010x} has no registered typed materialiser")]
    UnknownTemplate(u32),
    #[error("typed init parse failed: {0}")]
    ParseFailed(String),
    /// SUB-N2 (audit 2026-05-15): init_calldata exceeds the
    /// per-deploy cap. Without this guard, every per-template
    /// `parse()` calls `serde_json::from_slice(calldata)` with no
    /// length pre-flight — a multi-MB calldata forces parse-tree
    /// allocation in the contract engine BEFORE any gas charge
    /// fires (gas is computed per-deploy at the fee-oracle level
    /// which sits downstream of materialise).
    #[error("init_calldata length {got} exceeds cap {cap}")]
    CalldataTooLarge { got: usize, cap: usize },
}

/// SUB-N2 (audit 2026-05-15): hard cap on `init_calldata.len()`
/// enforced at the dispatch level BEFORE any per-template
/// `serde_json::from_slice` runs. 64 KiB is far above legitimate
/// template configs (the largest variable-shape templates —
/// `Singh-Lineage` ladders and `Sgb` fragments — top out at a few
/// hundred bytes in practice) and well below the byte-cost-per-deploy
/// the fee oracle's per-byte surcharge prices in (`PER_FRAGMENT_BYTE`
/// = 4 gas/byte → 64 KiB ≈ 256k gas, still under MAX_TX_GAS).
pub const MAX_INIT_CALLDATA: usize = 64 * 1024;

/// Dispatch an instruction to the right typed handler.
pub fn materialise(instr: &MaterialiseInstruction) -> Result<TypedInit, EngineError> {
    let cls = instr.template_class;
    let cd = instr.init_calldata.as_slice();
    // SUB-N2: pre-flight length check. Surfaces oversized calldata
    // as a typed error before any per-template parse() allocates.
    if cd.len() > MAX_INIT_CALLDATA {
        return Err(EngineError::CalldataTooLarge {
            got: cd.len(),
            cap: MAX_INIT_CALLDATA,
        });
    }

    let typed = if cls == SINGH_SABI {
        TypedInit::SinghSabi(init_singh_sabi::parse(cd).map_err(parse_err)?)
    } else if cls == SINGH_MIGRANT {
        TypedInit::SinghMigrant(init_singh_migrant::parse(cd).map_err(parse_err)?)
    } else if cls == SINGH_RESONANCE {
        TypedInit::SinghResonance(init_singh_resonance::parse(cd).map_err(parse_err)?)
    } else if cls == SINGH_POSTHUMA {
        TypedInit::SinghPosthuma(init_singh_posthuma::parse(cd).map_err(parse_err)?)
    } else if cls == MAYFLY {
        TypedInit::Mayfly(init_mayfly::parse(cd).map_err(parse_err)?)
    } else if cls == SDDC_AUCTION {
        TypedInit::Sddc(init_sddc::parse(cd).map_err(parse_err)?)
    } else if cls == SFSV_VAULT {
        TypedInit::Sfsv(init_sfsv::parse(cd).map_err(parse_err)?)
    } else if cls == SHLM_BOUNTY {
        TypedInit::Shlm(init_shlm::parse(cd).map_err(parse_err)?)
    } else if cls == SCL_LEASE {
        TypedInit::Scl(init_scl::parse(cd).map_err(parse_err)?)
    } else if cls == SAP_AQ {
        TypedInit::Sap(init_sap::parse(cd).map_err(parse_err)?)
    } else if cls == SINGH_TRIAGE_CONTRACT {
        TypedInit::SinghTriage(init_singh_triage::parse(cd).map_err(parse_err)?)
    } else if cls == SINGH_HEARTBEAT_PULSE {
        TypedInit::SinghHeartbeat(init_singh_heartbeat::parse(cd).map_err(parse_err)?)
    } else if cls == SINGH_LINEAGE_POLICY {
        TypedInit::SinghLineage(init_singh_lineage::parse(cd).map_err(parse_err)?)
    } else if cls == CHILDKEY_LETTER {
        TypedInit::Childkey(init_childkey::parse(cd).map_err(parse_err)?)
    } else if cls == MNEMOCHAIN_CARD {
        TypedInit::Mnemochain(init_mnemochain::parse(cd).map_err(parse_err)?)
    } else if cls == WITNESSFIT_STREAK {
        TypedInit::Witnessfit(init_witnessfit::parse(cd).map_err(parse_err)?)
    } else if cls == GALLERY_FORGETS {
        TypedInit::GalleryForgets(init_gallery_forgets::parse(cd).map_err(parse_err)?)
    } else if cls == SGB_TYPE_SYSTEM {
        TypedInit::Sgb(init_sgb::parse(cd).map_err(parse_err)?)
    } else if cls == SBAV_RUNTIME {
        TypedInit::Sbav(init_sbav::parse(cd).map_err(parse_err)?)
    } else if cls == SSM_GAME_SEMANTICS {
        TypedInit::Ssm(init_ssm::parse(cd).map_err(parse_err)?)
    } else if cls == REFRESH_MARKET_NAMESPACE {
        TypedInit::RefreshMarket(init_refresh_market::parse(cd).map_err(parse_err)?)
    } else if cls == DEADMAN_SWITCH {
        TypedInit::Deadman(init_deadman::parse(cd).map_err(parse_err)?)
    } else if cls == DECAY_ACCESS_PASS {
        TypedInit::DecayAccessPass(init_decay_access_pass::parse(cd).map_err(parse_err)?)
    } else if cls == BELL_ORACLE {
        TypedInit::BellOracle(init_bell_oracle::parse(cd).map_err(parse_err)?)
    } else if cls == MORTAL_DAO {
        TypedInit::MortalDao(init_mortal_dao::parse(cd).map_err(parse_err)?)
    } else if cls == SUBSCRIPTION_SERVICE {
        TypedInit::Subscription(init_subscription::parse(cd).map_err(parse_err)?)
    } else if cls == MORTAL_NFT_GENERAL {
        TypedInit::MortalNft(init_mortal_nft::parse(cd).map_err(parse_err)?)
    } else if cls == MORTAL_MESSAGE_PILOT {
        TypedInit::MortalMessage(init_mortal_message::parse(cd).map_err(parse_err)?)
    } else if cls == OPEN_BOUNTY {
        TypedInit::OpenBounty(init_open_bounty::parse(cd).map_err(parse_err)?)
    } else if cls == MULTISIG_PROPOSAL {
        TypedInit::Multisig(init_multisig::parse(cd).map_err(parse_err)?)
    } else if cls == TIME_LOCK {
        TypedInit::TimeLock(init_time_lock::parse(cd).map_err(parse_err)?)
    } else if cls == VESTING_SCHEDULE_CLASS {
        TypedInit::VestingSchedule(init_vesting_schedule::parse(cd).map_err(parse_err)?)
    } else {
        return Err(EngineError::UnknownTemplate(cls.0));
    };

    Ok(typed)
}

fn parse_err<E: std::fmt::Display>(e: E) -> EngineError {
    EngineError::ParseFailed(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_app_templates::{catalogue, find};
    use evaporchain_app_templates_deploy::DeployRequest;
    use evaporchain_app_templates_materialise::materialise_request;
    use serde_json::json;

    fn build_instr(
        class: evaporchain_app_templates::TemplateClass,
        params: serde_json::Value,
    ) -> MaterialiseInstruction {
        let req = DeployRequest::new(class, params, [0xAB; 32], 0, 1).unwrap();
        materialise_request(&req).unwrap()
    }

    #[test]
    fn dispatches_singh_sabi() {
        let instr = build_instr(
            SINGH_SABI,
            json!({"initial_energy": 1000, "floor_pct": 15, "half_life": 365}),
        );
        let typed = materialise(&instr).unwrap();
        match typed {
            TypedInit::SinghSabi(c) => {
                assert_eq!(c.initial_energy, 1000);
                assert_eq!(c.floor_pct, 15);
                assert_eq!(c.half_life, 365);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn dispatches_mayfly() {
        let instr = build_instr(MAYFLY, json!({"initial_energy": 500, "half_life": 50}));
        match materialise(&instr).unwrap() {
            TypedInit::Mayfly(c) => {
                assert_eq!(c.initial_energy, 500);
                assert_eq!(c.half_life, 50);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn dispatches_sddc_with_full_marketplace_shape() {
        let instr = build_instr(
            SDDC_AUCTION,
            json!({"ceiling": 1000, "floor": 100, "lot_lambda": 50, "duration_epochs": 1000}),
        );
        match materialise(&instr).unwrap() {
            TypedInit::Sddc(c) => {
                assert_eq!(c.ceiling, 1000);
                assert_eq!(c.floor, 100);
                assert_eq!(c.duration_epochs, 1000);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn dispatches_sfsv_epoch_and_energy_predicates() {
        // EpochReached vault (predicate_type 0).
        let instr = build_instr(
            SFSV_VAULT,
            json!({"future_self":"0xab","predicate_type":0,"release_param":10000,"deposit_amount":5000}),
        );
        match materialise(&instr).unwrap() {
            TypedInit::Sfsv(c) => {
                assert_eq!(c.deposit_amount, 5000);
                assert_eq!(c.predicate_type, 0);
                assert_eq!(c.release_param, 10000);
            }
            _ => panic!("wrong variant"),
        }
        // EnergyDecaysBelow vault (predicate_type 1) — was undeployable
        // via the pipeline before the init_sfsv threshold-gap fix.
        let instr2 = build_instr(
            SFSV_VAULT,
            json!({"future_self":"0xab","predicate_type":1,"release_param":250,"deposit_amount":1000}),
        );
        match materialise(&instr2).unwrap() {
            TypedInit::Sfsv(c) => assert_eq!(c.predicate_type, 1),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn dispatches_lineage_with_array_ladder() {
        let instr = build_instr(
            SINGH_LINEAGE_POLICY,
            json!({
                "ladder": [
                    {"days": 90, "share_bp": 2500},
                    {"days": 180, "share_bp": 5000},
                    {"days": 365, "share_bp": 10000}
                ]
            }),
        );
        match materialise(&instr).unwrap() {
            TypedInit::SinghLineage(c) => {
                assert_eq!(c.ladder.len(), 3);
                assert_eq!(c.ladder[0].days, 90);
                assert_eq!(c.ladder[2].share_bp, 10000);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn unknown_class_is_rejected() {
        // Build an instruction by hand bypassing materialise (which
        // would have already rejected the unknown class).
        let instr = MaterialiseInstruction {
            template_class: evaporchain_app_templates::TemplateClass(0x0001_0FFF),
            instance_id: evaporchain_app_templates_materialise::InstanceId([0; 32]),
            init_calldata: b"{}".to_vec(),
        };
        let err = materialise(&instr).unwrap_err();
        assert_eq!(err, EngineError::UnknownTemplate(0x0001_0FFF));
    }

    #[test]
    fn ill_typed_calldata_is_rejected() {
        // Sabi expects integers; feed it a string for `initial_energy`.
        let instr = MaterialiseInstruction {
            template_class: SINGH_SABI,
            instance_id: evaporchain_app_templates_materialise::InstanceId([0; 32]),
            init_calldata:
                br#"{"initial_energy": "not a number", "floor_pct": 15, "half_life": 365}"#.to_vec(),
        };
        let err = materialise(&instr).unwrap_err();
        assert!(matches!(err, EngineError::ParseFailed(_)));
    }

    #[test]
    fn extra_fields_in_calldata_are_ignored() {
        // Forward-compat: a V2 client with an extra field must not
        // break V1 typed parsing.
        let instr = MaterialiseInstruction {
            template_class: MAYFLY,
            instance_id: evaporchain_app_templates_materialise::InstanceId([0; 32]),
            init_calldata:
                br#"{"initial_energy": 500, "half_life": 50, "v2_extra_field": "ignore_me"}"#
                    .to_vec(),
        };
        let typed = materialise(&instr).unwrap();
        assert!(matches!(typed, TypedInit::Mayfly(_)));
    }

    #[test]
    fn every_catalogue_entry_dispatches_via_full_pipeline() {
        // Anti-regression: for every registered template, the deploy
        // → materialise → engine pipeline produces a TypedInit
        // variant. If a future PR adds a 21st template to the
        // catalogue without adding a handler here, this fails.
        for desc in catalogue() {
            let req = DeployRequest::new(desc.class, desc.default_params.clone(), [0xAB; 32], 0, 1)
                .expect("default_params is a JSON object");
            let instr =
                materialise_request(&req).expect("default_params should pass schema validation");
            let typed = materialise(&instr).unwrap_or_else(|e| {
                panic!(
                    "engine has no handler for {:#010x} ({}): {:?}",
                    desc.class.0, desc.brand, e
                );
            });
            // Sanity: we got *some* typed variant back.
            let _ = typed;
            // Look up the descriptor too — keeps `find` exercised.
            let _ = find(desc.class).unwrap();
        }
    }

    #[test]
    fn dispatch_is_deterministic() {
        // Same instruction → same typed init, twice.
        let instr = build_instr(
            SINGH_SABI,
            json!({"initial_energy": 1000, "floor_pct": 15, "half_life": 365}),
        );
        let a = materialise(&instr).unwrap();
        let b = materialise(&instr).unwrap();
        assert_eq!(a, b);
    }
}
