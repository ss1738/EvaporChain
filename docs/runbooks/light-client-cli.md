# Light Client CLI — Operator Runbook

How to probe an EvaporChain node via the `evaporchain-light-client` CLI. Intended audience: operators running a node and wanting a quick way to verify its state from outside the consensus loop, plus integrators wiring the SDK into a downstream product (mobile wallet, dapp, bridge, explorer) who want a runnable reference.

**Pairs with:** `crates/evaporchain-light-client-cli` (the binary), `crates/evaporchain-light-client` (SDK core), `crates/evaporchain-light-client-http` (HTTP transport).

---

## What the CLI does

Three subcommands, all driving the SDK against a running node over HTTP:

| Subcommand | Purpose | Verification layers exercised |
|---|---|---|
| `sync-latest` | Walk the chain forward from a trust anchor to the chain's tip | BFT BLS aggregate-sig per block |
| `get-state`   | Fetch + verify a state-query proof for a given trie key or account address | Verkle Pasta-curve Pedersen commitments |
| `watch`       | Follow the chain forward indefinitely, polling at a fixed cadence | BFT BLS per ingested block |

All three are **client-side only** — they make HTTP GET requests to the node's `/api/...` endpoints. They never write, never modify chain state, never restart anything.

## Build

```bash
cd ~/EvaporChain
cargo build --release --bin evaporchain-light-client
```

Binary lands at `target/release/evaporchain-light-client`. Output is a single static-linked Rust binary (~10 MB).

## Prerequisites

The target node must expose:

- `GET /api/light_header/:height` — JSON `LightBlockHeader`
- `GET /api/light_header/latest` — JSON `LightBlockHeader`
- `GET /api/state/proof/:key_hex` — JSON `EnergyVerkleProof`

These ship with `evaporchain-node` from commit `e56359a` (2026-05-07) onward. Nodes built before that commit will return 404 for `/api/state/proof/:key_hex`; the `get-state` subcommand will surface this as a transport error.

## Subcommand: `sync-latest`

Walk forward from a known-good height to the chain's reported latest, BFT-verifying every block along the way.

### Flags

| Flag | Required | Purpose |
|---|---|---|
| `--node URL` | yes | Node base URL, e.g. `http://localhost:8081`. No trailing slash. |
| `--genesis-height N` | no | Initial trust-anchor height. If unset, the CLI seeds at the chain's reported latest (zero-walk — useful when you already trust the node's claim of "latest"). |
| `--bearer-token TOK` | no | Sent as `Authorization: Bearer <TOK>` if the node sits behind an auth gateway. |

### Example

```bash
$ evaporchain-light-client sync-latest \
    --node http://localhost:8081 \
    --genesis-height 15190
{
  "genesis_anchor_height": 15190,
  "trust_period_secs": 1209600,
  "trusted_tip_height": 15271,
  "trusted_tip_state_root": "8c8804a1e4f95bda27128888ff730d024800d88e2cf35d84313d001f9233b3b7"
}
```

**What this proves**: every block from 15190 to 15271 was BFT-attested by ≥2/3 stake, with the cert's BLS aggregate sig verified against the validator-set's public keys.

### Exit codes

- `0` — sync succeeded; trusted tip advanced to the chain's reported latest.
- `1` — verification or transport failure. The trusted tip is at the last successfully-verified height. Re-run with `--genesis-height` set to that height to retry from there.

## Subcommand: `get-state`

Fetch + verify a Verkle state-query proof. Two ways to specify what state to query:

| Flag | Purpose |
|---|---|
| `--key HEX` | Raw 64-character hex 32-byte trie key. Use this when you already know the chain-internal trie key. |
| `--account HEX` | 64-character hex 32-byte account address. The CLI derives the trie key as `blake3("acct" \|\| address)` (matches `evaporchain_state::db::trie_key_for_account`). |

`--key` and `--account` are mutually exclusive; one is required.

### Examples

Query an account by address (recommended for most use cases):

```bash
$ evaporchain-light-client get-state \
    --node http://localhost:8081 \
    --account 0400000000000000000000000000000000000000000000000000000000000000
{
  "trusted_tip_height": 15400,
  "trusted_tip_state_root": "...",
  "queried": "account=0400...0000",
  "trie_key": "...",
  "value": "...32-byte-account-hash..."
}
```

Query by raw trie key (when you already know it):

```bash
$ evaporchain-light-client get-state \
    --node http://localhost:8081 \
    --key abc123...32-bytes-of-hex
```

### Strict expected-value mode

```bash
$ evaporchain-light-client get-state \
    --node http://localhost:8081 \
    --account 0400...0000 \
    --expected 0123abc...32-bytes-of-hex
# exits non-zero if the proof's value doesn't match the expected
```

Useful in CI / monitoring scripts that want to alert on state drift.

### Output JSON shape

