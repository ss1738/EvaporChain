# EvaporChain Explorer · Light

A *light-client* block explorer. Pulls compact CSLC headers, verifies
tx-inclusion + Verkle state proofs in your browser, never trusts the
indexer.

## What makes this "light"?

- **Compact headers only.** The homepage feed and tx/address detail pages
  pull `/api/light/headers` (≈120 bytes each) instead of full blocks.
  Block bodies are only fetched when you explicitly click "Load full block".
- **Client-side inclusion proofs.** "Verify inclusion" on the tx page
  pulls the Merkle path from `/api/light/tx-proof/:block/:tx_index` and
  recomputes the root in-browser via blake3. The proof root is anchored
  against the trusted compact header's `tx_merkle_root`, so the indexer
  cannot lie to you.
- **Structural Verkle verification.** "Verify balance" pulls the Verkle
  state proof from `/api/light/state-proof/account/:addr` and runs a
  structural check: depth/sibling/path-index consistency, hash
  well-formedness, root commitment binding to `state_root`. **Full
  cryptographic Pedersen-commitment opening lives in the prover crate
  and is not shipped here** — the verifier labels its result honestly.
- **CSLC ε-machine.** `/state-graph` reconstructs a Shalizi-Crutchfield
  ε-machine from a tx-count histogram of recent headers and posts to
  `/api/cslc_reconstruct`. The chain doesn't expose the full transition
  graph yet, so the page renders the state-count summary + a callout for
  the missing endpoint.

## Trust model

| Layer            | Trusted? | Notes                                                                  |
|------------------|----------|------------------------------------------------------------------------|
| Compact headers  | Yes      | The dApp's anchor of trust. Compare across nodes for paranoia.         |
| `tx_merkle_root` | Yes      | Field of the compact header.                                           |
| Tx inclusion     | Verified | Recomputed in-browser via blake3 against `tx_merkle_root`.             |
| Full block body  | Verified | `state_root` is checked against the compact header on load.            |
| Verkle proof     | Partial  | Structural check only; Pedersen opening requires the prover crate.     |
| Address tx feed  | Trusted  | `/api/transactions` is server-filtered; treat as a hint, not gospel.   |

## Routes

- `/` — live compact-header feed + chain tip + trusted-header watermark.
- `/block/[height]` — compact header; "Load full block" reveals the body.
- `/tx/[hash]` — tx-status state machine; "Verify inclusion" runs Merkle.
- `/address/[addr]` — balance, nonce, owned objects, "Verify balance".
- `/state-graph` — CSLC ε-machine summary.

## Run

```bash
cd dapps/explorer-light
npm install            # NOT bundled with this dApp's commit — install yourself
EVAPORCHAIN_RPC=http://localhost:9944 npm run dev
```

Defaults to `https://testnet.evaporchain.com` if `EVAPORCHAIN_RPC` is unset.
The dApp uses Next.js rewrites so `/api/*` is proxied to the upstream node.

## Stack

Next.js 16 (App Router, Turbopack) · React 19 · TS strict · Tailwind v4
· `lucide-react` · `@noble/hashes` (blake3) · `@evaporchain/wallet-sdk`
(workspace).

## Known gaps

- `GET /api/cslc/states` (transition graph) — not exposed yet; the
  `/state-graph` page degrades to a CSSR summary.
- Verkle full crypto verification — requires the prover crate; replaced
  with a clearly-labelled structural check.
- Sharding locator — the address page uses a "first byte mod num_shards"
  heuristic until a richer endpoint exists.
