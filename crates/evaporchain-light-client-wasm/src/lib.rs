//! WASM bridge for the EvaporChain Light Client SDK.
//!
//! Browsers / mobile-WASM-runtimes call into this crate via wasm-bindgen
//! async functions. The HTTP layer is `gloo-net::http::Request` which
//! delegates to the browser's `fetch` — no Tokio, no native sockets, no
//! `evaporchain-light-client-http` (which is native-only via `ureq`).
//!
//! ## Design — sync/async fork
//!
//! The SDK's `RpcTransport` trait is sync. Browser fetch is async-only.
//! Rather than introducing an async trait variant in the SDK core (would
//! be a load-bearing breaking change), this crate uses the SDK's
//! verifier primitives DIRECTLY — `LightClient::ingest_block`,
//! `LightClient::verify_state` — bypassing `RpcTransport` entirely.
//! Each WASM-exported async method:
//!
//!   1. `await`s the browser fetch via gloo-net,
//!   2. deserialises the response JSON to the SDK's chain types,
//!   3. calls the SDK's pure (non-trait) verification methods.
//!
//! Result: full BFT BLS + Verkle Pasta-curve Pedersen verification runs
//! in the browser without trusting the node, and the SDK core stays
//! sync-only + WASM-friendly.
//!
//! ## Cross-references
//!
//! - `evaporchain-light-client-http` — native HTTP transport (ureq).
//!   Same conceptual flow; this crate is the browser counterpart.
//! - `evaporchain-light-client-cli` — operator binary. Worked example
//!   for the native path.
//! - `INVENTION_STACK.md §4.1 row 8` — Lambda-Fold doctrine the SDK
//!   operationalises, now extended to browser consumers.

use evaporchain_light_client::transport::TransportError;
use evaporchain_light_client::{LightClient, LightClientError};
use evaporchain_light_client::LightBlockHeader;
use serde::Deserialize;
use wasm_bindgen::prelude::*;

// Note: the SDK's pub re-exports include `LightBlockHeader` (from
// evaporchain-consensus). State-proof verification needs the
// `EnergyVerkleProof` type from evaporchain-crypto, which is not
// re-exported by the SDK. We re-deserialise it via the SDK's
// `verify_state` method which takes `&EnergyVerkleProof` —  pull it in
// from the SDK's public types tree.
use evaporchain_light_client::state_query::verify_state;
// Re-import from the crypto layer for the JSON deserialise.
use evaporchain_light_client::*;

// ─── panic hook — surfaces Rust panics in the browser console ────────

#[wasm_bindgen(start)]
pub fn _start() {
    console_error_panic_hook::set_once();
}

// ─── error type — wasm-bindgen-friendly ──────────────────────────────

fn js_err(msg: impl AsRef<str>) -> JsValue {
    JsValue::from_str(msg.as_ref())
}

fn map_lc_err(e: LightClientError) -> JsValue {
    js_err(format!("light client: {e}"))
}

fn map_transport_err(e: TransportError) -> JsValue {
    js_err(format!("transport: {e}"))
}

// ─── HTTP fetch helpers via gloo-net ──────────────────────────────────

/// GET a JSON response from `<base>/<path>`, deserialising into `T`.
async fn fetch_json<T: for<'de> Deserialize<'de>>(
    base: &str,
    path: &str,
) -> Result<T, TransportError> {
    let url = format!("{base}{path}");
    let resp = gloo_net::http::Request::get(&url)
        .send()
        .await
        .map_err(|e| TransportError::Network(format!("fetch {url}: {e}")))?;
    if resp.status() == 404 {
        return Err(TransportError::NotFound);
    }
    if !resp.ok() {
        return Err(TransportError::Backend(format!(
            "HTTP {} from {url}",
            resp.status()
        )));
    }
    resp.json::<T>()
        .await
        .map_err(|e| TransportError::Parse(format!("decode {url}: {e}")))
}

// ─── WasmLightClient — the browser-facing handle ─────────────────────

/// Browser-facing Light Client. Wraps the SDK's `LightClient` and
/// provides async methods that fetch via `gloo-net` (browser fetch).
///
/// Construct via [`WasmLightClient::anchor`] which fetches a header
/// from the node and BFT-verifies it as the trust anchor.
#[wasm_bindgen]
pub struct WasmLightClient {
    inner: LightClient,
    base_url: String,
}

