# EvaporChain Validator Analytics

Read-only Next.js 16 dashboard for EvaporChain validators: stake, block-production,
delegations, slashes, finality and peer-set health. The dApp never writes to the
chain; it only consumes the public REST + Prometheus surfaces of an
`evaporchain-node`.

## Run

```sh
# from repo root
cd dapps/validator-analytics
npm install
EVAPORCHAIN_RPC=http://127.0.0.1:8080 npm run dev
# open http://localhost:3000
```

`EVAPORCHAIN_RPC` (default `https://testnet.evaporchain.com`) is the upstream
node. `next.config.ts` rewrites `/api/*` and `/metrics` to it so the browser
makes same-origin requests. All pages auto-refresh every 15 s.

## Pages

| Path | Shows | Source |
|---|---|---|
| `/` | Aggregate stats; top-10 by stake / blocks / uptime; slash leaderboard | `/api/validators`, `/api/network/health`, `/api/finality/gap` |
| `/validators` | Sortable validator table with per-row latency sparkline | `/api/validators`, `/metrics` |
| `/validators/[id]` | Stake breakdown, block-production sparkline, delegation flow, slash history, peer info | `/api/validators`, `/api/validator/:id/delegations`, `/api/network/peers`, `/metrics` |
| `/finality` | Recent commit→finalise gap histogram, unfinalised tail, alert state | `/api/finality/gap`, `/api/network/health` |
| `/network` | Peer count, subnet distribution, banned IPs, inbound rejections | `/api/network/peers`, `/metrics` |

## Metrics

`/metrics` is the Prometheus exposition for the node. Parsed in
`src/lib/promParse.ts` to extract:

- `evap_block_production_seconds_*` — per-validator block-execution histograms,
  labelled `producer="validator-{id}"`. Sparklines plot the cumulative bucket
  counts.
- `evap_peer_score{peer_id="…"}` — per-peer reputation gauge.
- `evap_inbound_rejections_total{reason="…"}` — Sybil counters.
- `evap_active_bans`, `evap_unfinalised_height_count`, `evap_worst_unfinalised_gap_seconds`.

If the node is started with `EVAPORCHAIN_ADMIN_KEY` set, `/metrics` requires
that key in the `Authorization` header — the dashboard degrades to "no metrics"
shells gracefully (rejection counts blank, latency sparklines empty).

## Coverage gaps (documented, not invented)

- **No slash event timeline endpoint.** The chain only exposes
  `total_slashed` per validator on `/api/validators` and the slash *action*
  POST `/api/validators/sanov_slash`. The detail page surfaces the cumulative
  figure and notes the gap.
- **No per-validator Bell-Beacon S-value gauge.** The detail page renders "—".
- **No hot/cold stake gauge.** Hot/cold lives behind POST endpoints
  (`/api/hot_cold_stake/{decay,promote,demote}`); we show effective stake only.
- **No validator-id ↔ peer-id mapping.** The peer view on the detail page is
  a best-effort match by address-prefix and may be empty.

## Visual language

Light/clean, no chart library. Sparklines and bucket bars are inline `<svg>` /
flex divs (≤ 50 LOC each). Tailwind v4 with `evap-*` palette mirroring
`dapps/singh-pool`.
