# Singh Pool — EvaporChain Concentrated-Liquidity AMM

LP positions are state Objects whose energy reserve decays at the position's
half-life. Decayed positions stop earning fees and eventually evaporate
unless the LP refreshes their energy.

This is the differentiator dApp for EvaporChain's substrate primitives — a
standard CL-AMM mechanic, but with energy decay as a first-class part of
liquidity provisioning.

## Substrate primitives exercised

- `GET /api/objects` — list LP positions (each is an EvaporObject).
- `POST /api/tx/refresh` — top up a position's energy reserve.
- `POST /api/patronage/pledge` — add an immunity covenant to a position.
- `GET /api/patronage/status` — read the patronage namespace.
- `GET /api/refresh_pool` — refresh-pool widget on the listing page.
- `GET /api/status` — current epoch for decay forecasts.

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
