# Multi-Token Gas — Verification Strategy

**Status:** verification artifact (NOT a build commitment).
**Companion to:** `docs/MULTI_TOKEN_GAS_OPTIONS.md` (the decision doc).
**Last updated:** 2026-05-08.

This document answers the question *"once multi-token gas is built, how do we verify the chain is actually accepting it correctly?"*

The answer is **not** "spin it up and try a swap." Real verification is layered: each layer covers failure modes the previous layer can't see. A feature this consensus-adjacent needs all 6 layers green before it ships to operators.

The verification model below targets **Option B (paymaster pattern)** from the options doc. Option A (status quo) needs no verification beyond what already exists. Option C (protocol-level multi-token) is "never build" — no verification plan written.

---

## 0. What "verified" means

A multi-token gas feature is **verified** when:

1. Every code path (tx envelope, paymaster signature, gas deduction, custody) has unit-test coverage.
2. Round-trip integration tests show user-pays-in-X → paymaster-covers-EVP → user-state-changes-correctly across the same in-process executor.
3. A single live node accepts and processes a paymaster-sponsored tx end-to-end.
4. The 5-node WAN cluster reaches consensus on paymaster-sponsored blocks (no state divergence).
5. Adversarial scenarios (paymaster down, bad signature, replay, slippage, race) all fail gracefully — not catastrophically.
6. Production observability surfaces enough state for operators to diagnose paymaster issues without reading logs.

**Until all 6 are green, the feature is NOT ready for mainnet.** Testnet operators can use it earlier with explicit "experimental" labels, but mainnet ships only after verification is complete.

---

## 1. Verification layers

### Layer 1 — Unit tests (in-crate, fast)

Where: `crates/evaporchain-execution/src/`, `crates/evaporchain-types/src/`, `crates/evaporchain-paymaster/src/`.

What to test:

