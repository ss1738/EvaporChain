//! Per-template typed materialiser registry.
//!
//! `app-templates-materialise` produces a [`MaterialiseInstruction`]
//! — a dispatch envelope `{ template_class, instance_id,
//! init_calldata }`. This crate is the registry that consumes that
//! envelope: each `template_class` maps to a typed handler that
//! parses `init_calldata` (canonical JSON bytes) into a typed
//! `InitConfig` struct the chain's `ContractEngine` instantiates.
//!
//! ## What this crate is
//!
//! A **dispatch table**, not the contracts themselves:
//!
//! - 20 thin per-template `init_*.rs` modules, one per registered
//!   template, each defining `InitConfig` (the typed shape) and a
//!   `parse(calldata) -> Result<InitConfig, ParseError>` function.
//! - `dispatch.rs` — the registry: `materialise(instr) ->
//!   Result<TypedInit, EngineError>`. Match on `template_class`,
//!   call the right handler.
//!
//! ## Why JSON parsing, not bincode
//!
//! `init_calldata` is canonical JSON bytes (the materialise layer
//! produces them via `serde_json::to_vec` of a key-sorted Value).
//! Parsing JSON into a typed struct is cheap and lets handlers fail
//! with precise errors ("expected u64 in `initial_energy`, got
//! string"). This is execution-time; we're already doing real
//! work, the JSON parse is in the noise.
//!
//! ## What this crate does NOT do
//!
//! - It does NOT instantiate the contract — it produces a typed
//!   `TypedInit` enum variant which the chain's `ContractEngine`
//!   pattern-matches on to construct the actual on-chain state.
//! - It does NOT enforce business rules (e.g., `floor < ceiling`).
//!   That's the per-contract logic. This layer enforces *types*
//!   only: u64 fields are u64, strings are strings, etc.
//!
//! ## Module map
//!
//! - [`dispatch`] — [`materialise`] driver + [`TypedInit`] +
//!   [`EngineError`].
//! - One `init_*` module per registered template, exposing the
//!   typed `InitConfig` and `parse` function.

pub mod dispatch;

pub mod init_bell_oracle;
pub mod init_childkey;
pub mod init_deadman;
pub mod init_decay_access_pass;
pub mod init_gallery_forgets;
pub mod init_mayfly;
pub mod init_mnemochain;
pub mod init_mortal_dao;
pub mod init_mortal_message;
pub mod init_mortal_nft;
pub mod init_multisig;
pub mod init_open_bounty;
pub mod init_refresh_market;
pub mod init_sap;
pub mod init_sbav;
pub mod init_scl;
pub mod init_sddc;
pub mod init_sfsv;
pub mod init_sgb;
pub mod init_shlm;
pub mod init_singh_heartbeat;
pub mod init_singh_lineage;
pub mod init_singh_migrant;
pub mod init_singh_posthuma;
pub mod init_singh_resonance;
pub mod init_singh_sabi;
pub mod init_singh_triage;
pub mod init_ssm;
pub mod init_subscription;
pub mod init_time_lock;
pub mod init_witnessfit;

pub use dispatch::{materialise, EngineError, TypedInit};

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use evaporchain_app_templates::class::MAYFLY;
    use evaporchain_app_templates_materialise::MaterialiseInstruction;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "engine::materialise dispatches a
    /// MaterialiseInstruction to the registered typed handler for
    /// its template_class. Valid Mayfly init JSON parses; unknown
    /// classes produce EngineError::UnknownTemplate; malformed JSON
    /// produces EngineError::ParseFailed. Per-handler InitConfig is
    /// type-shaped after dispatch."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Valid Mayfly init JSON.
        let cd = serde_json::to_vec(&serde_json::json!({
            "initial_energy": 1000u64,
            "half_life": 100u64,
        }))
        .unwrap();
        let mut iid = [0u8; 32];
        iid[0] = 0xAA;
        let instr = MaterialiseInstruction {
            template_class: MAYFLY,
            instance_id: evaporchain_app_templates_materialise::InstanceId(iid),
            init_calldata: cd,
        };
        let typed = materialise(&instr).unwrap();
        assert!(matches!(typed, TypedInit::Mayfly(_)));

        // Unknown template class.
        let bogus = MaterialiseInstruction {
            template_class: evaporchain_app_templates::TemplateClass(0xFFFF_FFFF),
            instance_id: evaporchain_app_templates_materialise::InstanceId(iid),
            init_calldata: vec![],
        };
        assert!(matches!(
            materialise(&bogus),
            Err(EngineError::UnknownTemplate(_))
        ));

        // Malformed JSON for valid class.
        let bad_json = MaterialiseInstruction {
            template_class: MAYFLY,
            instance_id: evaporchain_app_templates_materialise::InstanceId(iid),
            init_calldata: vec![b'!'],
        };
        assert!(matches!(
            materialise(&bad_json),
            Err(EngineError::ParseFailed(_))
        ));
    }
}
