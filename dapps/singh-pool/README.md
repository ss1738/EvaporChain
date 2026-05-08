# Singh Pool — EvaporChain Concentrated-Liquidity AMM

LP positions are state Objects whose energy reserve decays at the position's
half-life. Decayed positions stop earning fees and eventually evaporate
unless the LP refreshes their energy.

This is the differentiator dApp for EvaporChain's substrate primitives — a
standard CL-AMM mechanic, but with energy decay as a first-class part of
liquidity provisioning.

## Now wired to a real AMM (2026-05-08)

The `evaporchain-cl-amm` substrate (`SinghPool` — decay-aware xy=k with
energy-tagged LP shares) is now exposed via the node's HTTP API. The dApp
can drive real swaps + LP positions against on-chain liquidity instead of
the previous simulation-only flow.

**Node-side API (commits `0404d27`, `3333dab`, `50a9c40`, `51260a3`,
`6fa1d61`):**

- `GET /api/pool/list` — every pool's summary
- `GET /api/pool/:id` — full pool state
- `POST /api/pool/create` `{id, fee_bp, energy_floor}` — bootstrap a new pool
- `POST /api/pool/:id/mint` `{holder, amount_x, amount_y, anchor_energy, epoch}` — first-time mint or proportional add
- `POST /api/pool/:id/withdraw` `{holder, shares_to_burn}` — energy-floor-gated burn
- `POST /api/pool/:id/swap_x_for_y` `{amount_in}` — swap X→Y
- `POST /api/pool/:id/swap_y_for_x` `{amount_in}` — swap Y→X
- `POST /api/pool/:id/reanchor` `{holder, anchor_energy, epoch}` — top up energy so the holder can withdraw again

Pool ids follow the alphabetically-sorted pair convention: `"EVAP-FLUX"`
means `X = EVAP, Y = FLUX`. The chain's existing `/api/swap/{quote,execute}`
endpoints **automatically route through a Singh Pool when one exists for
the requested pair**; oracle-priced 1:1 is the fallback. Response includes
`route: "pool" | "oracle"` so the dApp can show users which path was taken.

u128 reserves serialise as decimal strings on the wire (avoids
JS-number precision loss). Pool state persists across node restarts via a
bincode-encoded ledger at `<data_dir>/singh_pools.bin`.

Smoke test against a running node: `scripts/test-singh-pool.sh [URL]`.

## Substrate primitives exercised

- `GET /api/objects` — list LP positions (each is an EvaporObject).
- `POST /api/tx/refresh` — top up a position's energy reserve.
- `POST /api/patronage/pledge` — add an immunity covenant to a position.
- `GET /api/patronage/status` — read the patronage namespace.
- `GET /api/refresh_pool` — refresh-pool widget on the listing page.
- `GET /api/status` — current epoch for decay forecasts.
- `GET /api/account/:addr/demurrage_preview` — show LPs how much demurrage their holder account will burn at next epoch sweep (commit `f1bc8c1`).

## Pages

- `/` — Pool list + create position dialog.
- `/swap` — Swap UI with active-vs-decayed liquidity awareness.
- `/positions/[id]` — Position detail with decay forecast, refresh, patronage.

## Local development

This dApp is a workspace package; deps install via the parent monorepo or
individually:

```bash
cd dapps/singh-pool
npm install
npm run dev
```

`next.config.ts` proxies `/api/*` to `EVAPORCHAIN_RPC` (default
`https://testnet.evaporchain.com`), and aliases `@evaporchain/wallet-sdk`
to the workspace-local source.

To typecheck without running deps install:

```bash
cd dapps/singh-pool
npx tsc --noEmit
```

## Stack notes

The other reference dApps in this repo use Vite. Singh Pool follows the
project's documented frontend default (Next.js 16 App Router) per the user's
global stack defaults. The wallet-sdk is wired the same way and the proxy
behaviour is equivalent to Vite's `server.proxy`.