#[wasm_bindgen]
impl WasmLightClient {
    /// Anchor a new light client at the chain's reported latest header
    /// (or a specific height if provided). The header is BFT-verified
    /// at construction.
    ///
    /// `node_url`: e.g. `"http://node.example.com:8081"` (no trailing slash).
    /// `genesis_height`: `null`/`undefined` = anchor at chain latest.
    /// `current_time_secs`: caller-supplied UNIX time (browsers can use
    ///   `Math.floor(Date.now() / 1000)`); used for trust-period checks.
    pub async fn anchor(
        node_url: String,
        genesis_height: Option<u64>,
        current_time_secs: u64,
    ) -> Result<WasmLightClient, JsValue> {
        let path = match genesis_height {
            Some(h) => format!("/api/light_header/{h}"),
            None => "/api/light_header/latest".to_string(),
        };
        let header: LightBlockHeader = fetch_json(&node_url, &path)
            .await
            .map_err(map_transport_err)?;
        let inner = LightClient::new(header, current_time_secs, /* vk_bytes */ None);
        Ok(Self {
            inner,
            base_url: node_url,
        })
    }

    /// Walk forward from the trusted tip to the chain's latest header,
    /// BFT-verifying every block. Returns the new trusted tip height
    /// after the walk.
    pub async fn sync_to_latest(&mut self, current_time_secs: u64) -> Result<u64, JsValue> {
        // Fetch latest from the node.
        let latest: LightBlockHeader = fetch_json(&self.base_url, "/api/light_header/latest")
            .await
            .map_err(map_transport_err)?;
        let target = latest.height;

        // Walk forward height-by-height. Same loop as the sync.rs path
        // in the SDK but driven via gloo-net fetches instead of the sync
        // RpcTransport.
        let mut cur = self.inner.current_height();
        while cur < target {
            cur += 1;
            let header: LightBlockHeader =
                fetch_json(&self.base_url, &format!("/api/light_header/{cur}"))
                    .await
                    .map_err(map_transport_err)?;
            self.inner
                .ingest_block(header, current_time_secs)
                .map_err(map_lc_err)?;
        }

        Ok(self.inner.current_height())
    }

    /// Verify a state-query proof for a 32-byte trie key against the
    /// trusted tip's state_root. Returns the value as hex (or empty
    /// string for non-membership proofs).
    pub async fn fetch_and_verify_state_hex(
        &self,
        trie_key_hex: String,
    ) -> Result<JsValue, JsValue> {
        let key_hex = trie_key_hex
            .strip_prefix("0x")
            .unwrap_or(&trie_key_hex);
        if key_hex.len() != 64 {
            return Err(js_err(format!(
                "trie_key_hex must be 64-char hex, got {}",
                key_hex.len()
            )));
        }
        // Fetch the proof via gloo-net.
        let proof: evaporchain_crypto::energy_verkle::EnergyVerkleProof =
            fetch_json(&self.base_url, &format!("/api/state/proof/{key_hex}"))
                .await
                .map_err(map_transport_err)?;
        // Verify against the trusted state_root using the SDK's pure
        // verification primitive. expected_value=None → just verify the
        // proof binds.
        verify_state(&proof, None).map_err(map_lc_err)?;
        // Return the proof's value as hex (or empty for non-membership).
        match proof.value {
            Some(v) => Ok(JsValue::from_str(&hex::encode(v))),
            None => Ok(JsValue::from_str("")),
        }
    }

    /// Current trusted tip height.
    #[wasm_bindgen(getter)]
    pub fn current_height(&self) -> u64 {
        self.inner.current_height()
    }

    /// Current trusted tip state_root as hex (no `0x` prefix).
    #[wasm_bindgen(getter)]
    pub fn current_state_root(&self) -> String {
        hex::encode(self.inner.current_state_root())
    }

    /// Trust-period in seconds (default 14 days; configurable on the
    /// SDK via `with_trust_period`).
    #[wasm_bindgen(getter)]
    pub fn trust_period_secs(&self) -> u64 {
        self.inner.trust_period_secs()
    }
}