```json
{
  "trusted_tip_height": <u64>,
  "trusted_tip_state_root": "<hex>",
  "queried": "key=...",       // or "account=..."
  "trie_key": "<hex>",
  "value": "<hex>" | null     // null = non-membership proof
}
```

## Subcommand: `watch`

Follow the chain forward indefinitely. Useful for eyeballing a node's progress without writing a script.

### Flags

| Flag | Required | Purpose |
|---|---|---|
| `--node URL` | yes | Node base URL. |
| `--genesis-height N` | yes | Initial trust anchor. Required (unlike `sync-latest`) — a watch from "latest" reports zero-walk every cycle, defeating the point. |
| `--poll-secs N` | no | Polling cadence in seconds. Default 5. Match this to the chain's block interval (~3-8s typically). |
| `--bearer-token TOK` | no | Auth gateway token. |

### Example

```bash
$ evaporchain-light-client watch \
    --node http://localhost:8081 \
    --genesis-height 15276 \
    --poll-secs 3
watching node http://localhost:8081 from height 15276 (poll every 3s; Ctrl-C to stop)
{"height":15281,"ingested_this_cycle":5,"state_root":"5ec50c41..."}
{"height":15281,"ingested_this_cycle":0,"state_root":"5ec50c41..."}
{"height":15282,"ingested_this_cycle":1,"state_root":"2fc2cdca..."}
{"height":15283,"ingested_this_cycle":1,"state_root":"a1b2c3d4..."}
^C
```

Cancel with Ctrl-C. On per-cycle sync failure (transient network hiccup, node restart, etc.), the trusted tip is preserved at the last good height and the watch continues. Exits non-zero only on Ctrl-C or unrecoverable transport setup error.

## Common error patterns + remedies

| Error | Cause | Remedy |
|---|---|---|
| `transport: resource not found` on `get-state` | Node binary predates `e56359a` (2026-05-07) — `/api/state/proof/:key_hex` not yet wired | Rebuild the node (`cargo build --release --bin evaporchain-node` then restart) |
| `Bft: insufficient signers: N < quorum M` | Node returned a header whose commit cert has too few signers — chain bug or block fetched mid-finalization | Retry; if persistent, file a bug |
| `Bft: BLS aggregate signature verification failed` | Cert's signature doesn't verify against the validator-set's public keys — chain integrity issue, fork attempt, or mismatched validator-set snapshot | Stop using the node; investigate the chain's BLS state |
| `transport: parse error: ...` | Node returned malformed JSON (gateway proxy issue, partial response, etc.) | Check gateway / reverse-proxy in front of the node |
| `Bft: Trust period expired` | More than 14 days elapsed between `LightClient` construction and the latest sync attempt | Re-anchor: drop the LightClient state, fetch a fresh trust anchor, and start over |

## Configurable URL templates (advanced)

The CLI's HTTP transport (`HttpTransport` from `evaporchain-light-client-http`) hard-codes path templates that match the chain's default `/api/...` shape. If your gateway exposes the SDK's required endpoints under different paths, you'll currently need to either:

- Run a thin reverse-proxy that re-paths your gateway's URLs to `/api/light_header/*` and `/api/state/proof/*`, OR
- Build your own `RpcTransport` impl wrapping `HttpTransport` with custom path templates (the `HttpTransport::with_paths` builder exists for this; the CLI just doesn't yet expose it as a flag).

## Why use this CLI over `curl`?

`curl` returns JSON. The CLI **verifies cryptographically** that the JSON came from a chain attested by ≥2/3 validator stake (BFT BLS aggregate-sig), or that a state proof binds to a trusted state root (Pasta-curve Pedersen commitments). A node returning malformed or adversarial JSON will be caught by the SDK; raw `curl` won't notice.

In practice, use both:
- `curl` for ad-hoc probing when you trust the node operationally.
- `evaporchain-light-client` for any decision that depends on chain truthfulness — wallet balance reads, dapp state lookups, bridge attestations, explorer rendering.

## Cross-references

- `crates/evaporchain-light-client/src/client.rs` — SDK core, `LightClient` struct, `ingest_block`, `verify_state`.
- `crates/evaporchain-light-client/src/transport.rs` — abstract `RpcTransport` trait + `TransportError`.
- `crates/evaporchain-light-client-http/src/lib.rs` — concrete `HttpTransport` over `ureq`.
- `crates/evaporchain-light-client-http/tests/e2e_http.rs` — synthetic-server e2e tests (good reference for building a custom transport).
- `crates/evaporchain-node/src/api.rs` — chain-side endpoint handlers (`get_light_header`, `get_state_proof`).
- `INVENTION_STACK.md §4.1 row 8` — Lambda-Fold doctrine the SDK operationalizes.
- `LAMBDA_FOLD_NOVA_PLAN.md` — Phase 5 hot-path integration this CLI consumes.
- `docs/runbooks/disaster-recovery.md` — broader node operations.
