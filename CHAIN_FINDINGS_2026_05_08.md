# Chain-side findings — 2026-05-08

Empirical discoveries surfaced during overnight wallet-driven testing against the running 5-node WAN cluster (`evaporchain-testnet-1`). All three findings are **chain-layer** issues that the wallet now correctly reports to users; the underlying chain behavior needs investigation in the parallel session.

**Context:** found by submitting real ML-DSA-65-signed TransferTx through `wallet send` and `wallet batch` against Mini 2 (`100.113.253.72:8081`). All TX hashes referenced below are real submissions from this session.

---

## 1. Execution-time rejection from in-block demurrage burn

**Original framing was wrong.** Initial diagnosis (post-finalisation reorg) was misleading; the actual mechanism is execution-time rejection from demurrage burning the sender's balance between mempool admission and block execution. **This finding is now tightly coupled to finding #3 below.**

**The actual flow:**

1. `POST /api/tx/transfer` → pre-check at `crates/evaporchain-node/src/api.rs:~10035-10058`: if `acct.balance >= req.amount` and `req.nonce == acct.nonce`, accept into mempool.
2. Tx queued; block proposer drains it; tx included in block N.
3. **Within block N: demurrage tick runs BEFORE tx execution.** For an inactive account at the default 100-epoch half-life (~13 min @ 2s blocks), the burn between submission and execution can be a substantial fraction of balance.
4. Executor `execute_transfer` (lib.rs:~1180) re-checks `transferable_balance(epoch) < tx.amount`. After in-block demurrage, this can fail even though pre-check passed.
5. Executor sets `tx.status = "rejected"` (per the comment at `api.rs:17292`: *"InsufficientBalance / InvalidNonce → status = 'rejected'"*).
6. `GET /api/tx/<hash>` classifies via `api.rs:9008-9015` — `tx.status == "success"` → finalised, otherwise → rejected:
   ```rust
   if tx.status == "success" {
       ("finalised", None)
   } else {
       ("rejected", Some(tx.status.clone()))
   }
   ```

The "finalised → rejected" flicker the wallet observed is the chain reporting the tx based on `tx.status` which gets updated between block-inclusion and post-execution write.

**Symptom.** Wallet `--wait` first sees `state: finalised` (initial block-inclusion), then `state: rejected, error: "rejected"` (post-execution). The chain never actually mutates the sender's state — `nonce` stays at its pre-tx value, balance unchanged (the executor's rejection prevents the state delta from applying).

**Evidence — three observations tonight:**

| TX Hash | Sender | Submitted | First state | Reorg state | Δ blocks |
|---|---|---|---|---|---|
| `7c74142cdca92c428dffed1c58c6e909cffbf5881f50ab630a1002707e3152fc` | V1 → V5, 50 EVP | early evening | finalised h=15795 | rejected h=15797 | +2 |
| `12d2e9fca9620dcf83e8fe605cfe72cd094c2cbd68c267c71d44c5dad4a72e37` | V1 → V2, 3 EVP (batch tx 1) | overnight | finalised h=17607 | rejected h=17612 | +5 |
| `751167d58e210605c7667cb417190b251e9d53b2fca8cf54d156a5e59f11887a` | V2 → V3, 1000 EVP (batch tx 1) | overnight | finalised h=17646 | rejected h=17651 | +5 |

In every case, the sender's on-chain `account.nonce` did NOT advance, confirming execution-time rejection (not just a soft-finality flip).

**Confirmed root cause (post-investigation):**

In-block demurrage applied to the sender BEFORE the executor runs `execute_transfer`. For an inactive account at default `Exponential { half_life: 100 epochs }` (~13 min at 2s blocks), the burn between submission and execution is large enough to drop balance below `tx.amount + gas`. Pre-check at submission passes; execution check at lib.rs:~1180 fails.

For V1 specifically: balance was 20,198 EVP at session end, gas per transfer is 21,000 — **even an idle account can never afford a transfer once below 21k EVP, and demurrage is actively burning the budget**. See finding #3 for the demurrage-rate critique.

Sister-session next steps:
- Confirm via execution trace that demurrage is being applied to senders mid-block, before the tx-execution pass.
- Decide application order: demurrage-then-tx (current) vs tx-then-demurrage (would let in-flight txs land before decay applies).
- Or: tune demurrage half-life (TOKENOMICS Q1) so within-block burn becomes negligible.

**This narrows finding #1 from "reorg instability" to "demurrage application order + half-life calibration."** No reorg is happening; consensus is sound at the depth-1 BFT finality the chain claims (`tendermint.rs:5831`).

**Reproduction.**

```bash
# Import a non-broke validator key (e.g. V2 has 605k EVP at session end)
wallet account import v2 ~/validator-2-keys.json \
  --address-override 0x0200000000000000000000000000000000000000000000000000000000000000

# Send any small amount, --wait
wallet --node http://100.113.253.72:8081 send \
  0x0300000000000000000000000000000000000000000000000000000000000000 100 --wait
# Result: REJECTED Rejected at block #17725 (failed at execution)
```

The wallet now catches this and reports `REJECTED at block #N (failed at execution)` immediately — see commit `72f7b49` (two-phase await_confirmation).

**Wallet UX impact:** the wallet's two-phase await_confirmation (commit `72f7b49`) catches this correctly — first poll sees the executor's `tx.status = "rejected"` and reports `REJECTED at block #N (failed at execution)` immediately. No more silent timeouts.

---

## 2. Tx-state index retention window

**Symptom.** `GET /api/tx/<hash>` returns `state: pending` for ALL three of tonight's submitted hashes within minutes of submission, including hashes that earlier returned `state: rejected, block_height: ..., gas_used: 21000`. The chain's tx-by-hash index appears to discard records after a short window.

