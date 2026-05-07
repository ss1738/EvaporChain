//! Optional transport abstraction for fetching chain data from a
//! node.
//!
//! The SDK proper does not bundle an HTTP client (keeps WASM-target
//! builds lean and lets consumers pick their own runtime). Instead,
//! consumers implement [`RpcTransport`] for whatever transport they
//! have — `reqwest` on tokio servers, `wasm-bindgen` `fetch` in
//! browsers, native `URLSession` over FFI on iOS, etc. — and pass
//! it to the higher-level sync helpers on [`LightClient`].
//!
//! ## Sync vs async
//!
//! [`RpcTransport`] is a SYNCHRONOUS trait. Consumers in async
//! environments wrap their async calls behind a sync facade
//! (typically by blocking on a runtime, or bridging via a
//! Web-Worker `Atomics.wait` pattern in browser contexts). This
//! keeps the SDK's core interface uniform across native / WASM /
//! FFI consumers and avoids pulling in `async-trait` (a heavy
//! dependency that's still evolving).
//!
//! For consumers who want a fully-async surface, a thin
//! `evaporchain-light-client-async` add-on crate can be added
//! later to wrap [`RpcTransport`] in `async fn`s — but that's
//! out of scope for the core SDK.
//!
//! ## What the trait fetches
//!
//! The methods cover the four data classes the SDK needs:
//!
//! - **Block headers** (`fetch_header_at` + `fetch_latest_header`)
//!   for BFT verification.
//! - **State proofs** (`fetch_state_proof`) for Verkle state-query
//!   verification.
//! - **Nova attestation + vk_bytes** (`fetch_nova_attestation` +
//!   `fetch_vk_bytes`, feature `nova`) for Nova-IVC sublinear
//!   verification.

use evaporchain_consensus::light_client::LightBlockHeader;
use evaporchain_crypto::energy_verkle::EnergyVerkleProof;

/// Errors a transport can surface. Keep the variants narrow so
/// the SDK can map them into [`crate::LightClientError`] cleanly.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// The requested resource doesn't exist at the node (e.g.,
    /// querying a height past tip, or a state key that's not
    /// committed).
    #[error("resource not found")]
    NotFound,

    /// Network-layer error (TCP reset, TLS handshake failed, DNS
    /// lookup failed, browser fetch rejected, etc.). The string
    /// is consumer-supplied for diagnostics.
    #[error("network error: {0}")]
    Network(String),

    /// The node responded but the response body couldn't be parsed
    /// as the expected type (malformed JSON, truncated bytes,
    /// schema drift, etc.).
    #[error("parse error: {0}")]
    Parse(String),

    /// Transport backend reported an error not covered above.
    #[error("transport backend error: {0}")]
    Backend(String),
}

/// The data-fetch interface consumers implement. Methods are sync
/// — see module-level docs for async-bridging guidance.
pub trait RpcTransport {
    /// Fetch a block header at the given height. Used by
    /// [`crate::LightClient::sync_to_height`] to walk forward
    /// from the trusted tip.
    fn fetch_header_at(&self, height: u64) -> Result<LightBlockHeader, TransportError>;

    /// Fetch the most-recent block header from the node. Used by
    /// [`crate::LightClient::sync_to_latest`].
    fn fetch_latest_header(&self) -> Result<LightBlockHeader, TransportError>;

    /// Fetch a Verkle state-query proof for the given key. The
    /// proof's `value` field is `Some(_)` for membership,
    /// `None` for non-membership.
    fn fetch_state_proof(&self, key: &[u8; 32]) -> Result<EnergyVerkleProof, TransportError>;

    /// Fetch the chain's running Nova-folded instance (proof +
    /// witness). Feature-gated because most consumers don't need
    /// the sublinear path.
    #[cfg(feature = "nova")]
    fn fetch_nova_attestation(
        &self,
    ) -> Result<crate::nova::NovaAttestation, TransportError>;

    /// Fetch the chain's compiled `vk_bytes`. Typically called
    /// once at SDK initialization and cached. Feature-gated
    /// alongside [`Self::fetch_nova_attestation`].
    #[cfg(feature = "nova")]
    fn fetch_vk_bytes(&self) -> Result<Vec<u8>, TransportError>;
}

/// Map a [`TransportError`] into the SDK's unified error type.
/// Pure mechanical translation; useful in `?`-chains.
pub fn transport_err_to_sdk(e: TransportError) -> crate::LightClientError {
    crate::LightClientError::Protocol(format!("transport: {e}"))
}

// ───────────────────────── tests ───────────────────────────────

#[cfg(test)]
pub(crate) mod test_transport {
    //! In-memory mock transport for SDK tests. Holds pre-built
    //! headers + state proofs in maps; consumers (in tests) seed
    //! it with known data and pass it to the sync helpers.

    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    pub struct MockTransport {
        pub headers: Mutex<BTreeMap<u64, LightBlockHeader>>,
        pub state_proofs: Mutex<BTreeMap<[u8; 32], EnergyVerkleProof>>,
        #[cfg(feature = "nova")]
        pub nova_attestation: Mutex<Option<crate::nova::NovaAttestation>>,
        #[cfg(feature = "nova")]
        pub vk_bytes: Mutex<Option<Vec<u8>>>,
    }

    impl MockTransport {
        pub fn new() -> Self {
            Self {
                headers: Mutex::new(BTreeMap::new()),
                state_proofs: Mutex::new(BTreeMap::new()),
                #[cfg(feature = "nova")]
                nova_attestation: Mutex::new(None),
                #[cfg(feature = "nova")]
                vk_bytes: Mutex::new(None),
            }
        }

        pub fn insert_header(&self, header: LightBlockHeader) {
            self.headers.lock().unwrap().insert(header.height, header);
        }

        pub fn insert_state_proof(&self, key: [u8; 32], proof: EnergyVerkleProof) {
            self.state_proofs.lock().unwrap().insert(key, proof);
        }
    }

    impl RpcTransport for MockTransport {
        fn fetch_header_at(&self, height: u64) -> Result<LightBlockHeader, TransportError> {
            self.headers
                .lock()
                .unwrap()
                .get(&height)
                .cloned()
                .ok_or(TransportError::NotFound)
        }

        fn fetch_latest_header(&self) -> Result<LightBlockHeader, TransportError> {
            self.headers
                .lock()
                .unwrap()
                .iter()
                .next_back()
                .map(|(_, h)| h.clone())
                .ok_or(TransportError::NotFound)
        }

        fn fetch_state_proof(&self, key: &[u8; 32]) -> Result<EnergyVerkleProof, TransportError> {
            self.state_proofs
                .lock()
                .unwrap()
                .get(key)
                .cloned()
                .ok_or(TransportError::NotFound)
        }

        #[cfg(feature = "nova")]
        fn fetch_nova_attestation(
            &self,
        ) -> Result<crate::nova::NovaAttestation, TransportError> {
            self.nova_attestation
                .lock()
                .unwrap()
                .clone()
                .ok_or(TransportError::NotFound)
        }

        #[cfg(feature = "nova")]
        fn fetch_vk_bytes(&self) -> Result<Vec<u8>, TransportError> {
            self.vk_bytes
                .lock()
                .unwrap()
                .clone()
                .ok_or(TransportError::NotFound)
        }
    }
}
