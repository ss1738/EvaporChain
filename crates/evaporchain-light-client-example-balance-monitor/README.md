# evaporchain-light-client-example-balance-monitor

Worked-example binary demonstrating the [`evaporchain-light-client`](../evaporchain-light-client) SDK end-to-end. Polls a single account's verified state on a fixed cadence and prints a JSON line every time the value changes.

**Not for production.** This is integrator copy-paste material — pick it apart, lift the pieces you need.

## What it shows

- Anchoring a `LightClient` at chain latest (or at a specific genesis height).
- Walking the trust anchor forward each poll cycle (`sync_to_latest`).
- Verifying a state-query proof against the trusted tip (`fetch_and_verify_state`).
- Account-address → trie-key derivation: `blake3("acct" || addr)` (matches the chain's `evaporchain_state::db::trie_key_for_account`).
- Suppressing redundant unchanged events — only prints on first observation or value change.

## Run

```bash
cargo run --release --bin evaporchain-balance-monitor -- \
    --node http://localhost:8081 \
    --account 0400000000000000000000000000000000000000000000000000000000000000 \
    --poll-secs 5
```

## Output

stderr (status):
```
monitoring account 0400...0000 on http://localhost:8081 (poll every 5s; Ctrl-C to stop)
anchored at height 15400 state_root 8c8804a1...
```

stdout (one JSON line per change):
```json
{"height":15400,"state_root":"8c88...","account":"0400...","trie_key":"...","value":"..."}
{"height":15428,"state_root":"2fc2...","account":"0400...","trie_key":"...","value":"..."}
```

Pipe stdout into `jq`, a log aggregator, a webhook poster — whatever your integration needs.

## Limitations (by design — don't fix in this example)

- **No persistence.** Restart re-anchors at chain latest, defeating the trust period's purpose. Real consumers persist the trusted tip across restarts.
- **No trust-period re-anchoring.** Run more than 14 days against the same anchor and you'll start hitting trust-period-expired errors.
- **No retries on transport failure** beyond logging + sleeping for one poll interval.
- **No nova feature.** This example covers the BFT + Verkle path only — sufficient for ≥99% of wallet/dapp use cases. Add `nova` if you need sublinear validity.

These are the things a real integrator must wire — keeping them out of the example keeps the SDK calls explicit and copyable.

## Cross-references

- [`evaporchain-light-client`](../evaporchain-light-client) — SDK core.
- [`evaporchain-light-client-cli`](../evaporchain-light-client-cli) — fuller CLI tool with three subcommands.
- [`docs/runbooks/light-client-cli.md`](../../docs/runbooks/light-client-cli.md) — operator runbook.