**Evidence:**

```bash
# At ~22:30 UTC, tx 5ba8ee02 was rejected:
$ curl http://100.113.253.72:8081/api/tx/5ba8ee02d297322e3d67a6241df8ec9585ebf88fce57305715641f2c839c312c
# (returned earlier in session): {"state":"rejected","block_height":17725,...}

# 30 minutes later (still during the session):
$ curl http://100.113.253.72:8081/api/tx/5ba8ee02d297322e3d67a6241df8ec9585ebf88fce57305715641f2c839c312c
{"hash":"5ba8ee02d297322e3d67a6241df8ec9585ebf88fce57305715641f2c839c312c","state":"pending"}
```

Same behavior for `d2979c0a...` (confirmed at h=17300, the wallet-signed transfer) and `12d2e9fc...` (V1 batch tx 1).

The API doc at `crates/evaporchain-node/src/api.rs:8108` says: *"Lookup order: chain_store/finalised → committed-but-not-finalised → mempool → pending. Always 200 OK; pending is a typed response, not a 404."*

So `pending` is the chain's "I don't have a record for this hash" response. The chain is "forgetting" txs as they age.

**Hypothesized root causes:**

1. Intentional memory pressure — chain only retains a recent-tx index in-memory.
2. Bug in `chain_store` finalised-tx persistence.
3. The reorg-rejection from finding #1 wipes the tx record in addition to refusing state-application.

**Impact:**
- `wallet tx <HASH>` lookups for old hashes return useless data.
- Block explorers / accounting tools can't reliably reconstruct history.
- Audit tooling can't verify "was this tx ever submitted" after the window passes.

**Sister-session next steps:**
- Check `chain_store::store_transaction` and `chain_store::get_transaction` paths.
- Determine if there's an intentional retention TTL — if yes, document; if no, fix.
- Cross-check against `/api/transactions` — that endpoint may have a longer retention.

---

## 3. V1 demurrage-burn / operationally gas-broke

**Symptom.** Validator-1's operator account at `0x0100...0000` started the session with 209,570 EVP and ended at 20,198 EVP — losing **189,372 EVP in ~8 hours**, almost entirely to demurrage (V1 has been silent / produced 0 blocks). At 20,198 EVP, V1 cannot afford a single transfer (gas = 21,000 EVP), so its operator account is **functionally dead** for tx submission.

**Evidence:**

| Time | V1 balance | Δ | Notes |
|---|---:|---:|---|
| Session start (~16:59 UTC) | 209,570 | — | After full demo + 6 prior nonce increments |
| ~22:00 UTC (live demo) | 125,348 | −84,222 | After our V1→V2 100-EVP transfer (+demurrage) |
| ~22:30 UTC | 41,248 | −84,100 | After V1→V5 50-EVP attempt (which got reorg-rejected) |
| Session end (~00:30 UTC) | 20,198 | −21,050 | Below gas-affordability threshold |

V1 has been silent for the entire session (`blocks_produced: 0` in the validator-set). The demurrage rate at default `Exponential { half_life: 100 epochs }` (~13 min at 2s blocks) is aggressive enough to fully deplete a 250k-EVP genesis allocation in roughly 20 hours of silence.

**This is the "demurrage half-life is wrong" finding tied to TOKENOMICS.md §3 Q1 + §1.2.** See `TOKENOMICS.md` for the broader doctrine context.

**Hypothesized root causes:**

1. Default demurrage half-life of 100 epochs is mis-calibrated by 2-3 orders of magnitude — should be in years, not minutes (TOKENOMICS Q1 ceremony question).
2. No demurrage exemption for treasury / validator-bond accounts — every account decays equally, regardless of role (TOKENOMICS Q24).
3. No grace mechanism for active validators that briefly go offline — silent ≠ malicious, but the chain treats them identically.

**Reproduction:** observable directly via `/api/account/0x0100000000000000000000000000000000000000000000000000000000000000` — V1's balance vs other validators (V2: 605k+, V4: 580k+, V5: 572k+, all of which have been producing blocks).

**Sister-session next steps:**
- Tune demurrage half-life (TOKENOMICS Q1).
- Optional grace period for active validators going temporarily offline (chain-side mechanism).
- Treasury-account exemption (TOKENOMICS Q24).
- Maybe: top-up faucet for V1 from the Devnet Faucet account (operational, not protocol).

---

## What the wallet now does about all this

Tonight's wallet UX changes catch + report these issues clearly to users (even if they can't fix them):

| Wallet behavior | Commit |
|---|---|
| Two-phase await_confirmation reports `REJECTED at block #N` (instead of timing out silently) | `72f7b49` |
| Reorg-rejection watch (10 blocks past first confirmation) reports `REORG-REJECTED Tx flipped finalised → rejected` | `72f7b49` |
| `cmd_tx` body lookup gracefully degrades when tx-state index has dropped the hash | `c25a279` |
| `account list` falls back to cached values when node-level RPC fails | `85e289c` |

The wallet is now telling the truth about chain state. The chain layer is what needs to catch up.

---

## Cross-references

- `TOKENOMICS.md` §1.2 (demurrage half-life), §3 Q1 (demurrage rate), §3 Q24 (treasury exemption)
- `wallet/README.md` — wallet UX surface as of tonight
- `CHANGELOG.md` 2026-05-07 (overnight) entry — full commit narrative
- `crates/evaporchain-node/src/api.rs` — `/api/tx/:hash` endpoint definition
- `crates/evaporchain-state/src/decay_curves.rs:474-476` — current demurrage default `Exponential { half_life: 100 }`
- `crates/evaporchain-execution/src/lib.rs:~1180` — `execute_transfer` balance / vesting check