| Test | What it validates |
|---|---|
| `tx_with_paymaster_field_deserialises` | Tx envelope adds `paymaster: Option<AccountAddress>` correctly; backwards-compatible (`#[serde(default)]`); old tx still parse. |
| `paymaster_signature_verifies` | BLS / ML-DSA signature over the canonical tx bytes accepts a valid paymaster sig and rejects a forgery. |
| `gas_deducted_from_paymaster_when_present` | When `tx.paymaster = Some(p)`, gas is deducted from `p.balance`, not from `tx.from.balance`. |
| `gas_deducted_from_sender_when_paymaster_absent` | Backwards-compat: `tx.paymaster = None` → existing behaviour, gas from sender. |
| `paymaster_insufficient_balance_rejects_at_admission` | Paymaster account balance < required gas → tx rejected at admission, no chain mutation. |
| `paymaster_signature_mismatch_rejects_at_validity` | Paymaster sig over WRONG tx bytes → consensus-layer validity check rejects the block. |
| `paymaster_nonce_handling` | Paymaster's nonce increments correctly per tx; replay-protected. |
| `paymaster_field_does_not_affect_tx_hash_canonicalisation` | The `paymaster` field is part of the canonical tx hash; changing it changes the hash. (Caught the same class of bug as `4ec297d` audit doc's hash regression test.) |

Acceptance: every test passes on `cargo test -p evaporchain-execution -p evaporchain-types -p evaporchain-paymaster`.

Effort: ~6-8 tests, ~1 day.

### Layer 2 — Integration tests (in-process, full executor)

Where: `crates/evaporchain-execution/src/integration_tests/paymaster.rs` (new).

What to test:

| Test | What it validates |
|---|---|
| `full_paymaster_round_trip` | Construct `ApiState` (in-memory DB, fixture). Submit tx with paymaster sig. Run a block. Assert: user's state changed (transfer / object create), paymaster balance dropped by gas amount, sender balance unchanged. |
| `paymaster_payment_collection_separate_tx` | Paymaster's compensation flow is a SEPARATE Transfer tx from the user → paymaster in token X. Verify both txs land in the same block atomically (or not, depending on design choice). |
| `multiple_paymasters_compete` | Two paymasters can sponsor different txs in the same block. Their balances update independently. No mutual interference. |
| `paymaster_with_singh_pool_swap` | Composability check: tx that triggers `/api/swap/execute` (Singh-Pool-routed) sponsored by a paymaster. Both gas accounting and AMM math correct. |
| `paymaster_with_object_lifecycle` | Composability: tx that creates an object (storage_deposit + gas) sponsored by a paymaster. Object created with correct owner, gas paid by paymaster. |
| `conservation_invariant_holds_under_paymaster` | §1.2 audit: pre-block compartment sum vs post-block. Paymaster shouldn't violate the invariant — it just shifts who pays. |

Acceptance: every test passes; conservation audit (`evaluate_conservation_gate`) returns Ok in observe mode.

Effort: ~6 tests, ~2 days.

### Layer 3 — Single-node live smoke (running node, real HTTP)

Where: `scripts/test-paymaster.sh` (new). Mirrors `scripts/test-singh-pool.sh` style.

Steps the script runs:

1. **Setup**:
   - register/login, get auth token
   - create paymaster account via standard transfer (fund it from the faucet or a genesis-allocated address)
   - confirm paymaster has EVP balance via `/api/account/<paymaster>`

2. **Sponsored transfer**:
   - construct `Transaction::Transfer` with `paymaster: <paymaster-addr>`
   - get paymaster service to countersign (paymaster service either runs locally or is mocked for the smoke)
   - submit via `/api/tx/transfer`
   - assert HTTP 200 + tx_hash returned

3. **Verify state**:
   - poll `/api/blocks?limit=5` for the tx hash
   - probe sender balance: should be unchanged minus the transfer amount
   - probe paymaster balance: should be down by gas
   - probe receiver balance: up by transfer amount

4. **Sponsored swap**:
   - same flow but tx is `/api/swap/execute`
   - verify Singh Pool reserves updated, paymaster paid gas

5. **Sponsored object create**:
   - same flow but `/api/tx/create-object`
   - verify object visible at `/api/object/<id>`, paymaster paid gas

6. **Edge cases (intentional failures)**:
   - paymaster sig over wrong tx → expect HTTP 4xx
   - paymaster's balance insufficient → expect "insufficient" error
   - missing paymaster sig (with paymaster field set) → expect rejection

Acceptance: script exits 0 with all PASS. PASS/FAIL output mirrors `smoke-identity-endpoints.sh` / `test-singh-pool.sh`.

Effort: ~1 day.

### Layer 4 — Multi-node BFT smoke (5-node WAN cluster)

Where: live cluster (M1, M2, M3, H1, H2) per `docs/runbooks/cluster-deploy.md`.

Steps:

1. **Pre-deploy state snapshot**: probe all 5 nodes for current block height + state_root. Save baseline.

2. **Deploy paymaster-feature commit** to all 5 nodes via stop-the-world per the runbook. Verify all 5 come back up at the same height.

3. **Spawn paymaster service** on the operator workstation (or a 6th observer node). Fund its account from the faucet.

4. **Run the layer-3 smoke script** against each of the 5 cluster nodes in turn. All should respond identically.

5. **Probe state convergence**: after each sponsored tx, all 5 nodes should agree on the new state_root within 3 blocks. Use `scripts/cluster-dashboard.py` or a manual loop.

6. **Validator-rotation soak**: keep submitting sponsored txs through ~50 blocks. Verify:
   - All 5 nodes' eulogy_count, ghost_object_count, refresh_pool_total stay synchronised within 2 blocks.
   - No node falls into observe-only / out-of-sync mode.
   - dead_producer_redirect_total stays 0 (no new validator tombstones from gas-handling bugs).

Acceptance: 5/5 nodes agree on state_root after every sponsored tx. Cluster does NOT fork. No state divergence between paymaster-sponsored blocks and non-sponsored blocks.

Effort: ~2 days (1 day deploy + 1 day soak observation).

### Layer 5 — Adversarial / chaos scenarios

Where: dedicated test harness OR manual scenario list. Document each as a pass/fail.

| Scenario | Expected outcome |
|---|---|
| Paymaster service crashes mid-tx (after countersigning, before tx submission) | User gets a clear "paymaster unavailable" error from wallet. No funds lost. Tx never reaches the chain. |
| Paymaster signs but bills user the wrong rate (e.g., 10× expected USDC) | Wallet's quote-vs-final check rejects the inflated bill. User can ALWAYS see the final amount before confirming. |
| Paymaster's EVP balance drains to zero mid-soak (~50 sponsored txs) | New tx attempts return clear "paymaster insufficient balance" error. User can switch to a different paymaster or pay in EVP directly. |
| Two paymasters race to sponsor the same UserOp | First-in-block-mempool wins; second's signature is invalid (different bundler context). No double-spend. |
| Paymaster signature is replayed with a different tx body | Hash mismatch at validity check. Block rejected. |
| Paymaster signs at block N; tx lands in block N+10 (delayed) | Still valid as long as paymaster nonce hasn't advanced. Implements a 100-block expiry for paymaster sigs to prevent indefinite-future replays. |
| User pays paymaster's bill (token X transfer) but paymaster signature fails to land | Token X is in paymaster's wallet; user is out the X. Mitigation: user-paymaster bill is sent ATOMIC with the tx — both land or neither. (This is a design choice; document it.) |
| Singh Pool swap sponsored by paymaster, AMM rejects mid-block (slippage) | Paymaster still paid gas (block was valid). User did NOT get the swap. Standard tx-failure semantics. Wallet should warn user before submitting. |
| Conservation audit fires a `RedirectChangedTotal` violation on a paymaster block | This is the canary — paymaster code shouldn't violate §1.2. If this fires, halt deploy. |
| Reorg crosses a paymaster block | Paymaster's gas debit must be unwound on rollback. Standard reorg handling, but verify with explicit test. |

Acceptance: every scenario produces the expected outcome. Document each fail mode in operator runbook.

Effort: ~3 days.

### Layer 6 — Production observability

Without these, operators are flying blind on paymaster health. Required before mainnet.

| Endpoint | Purpose |
|---|---|
| `GET /api/paymaster/list` | Every active paymaster account: address, EVP balance, txs-sponsored-so-far, last activity. |
| `GET /api/paymaster/:addr` | Per-paymaster metrics: balance, sponsored count, sponsored txs by token type, error rate. |
| `GET /api/paymaster/:addr/audit_log` | Last 100 sponsorship events: tx_hash, timestamp, amount, payment-collection status. |
| `four_act` augmentation | New field: `paymaster_gas_total` — total EVP redirected from sender accounts to paymaster accounts via the sponsorship path. Companion to `dead_producer_redirect_total`. |
| Health alerts | Paymaster balance < 1 hour worth of sponsorship → warn. < 10 minutes → alert. Implement via prometheus-style metrics or push to operator's monitoring. |
| Logging | Every sponsored tx logs: paymaster address, sender, gas amount, payment-token, payment-amount, success/fail. Single line, parseable. |

Acceptance: `/api/paymaster/list` returns a non-empty (or explicitly-empty-ack'd) list on a running cluster. Operator can answer "is my paymaster healthy?" in under 30 seconds via the API alone.

Effort: ~2 days.

---

## 2. Per-option verification effort summary

For Option B (paymaster pattern, the recommended one):

| Layer | Effort | Cumulative |
|---|---|---|
| 1. Unit tests | 1 day | 1 day |
| 2. Integration tests | 2 days | 3 days |
| 3. Single-node smoke | 1 day | 4 days |
| 4. Multi-node BFT smoke | 2 days | 6 days |
| 5. Adversarial scenarios | 3 days | 9 days |
| 6. Production observability | 2 days | 11 days |

**Total verification effort: ~11 days = ~2.5 weeks** (in addition to the ~5-day build effort from the OPTIONS doc).

So the full V1.5 paymaster ship from start to mainnet-ready is ~3.5 weeks, not 1 week. The 1-week estimate in the OPTIONS doc is **build only**; verification is the extra 2.5 weeks.

This is normal for consensus-adjacent features. Worth knowing up front.

---

## 3. Reusable verification pattern (from this session's experience)

The 2026-05-08 afternoon arc shipped 22 commits. The verification pattern that worked:

1. **Unit tests catch silent failures**. Commit `6fa1d61` shipped 8 unit tests for the Singh Pool persistence helpers and **caught a real bug** that would have made every persistent-pool restart silently lose state. Tests > review for round-trip-shape bugs.

2. **Live-cluster empirical re-run is the gold check**. After each consensus-affecting deploy, re-run the empirical decay test (`scripts/test-singh-pool.sh` post-deploy). If the decay engine still works on the new binary, the feature is genuinely shipped — not just compiled.

3. **Documentation-quality verification matters as much as code-quality**. Commit `f8605d7` corrected the `light_cone_block_count` docstring after we hit it as a misleading liveness signal during the deploy chaos. **Docs that lie are worse than missing docs.** Verify your docstrings against actual behaviour.

4. **Empirical correction is part of verification**. Commit `1cb6677` documented the demurrage-threshold-dormancy issue we found AFTER claiming "decay-thesis empirically validated at 5 layers." The audit doc now has a correction addendum. **Verification doesn't end with the first green test run** — it's a posture, not a milestone.

For multi-token gas, apply this pattern:
- After Layer 3 passes, expect to find at least one "wait this isn't actually doing what I thought" issue in Layer 4. Plan for it.
- Document the correction in the audit doc, just like 1cb6677.
- Don't ship to mainnet until Layer 5's adversarial scenarios are FULLY documented in the runbook.

---

## 4. Empirical reproduction recipes

Concrete commands an operator runs to verify each layer is actually green on a running node.

### Layer 1+2 (in-process)

```bash
# On a Mini (per CLAUDE.md no-builds-on-MacBook rule):
ssh satyawan-mini-1@100.113.253.72 \
  "cd /Users/satyawan-mini-1/EvaporChain && cargo test -p evaporchain-execution paymaster"
```

Expected: all tests pass, similar output style to the 8 Singh-Pool helper tests in commit `6fa1d61`.

### Layer 3 (single-node smoke)

```bash
# Locally, against a running node:
./scripts/test-paymaster.sh http://localhost:8081
```

Expected output mirrors `test-singh-pool.sh`: PASS lines per step, exits 0.

### Layer 4 (multi-node BFT smoke)

```bash
# Probe all 5 nodes after each sponsored tx:
for n in 100.119.53.101 100.113.253.72 100.103.216.125 100.66.208.20 100.91.235.22; do
  curl -s --max-time 5 "http://$n:8081/api/blocks?limit=1" | python3 -c "
import sys, json
b = json.load(sys.stdin)[0]
print(f'$n  blk={b[\"number\"]} sr={b[\"state_root\"][:16]}')"
done
```

Expected: all 5 nodes agree on the latest committed `state_root`. Mismatch = fork; halt deploy.

### Layer 5 (adversarial)

Each scenario runs as a separate manual script + observation. Document outcomes in the runbook (`docs/runbooks/cluster-deploy.md` adds a "Multi-token gas adversarial" appendix).

### Layer 6 (observability probe)

```bash
curl -s "http://100.113.253.72:8081/api/paymaster/list" | python3 -m json.tool
```

Expected: list of active paymasters with balance, txs-sponsored, last-activity. If empty, reads `{paymasters: [], count: 0}` (not an error).

---

## 5. Acceptance criteria — when can this ship?

```
✓ Layer 1: every unit test passes (≥ 6 tests)
✓ Layer 2: every integration test passes (≥ 6 tests, conservation audit OK)
✓ Layer 3: scripts/test-paymaster.sh exits 0 against a single node
✓ Layer 4: 5-node cluster reaches state_root agreement on every sponsored block
            for ≥ 50 blocks of soak (no divergence)
✓ Layer 5: every adversarial scenario produces expected (graceful) outcome
            — documented in runbook
✓ Layer 6: /api/paymaster/list responds with valid envelope on all 5 nodes
            — health alerts wired
```

Mainnet-ready when all 6 are green AND there's been ≥ 1 week of testnet soak with real (or simulated) sponsored-tx volume on the 5-node cluster.

**Anything less is not ready.** Skipping Layer 4 or 5 is the path to a fork in production.

---

## 6. References

### EvaporChain code

- `crates/evaporchain-execution/src/lib.rs:580+` — gas constants (current EVP-only model)
- `crates/evaporchain-execution/src/parallel.rs:2086+` — gas deduction in execute_block
- `crates/evaporchain-execution/src/rewards.rs` — reward distribution (companion to gas deduction)
- `scripts/test-singh-pool.sh` — reference pattern for layer-3 smoke
- `scripts/smoke-identity-endpoints.sh` — older reference pattern, same style
- `docs/runbooks/cluster-deploy.md` — stop-the-world deploy procedure (used for layer-4)

### Companion documents

- `docs/MULTI_TOKEN_GAS_OPTIONS.md` — research + decision doc; Options A/B/C
- `SESSION_PROGRESS.md` — current build state + what's next
- `AUDIT_2026_05_08_DECAY_LOOP.md` — audit-doc-with-correction-addendum pattern; do the same for paymaster

### External research on testing consensus features

- ERC-4337 reference implementations (multiple) — known test vectors for paymaster operations.
- StarkNet's account-abstraction test suite — closest spec-conformance harness.
- Cosmos SDK's `x/auth` paymaster equivalent test patterns (fee-grant module).
- Foundry / Hardhat test patterns for paymaster contracts on Ethereum L1.

---

## 7. What this document does NOT cover

- **Performance benchmarks** (paymaster throughput, latency overhead). Those go in a separate perf doc when the build lands.
- **Centralisation analysis** (single-paymaster trust assumptions). Lives in a governance / decentralisation doc.
- **Fee-pricing policy** (how paymasters set exchange rates). Lives in the paymaster service's own README.
- **UX flows** (wallet design, error messages). Lives in wallet-side product docs.

This doc is purely: how do we mechanically verify the chain accepts multi-token-gas-sponsored txs correctly? Six layers, ~11 days of verification work in addition to the build. Acceptance criteria are non-negotiable for mainnet.

---

## 8. Trigger for re-reading this doc

- ⚪ Option B build starts (next session that picks up the V1.5 paymaster work)
- ⚪ A test layer fails unexpectedly during build — re-read the relevant section + the "reusable pattern" notes from §3
- ⚪ Mainnet ceremony approaches — verify all 6 layers green before genesis

Until then, this doc sits next to `MULTI_TOKEN_GAS_OPTIONS.md` as the verification half of the multi-token-gas decision package.
