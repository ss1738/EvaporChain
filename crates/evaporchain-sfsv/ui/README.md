# SFSV UI — thin view layer

Single-file dApp UI for the SFSV reference. Closes gap **#4** from `research/SFSV_ARCHITECTURE.md` §10.2.

## What it is

`index.html` — vanilla HTML + CSS + JavaScript. Two real buttons (Lock and Reclaim) plus a live state panel that polls `/api/contract/:addr/state` every 3 seconds. No framework, no build step, no `npm install`.

Per spec: *"Single page, no framework BS, two buttons: lock and reclaim."*

## Run it

```bash
# Option 1 — directly from the filesystem (works in most browsers,
# unless your browser blocks fetch:file: for cross-origin .es loads)
open crates/evaporchain-sfsv/ui/index.html

# Option 2 — serve via a static server from the workspace root
# (avoids fetch:file: restrictions; preferred)
python3 -m http.server 8080
# then open http://localhost:8080/crates/evaporchain-sfsv/ui/

# Option 3 — alongside a local devnet
./scripts/launch-devnet.sh &
python3 -m http.server 8080 &
open http://localhost:8080/crates/evaporchain-sfsv/ui/
```

## What it does

1. **Connection** — node URL, optional auth token, caller address. Probe button verifies reachability via `/api/version`.
2. **Lock** — fetches the `.es` source from `contracts/evaporscript/future_self_vault.es`, deploys it via `/api/tx/deploy-script`, polls `/api/contract/by-deploy/:hash` to resolve the contract address, then calls `set_terms(...)` via `/api/tx/call-script` to seal the vault.
3. **Reclaim** — calls `try_payout()` via `/api/tx/call-script` against the deployed contract.
4. **Live state** — polls `/api/contract/:addr/state` every 3 seconds; shows epoch, contract energy, `predicate_satisfied`, `released`, holder.
5. **Activity log** — every API call appended with latency.

Connection fields persist across reloads via `localStorage` under `sfsv_ui_v1`.

## Pairs with

- **Contract source-of-truth:** `../../../contracts/evaporscript/future_self_vault.es`
- **Architecture spec:** `../../../research/SFSV_ARCHITECTURE.md` (especially §10.2 gap #4)
- **Deploy script equivalent:** `../../../scripts/deploy-sfsv.sh` (same flow, scripted)
- **Substrate-crate README:** `../README.md`

## Forkability

To adapt this UI to a different decay-dApp:

1. Update the `fetch("../../../contracts/evaporscript/future_self_vault.es")` path to your `.es` source.
2. Rename the Lock form fields to match your domain (future-self → recipient / metadata / whatever).
3. Update the `set_terms` args array to match your contract's signature.
4. Keep everything else verbatim — the state-polling layer, the activity log, the persistence, the layout.

Total fork delta: ~30 lines.

## Browser compatibility

Modern browser with `fetch`, `async`/`await`, `localStorage`. Tested mental model: Safari 17+, Chrome 110+, Firefox 120+. No polyfills, no IE.
