# evaporchain-light-client-cli

Operator-facing CLI binary for the [`evaporchain-light-client`](../evaporchain-light-client) SDK. Two roles in one:

1. **Operator tool** — quickly probe a node's chain truthfulness from the command line without writing code.
2. **Worked-example reference** — the binary's source (`src/main.rs`) is a complete, copyable example of wiring the SDK + HTTP transport together. Read it when building your own integration (mobile wallet, dapp, bridge, explorer).

## Build

```bash
cargo build --release --bin evaporchain-light-client
```

Output: `target/release/evaporchain-light-client` — single ~10 MB static-linked Rust binary.

## Subcommands

```text
evaporchain-light-client sync-latest --node URL [--genesis-height N]
evaporchain-light-client get-state   --node URL (--key HEX | --account HEX) [--expected HEX]
evaporchain-light-client watch       --node URL --genesis-height N [--poll-secs N]
```

| Subcommand | Purpose | Verifies |
|---|---|---|
| `sync-latest` | Walk the chain forward from a trust anchor to the chain's tip | BFT BLS aggregate-sig per block |
| `get-state`   | Fetch + verify a state-query proof | Verkle Pasta-curve Pedersen commitments |
| `watch`       | Follow the chain forward indefinitely | BFT BLS per ingested block |

For full flag reference, examples, exit codes, and troubleshooting, see [`docs/runbooks/light-client-cli.md`](../../docs/runbooks/light-client-cli.md).

## Quick example

```bash
# Walk the chain from a trust anchor up to the tip:
evaporchain-light-client sync-latest \
    --node http://localhost:8081 \
    --genesis-height 15190

# Verify an account's state at the chain's latest:
evaporchain-light-client get-state \
    --node http://localhost:8081 \
    --account 04000...0000
```

All three subcommands print structured JSON on success. Exit code 0 on success, non-zero on verification or transport failure.

## Source as reference

The binary is intentionally small (~450 lines, all in `src/main.rs`) and well-commented. If you're building a downstream integration, `src/main.rs` is the canonical worked example for:

- Constructing an `HttpTransport` (with optional bearer-token + custom path templates).
- Anchoring a `LightClient` at a trusted height.
- Walking forward via `sync_to_height` / `sync_to_latest`.
- Verifying a state-query proof via `fetch_and_verify_state`.
- Account-address → trie-key derivation (`blake3("acct" || addr)` — matches `evaporchain_state::db::trie_key_for_account`).

## Cross-references

- [`evaporchain-light-client`](../evaporchain-light-client) — SDK core.
- [`evaporchain-light-client-http`](../evaporchain-light-client-http) — HTTP transport.
- [`docs/runbooks/light-client-cli.md`](../../docs/runbooks/light-client-cli.md) — full operator runbook.
