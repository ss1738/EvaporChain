//! Protocol types for EvaporChain BFT consensus.
//!
//! Extracted from `evaporchain-consensus` 2026-05-08 to break the
//! Light Client SDK's transitive dep on `evaporchain-state` (which
//! pulls RocksDB and breaks `wasm32-unknown-unknown` builds — see
//! `crates/evaporchain-light-client-wasm/README.md` for the full
//! diagnosis).
//!
//! This crate hosts ONLY the types + the BLS-using verifier:
//!
//!   * `LightBlockHeader` — minimal header for light-client verification
//!   * `TrustedState` — light-client anchor + commit history
//!   * `VerificationResult` — what `LightClientVerifier::verify` returns
//!   * `LightClientError` — verification error variants
//!   * `ValidatorInfo`, `ValidatorSet` — types used by the verifier
//!   * `LightClientVerifier` — the BFT BLS aggregate-sig verifier itself
//!
//! `evaporchain-consensus` continues to host the runtime (Tendermint
//! loop, mempool, fork-choice, slashing, epoch-transition manager,
//! validator-set governance) — those are state-DB-attached and
//! native-only. They re-export from this crate so the existing
//! consensus API surface stays stable.
//!
//! Browser / mobile / WASM consumers depend on this crate (+
//! `evaporchain-types` + `evaporchain-crypto`) directly, NOT on
//! `evaporchain-consensus`.

// Phase 1: empty scaffold. Type extraction happens in Phase 2-4.
