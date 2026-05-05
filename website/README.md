# EvaporChain Website

The public-facing site for EvaporChain: testnet explorer, faucet, wallet shell, dApp directory, docs portal, and live decay visualisation.

## Stack

- **Next.js 16** (App Router, Turbopack)
- **React 19**, TypeScript
- **Tailwind v4** (`@theme inline`)
- **Three.js** + `@react-three/fiber` + `@react-three/drei` for the energy-decay 3D viz
- **framer-motion** for transitions
- **lucide-react** for icons

## Pages

| Route | Purpose |
|---|---|
| `/` | Landing page with live decay visualisation |
| `/explorer` | Block / tx / account browser against the testnet RPC |
| `/faucet` | Testnet token dispenser |
| `/wallet` | Browser-side wallet UI (talks to the wallet-sdk) |
| `/staking` | Validator list, delegate / undelegate, current rewards |
| `/dao` | Governance proposals, vote, history |
| `/nft`, `/tokens` | Asset views with decay state |
| `/identity` | Identity / namespace browser |
| `/developers` | SDK + EvaporScript quickstarts |
| `/docs` | Embedded docs portal |
| `/whitepaper` | Long-form whitepaper |

## Local development

```bash
cd website
npm install
npm run dev          # Turbopack dev server on :3000
npm run build        # production build
npm run start        # serve a built bundle
npm run lint
```

The site reads the testnet RPC endpoint from `NEXT_PUBLIC_RPC_URL` (defaulting to a public testnet URL when unset). Point it at a local node to develop against your own chain:

```bash
NEXT_PUBLIC_RPC_URL=http://localhost:8080 npm run dev
```

## Deployment

Deploys to Vercel from the project root. No special build flags needed beyond the standard Next.js Vercel preset.

## Related

- Node + JSON-RPC: `crates/evaporchain-node`
- Browser wallet SDK: `wallet-sdk/` (consumed via the workspace)
- dApps embedded as iframes / linked targets — current set in `dapps/`:
  - `singh-pool` — Singh-Lyapunov staking pool
  - `validator-analytics` — per-validator dashboard (uptime, slash history, attestations)
  - `gov-portal` — governance proposals + voting
  - `explorer-light` — minimal block/tx browser using the sublinear light-client verifier
  - `governance` — legacy governance app (kept for back-compat)
  - `nft-marketplace`, `energy-pool`, `mortal-messages` — early-phase reference dApps
  - `explorer` — full-fat explorer (heavier dep tree than `explorer-light`)
