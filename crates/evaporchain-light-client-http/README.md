# evaporchain-light-client-http

HTTP transport for [`evaporchain-light-client`](../evaporchain-light-client). Implements the SDK's `RpcTransport` trait over `ureq` — a sync, ~10-dependency HTTP client.

## When to use this crate

- **You're a native Rust consumer** (CLI, server-side service, desktop app) and want a drop-in transport so you don't have to wire HTTP yourself.
- **You're prototyping** and want the canonical reference implementation of `RpcTransport` to look at.

## When NOT to use this crate

- **Browser / WASM**: `ureq` makes native blocking sockets. Implement `RpcTransport` directly over `web-sys::fetch` or `gloo-net::http::Request`.
- **Async runtime (tokio etc.) where blocking is unacceptable**: this transport is sync. Either bridge via `spawn_blocking` (cheap, perfectly fine for light-client polling at chain-block cadence) or implement an async `RpcTransport` over `reqwest` + bridge into the SDK's sync trait via `block_on`.
- **Embedded / no_std**: too heavy.

## Quickstart

```rust
use evaporchain_light_client::{LightClient, RpcTransport};
use evaporchain_light_client_http::HttpTransport;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let transport = HttpTransport::new("http://my-node.example.com:8081");
let now = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)?
    .as_secs();

let genesis = transport.fetch_header_at(15190)?;
let mut lc = LightClient::new(genesis, now, /* vk_bytes */ None);
lc.sync_to_latest(&transport, now)?;
# Ok(()) }
```

## Configurable URL paths

The chain's default endpoint shape is hard-coded into the transport's defaults. If your gateway exposes the SDK's required endpoints under different paths, override via `with_paths`:

```rust
# use evaporchain_light_client_http::HttpTransport;
let transport = HttpTransport::new("https://gateway.example.com").with_paths(
    "/v2/chain/header/{height}",       // header at height
    "/v2/chain/header/latest",          // latest header
    "/v2/chain/state/{key_hex}/proof",  // state-query proof
    "/v2/chain/nova/attestation",       // running Nova attestation
    "/v2/chain/nova/vk",                // chain's compiled vk_bytes
);
```

Default templates are exported as `DEFAULT_HEADER_PATH`, `DEFAULT_LATEST_HEADER_PATH`, `DEFAULT_STATE_PROOF_PATH`, `DEFAULT_NOVA_ATTESTATION_PATH`, `DEFAULT_NOVA_VK_BYTES_PATH` constants.

## Auth gateways

```rust
# use evaporchain_light_client_http::HttpTransport;
let transport = HttpTransport::new("https://protected.example.com")
    .with_bearer_token("eyJ...");
```

Sent as `Authorization: Bearer <token>` on every request.

## Expected response schemas

| Endpoint | JSON body |
|---|---|
| Header at height / latest | `evaporchain_consensus::light_client::LightBlockHeader` |
| State-query proof | `evaporchain_crypto::energy_verkle::EnergyVerkleProof` |
| Nova attestation | `evaporchain_lambda_fold::nova_path::NovaFoldedInstance` |
| vk_bytes | `{ "vk_bytes_hex": "<hex>" }` |

## Error mapping

| HTTP outcome | Maps to `TransportError` variant |
|---|---|
| 404 | `NotFound` |
| Other status (4xx/5xx) | `Backend(...)` |
| Connection / TLS / DNS | `Network(...)` |
| Body decode failure | `Parse(...)` |

## Cross-references

- [`evaporchain-light-client`](../evaporchain-light-client) — SDK core (transport-agnostic).
- [`evaporchain-light-client-cli`](../evaporchain-light-client-cli) — operator CLI binary, wires this transport.
- `tests/e2e_http.rs` — synthetic stdlib-only HTTP-server e2e tests; useful as a reference when implementing alternative transports.
- `docs/runbooks/light-client-cli.md` — operator runbook covering CLI usage.
