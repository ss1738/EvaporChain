# EvaporChain Governance Portal

Public-facing dApp for stake-quorum amendments to the EvaporChain L1.

## Run

```sh
cd dapps/gov-portal
npm install
EVAPORCHAIN_RPC=http://localhost:8545 npm run dev
```

Defaults to `https://testnet.evaporchain.com` if `EVAPORCHAIN_RPC` is unset.
Open http://localhost:3000.

## End-to-end flow

1. **Draft.** `/proposals/new` — pick `fork_choice` or `upgrade_contract`,
   fill in the payload, set a required-stake target. `POST /api/proposals`
   creates an entry in `data/proposals.json` with state `open`.
2. **Endorse.** Validators visit `/proposals/[id]`, paste their hex address
   (looked up against `/api/validators` to snapshot their active stake),
   and click *Sign & endorse*. If a wallet provider is injected at
   `window.evaporchain.signMessage`, the canonical
   `endorsementSignablePayload` is signed and recorded; otherwise the
   endorsement is logged in paste-mode (chain ignores the off-chain
   signature anyway). `PATCH /api/proposals/[id]` with `action:"endorse"`
   appends the row.
3. **Submit.** Once `sum(endorsements.stake) >= required_stake`, the
   *Broadcast* button activates for any visitor. It calls
   - `POST /api/governance/fork_choice_mode` with `{ mode, attractors,
     endorser_stakes, required_stake }`, or
   - `POST /api/tx/upgrade_contract` (governance path: no
     `admin_signature_hex`) with the same `endorser_stakes` /
     `required_stake` fields.

   On success the proposal flips to `active` and the `tx_hash` is stored.
   Failures stay `open` for re-submission.

## Security model

The portal is an **off-chain coordination forum**, not an authority. The
chain re-checks `sum(endorser_stakes) >= required_stake` against its own
validator registry before applying the amendment, and verifies the
BLAKE3 of the new bytecode for upgrade-contract. A malicious portal can
delay or hide proposals but cannot pass an amendment without genuine
on-chain stake. Endorsement signatures are stored for audit replay; the
chain's quorum check does not consume them.

## Files

- `src/lib/types.ts` — `Proposal`, `Endorsement`, signable payload helper.
- `src/lib/proposalStore.ts` — file-backed JSON store with in-process mutex.
- `src/lib/api.ts` — wallet-sdk wrappers + raw `upgrade_contract` POST.
- `src/app/api/proposals/{route,[id]/route}.ts` — REST endpoints.
- `src/app/{page,proposals/new/page,proposals/[id]/page}.tsx` — UI.
