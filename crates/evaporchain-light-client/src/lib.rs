//! EvaporChain Light Client SDK.
//!
//! Foundation for any consumer that needs to verify EvaporChain state
//! without running a full validator: browser wallets, mobile apps,
//! dapps, bridges, explorers, embedded verifiers.
//!
//! ## What this SDK does
//!
//! Three verification layers, composed into a single
//! [`LightClient`] interface:
//!
//! 1. **BFT commit-certificate verification** — wraps
//!    [`evaporchain_consensus_types::LightClientVerifier`].
//!    Each block header carries a BLS-aggregate `CommitCertificate`
//!    proving 2/3+ validator stake attested to it. The SDK verifies
//!    the certificate against the validator set and tracks trust-
//!    period expiry per Tendermint light-client spec (ICS-007).
//!
//! 2. **Nova-IVC sublinear block-validity verification** (feature
//!    `nova`). Wraps
//!    [`evaporchain_proving::RealBlockProver::verify_with_vk_bytes`].
//!    Light client holds only `vk_bytes` (~few KB, fixed-size); each
//!    block's energy-fold witness is verified in ~23 ms at any chain
//!    length (sublinear claim empirically locked at 1.083× of 10
//!    folds per the Phase 6.1 benchmark). This operationalises the
//!    Layer 5 Lambda-Fold Real Nova investment by exposing the
//!    sublinear path at the SDK level.
//!
//! 3. **Verkle state-query verification** (next commit). Wraps
//!    [`evaporchain_crypto::verkle`] proof verification. After
//!    verifying a block header, the SDK can query state via
//!    `(key, value, merkle_proof)` triples and confirm the proof
//!    binds against the trusted state root.
//!
//! ## Design
//!
//! - **No native-only deps** at the SDK level — WASM-target
//!   compatible (with `default-features = false` when consumed).
//! - **No HTTP client** in the core (separate `http` feature for
//!   the optional RPC layer; default consumers bring their own
//!   transport).
//! - **Single `LightClient` struct** consolidates the three
//!   verifier layers; consumers call `ingest_block` then
//!   `verify_state` for state queries.
//!
//! ## Cross-references
//!
//! - `INVENTION_STACK.md §4.1 row 8` — Lambda-Fold doctrine.
//! - `LAMBDA_FOLD_NOVA_PLAN.md` Phase 5 — Tendermint-side
//!   Nova integration that this SDK consumes.
//! - `crates/evaporchain-consensus/src/light_client.rs` — the BFT
//!   verifier this SDK wraps.
//! - `crates/evaporchain-proving/src/nova.rs` — the Nova verifier
//!   this SDK wraps.

#![cfg_attr(not(any(test, feature = "nova")), allow(dead_code))]

pub mod client;
pub mod error;
pub mod state_query;
pub mod sync;
pub mod transport;

#[cfg(feature = "nova")]
pub mod nova;

pub use transport::{RpcTransport, TransportError};

pub use client::LightClient;
pub use error::LightClientError;

// Re-exports of the underlying types so consumers don't need to
// depend on consensus + proving + crypto directly. Each consumer
// gets a single SDK dep with all the types they need.
pub use evaporchain_consensus_types::{LightBlockHeader, TrustedState, VerificationResult};
