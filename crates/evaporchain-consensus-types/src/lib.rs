//! Protocol types for EvaporChain BFT consensus.
//!
//! Extracted from `evaporchain-consensus` 2026-05-08 to break the
//! Light Client SDK's transitive dep on `evaporchain-state` (which
//! pulls RocksDB and breaks `wasm32-unknown-unknown` builds — see
//! `crates/evaporchain-light-client-wasm/README.md` for the full
//! diagnosis).
//!
//! ## Status: PHASE 1 SCAFFOLD — types not yet moved.
//!
//! This file currently contains only the extraction spec. The actual
//! type movements (Phases 2-4) are tractable but mechanical surgery
//! that needs a focused block. Each phase below is independently
//! shippable; once all four are merged, `evaporchain-light-client`
//! drops its `evaporchain-consensus` dep entirely (Phase 5) and the
//! WASM scaffold at `crates/evaporchain-light-client-wasm/` builds.
//!
//! ## Extraction spec
//!
//! ### Phase 2: leaf types from `light_client.rs`
//!
//! Move FROM `crates/evaporchain-consensus/src/light_client.rs`:
//!
//! | Item | Source line | Notes |
//! |---|---|---|
//! | `LightBlockHeader` | 27-38 | Carries a `ValidatorSet` field — depends on Phase 3 |
//! | `TrustedState` | 42-45 | Pure |
//! | `VerificationResult` | 49-59 | Pure |
//! | `LightClientError` | 63-69 | Pure |
//! | `TRUST_PERIOD_SECS` const | 75 | Pure |
//! | `TRUST_THRESHOLD_NUMERATOR` / `TRUST_THRESHOLD_DENOMINATOR` consts | 79-80 | Pure |
//! | `MAX_SKIP_HEIGHT_GAP` const | 83 | Pure |
//!
//! `evaporchain-consensus`'s `light_client.rs` adds `pub use evaporchain_consensus_types::*;`
//! to keep the existing API stable for downstream callers.
//!
//! ### Phase 3: ValidatorInfo + ValidatorSet types
//!
//! Move FROM `crates/evaporchain-consensus/src/validator_set.rs`:
//!
//! | Item | Source line | Move target |
//! |---|---|---|
//! | `ValidatorInfo` struct + `impl` | 49-180 | Full move |
//! | `ValidatorSet` struct | 186-188 | Full move |
//! | `ValidatorSet` core methods | 190-~500 | The pure-type methods: `new`, `with_validators`, `get_validator`, `get_validator_by_id`, `validators`, `len`, `is_empty`, `total_stake`, `effective_stake`, etc. |
//! | `Default for ValidatorSet` | 710-712 | Pure |
//! | `HEALTH_BONUS_CAP`, `MAX_HEALTH_SCORE`, `MIN_STAKE` | 25-37 | Pure consts (used by ValidatorInfo::effective_weight) |
//!
//! Stay in `evaporchain-consensus/src/validator_set.rs`:
//! - Slashing methods on `ValidatorSet` (mutations that touch slashed_pool, jailed flags, etc) — these belong with the consensus runtime.
//! - `slash_delegations_for_validator` free fn (line 732)
//! - `ValidatorSetChange` enum (line 782)
//! - `EpochTransitionResult` (line 793) + `EpochTransitionManager` (line 822)
//! - `ValidatorSetSource` trait impl (line 1073) — depends on consensus's trait
//! - Slashing constants (`SLASH_EQUIVOCATION_PCT`, `SLASH_DOWNTIME_PCT`, etc).
//!
//! Tactical recipe:
//! 1. Copy ValidatorInfo / ValidatorSet types + their pure-type methods to this crate.
//! 2. In `evaporchain-consensus/src/validator_set.rs`, replace the moved type definitions with `pub use evaporchain_consensus_types::{ValidatorInfo, ValidatorSet};`.
//! 3. The slashing methods stay as a separate `impl ValidatorSet { ... }` block in consensus — Rust allows splitting impls across crates as long as they're in the trait-impl-coherence-permitted patterns. (For inherent impls on a foreign type, you can't split, so the slashing methods need to become free fns OR a trait. Probably free fns is simpler.)
//!
//! Estimated complexity: ~1 hour. Test surface: existing validator-set unit tests + the consensus-level integration tests that exercise slashing.
//!
//! ### Phase 4: LightClientVerifier
//!
//! Move FROM `light_client.rs`:
//!
//! | Item | Source line | Notes |
//! |---|---|---|
//! | `LightClientVerifier` struct | 88-93 | Holds `BTreeMap<u64, TrustedState>` |
//! | `LightClientVerifier::impl` | 95-end | All methods including `verify`, `with_trust_period`, `current_height`, `current_state_root`, `trust_period_secs` |
//!
//! Dependencies (already in this crate's Cargo.toml):
//! - `evaporchain_crypto::signatures::{BlsPublicKey, BlsSignature, BlsVerifier}` for the BFT BLS aggregate-sig check.
//! - `evaporchain_crypto::hash::blake3_hash` for header hashing.
//! - `evaporchain_types::CommitCertificate` for the cert type.
//!
//! After this phase, the Light Client SDK can depend on `evaporchain-consensus-types` directly and stop pulling `evaporchain-consensus`.
//!
//! ### Phase 5: switch SDK dep
//!
//! In `crates/evaporchain-light-client/Cargo.toml`:
//!
//! ```diff
//! - evaporchain-consensus = { path = "../evaporchain-consensus" }
//! + evaporchain-consensus-types = { path = "../evaporchain-consensus-types" }
//! ```
//!
//! Update imports in `client.rs`, `nova.rs`, `transport.rs`, `state_query.rs`, `sync.rs`, and `lib.rs` from
//! `use evaporchain_consensus::light_client::{...}` to
//! `use evaporchain_consensus_types::{...}`.
//!
//! Verify `cargo check -p evaporchain-light-client` passes WITHOUT pulling `evaporchain-state` in the dep graph (`cargo tree -p evaporchain-light-client | grep evaporchain-state` should return nothing).
//!
//! After Phase 5, the WASM scaffold at `crates/evaporchain-light-client-wasm/` builds modulo the BLS issue (Refactor B).
//!
//! ## Refactor B (separate; not in this crate)
//!
//! `evaporchain-crypto` depends on `blst` (C library) which doesn't compile to wasm32. To unblock the browser-side path, abstract the BLS backend behind a feature flag:
//!
//! ```toml
//! # In crates/evaporchain-crypto/Cargo.toml:
//! [features]
//! default = ["bls-native"]
//! bls-native = ["blst"]
//! bls-portable = ["bls12_381"]  # pure-Rust, wasm-friendly
//! ```
//!
//! With both Refactors A + B done, the WASM bridge crate at `crates/evaporchain-light-client-wasm/` builds against `wasm32-unknown-unknown` with the existing source unchanged. The full browser-side BFT BLS + Verkle Pasta-curve Pedersen verification path is then operational.

// Phase 2-4 type definitions land here.
