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

pub mod init_childkey;
pub mod init_gallery_forgets;
pub mod init_mayfly;
pub mod init_mnemochain;
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
pub mod init_witnessfit;

pub use dispatch::{materialise, EngineError, TypedInit};
