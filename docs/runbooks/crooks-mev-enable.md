# Crooks-MEV Refund — Enable Runbook

Operator procedure for moving the chain through the three Crooks-MEV settlement stages: **observe** (default, no behavioural change) → **enforce** (validators must include refunds) → **enforce + slashing** (validators that omit refunds get slashed via entropic-slashing).

**Pairs with:** `CROOKS_MEV_INTEGRATION_PLAN.md` (35/35 task boxes shipped). **Doctrine:** `INVENTION_STACK.md §A1.3` Crooks-MEV Refund row. **Plan:** see `DOCTRINE_PUNCH_LIST.md` Layer 6.

---

## Stage 0 — Default (no action)

The chain ships with `crooks_mev_settlement_mode = observe` (default) and `crooks_mev_missing_refund_slash_enabled = false` (default). At this stage:

- The detector at `tendermint.rs::on_block_committed` runs and populates the per-validator ring buffer of `MevObservation`s (cap 1024).
- Refund estimates are computed per observation and travel with each entry through the buffer's lifetime.
- **No `RefundTx` is required, no validator is slashed, no balance moves.** The chain is bit-identical to a pre-Crooks build except for the observation log.

If you want to keep the chain in this mode indefinitely (purely observational), there is nothing to do. The runbook below applies only when moving to enforcement.

---

## Stage 1 — Flip to `enforce` mode

**Pre-flight checklist:**

- [ ] All validators on a binary at or after commit `bcdcc10` (Phase 4–7 fully shipped).
- [ ] `GET /api/mev/observations` returns a non-empty buffer on at least one validator (proves detection is live).
- [ ] At least 24 hours of observation runtime has accumulated, so operators have a feel for the false-positive rate at the current confidence threshold (default 500 milli = 0.5).
- [ ] Coordinated upgrade window communicated — every validator must flip the flag at roughly the same height, otherwise honest validators that flipped early will reject blocks from validators that haven't.

**Procedure:**

1. Confirm current settlement mode on each validator:

   ```bash
   curl -s http://VALIDATOR_HOST:8080/api/governance/flags | jq '.crooks_mev_settlement_mode'
   # expected: "observe" (or unset → observe via default)
   ```

2. On a coordinator validator, propose the governance change. The exact governance-call shape depends on your deployment's governance facility; via the `governance_set_param` path:

   ```bash
   # Submit a governance proposal flipping the flag.
   # In code: TendermintConsensus::governance_set_param("crooks_mev_settlement_mode", "enforce")
   # Allowlist values: ["observe", "enforce"]
   ```

3. Wait for the proposal to pass (stake-weighted vote ≥ 2/3 honest). Confirm:

   ```bash
   curl -s http://VALIDATOR_HOST:8080/api/governance/flags | jq '.crooks_mev_settlement_mode'
   # expected: "enforce"
   ```

4. Watch the next 100 blocks closely. In `enforce` mode:
   - Validators that propose a block **without** a required `RefundTx` will have their proposal rejected by other validators (Phase 3.4: `validate_block_refunds` returns `MissingRefund`).
   - Validators that propose a block **with** an incorrect `RefundTx` (wrong amount / wrong attacker / wrong victim / wrong source-block ref) will have their proposal rejected (`MismatchedRefund`).
   - Validators that propose a `RefundTx` for an unobserved sandwich will have their proposal rejected (`UnexpectedRefund`).

5. Monitor block production rate. A small dip is expected during the first epoch as validators learn to include refunds; a sustained dip indicates the detector is producing false positives at a rate the chain can't tolerate. Roll back via the rollback procedure below.

**Validation tests for this stage:**

- `tendermint.rs::test_validate_block_refunds_observe_vs_enforce` — green
- `tendermint.rs::test_crooks_mev_end_to_end_consensus_pipeline` — green (covers detection → refund computation → digest convergence → enforce-mode rejection of empty proposal → settled_refunds replay → operator dispute)

---

## Stage 2 — Enable proposer slashing for missing refunds

**Pre-flight checklist:**

- [ ] Stage 1 has been live for at least 7 days with no operator complaints about false-positive rejections.
- [ ] `mev_missing_refund_violations` counter on any validator shows the violation rate is what you'd expect from genuinely-malicious proposers (not honest validators tripping over false positives).
- [ ] Coordinated upgrade window communicated — slashing is a one-way action; if you flip the flag and find a bug, slashed stake doesn't come back without a hard fork.

**Procedure:**

1. Confirm current slashing flag:

   ```bash
   curl -s http://VALIDATOR_HOST:8080/api/governance/flags | jq '.crooks_mev_missing_refund_slash_enabled'
   # expected: "false" (or unset → false via default)
   ```

2. Inspect the violation counter so you know what you're about to act on:

   ```bash
   # In code: tendermint.rs::TendermintConsensus::mev_missing_refund_violations()
   # Returns &HashMap<ValidatorId, u64> — count of MissingRefund rejections per proposer.
   # Surface via your operator dashboard or a custom RPC.
   ```

3. Propose the governance change:

   ```bash
   # In code: TendermintConsensus::governance_set_param("crooks_mev_missing_refund_slash_enabled", "true")
   # Allowlist values: ["true", "false"]
   ```

4. Once passed, validators run `apply_mev_missing_refund_slashes` per applicable bookkeeping. The slash amount per validator is computed via `evaporchain_entropic_slashing::entropic_slash(stake, &[count, 1])` — large-deviation magnitude — and applied via `validator_set::slash_with_amount(id, amount, jail=false)`. Slashing **does not jail** — MissingRefund is operator-policy violation, not equivocation.

5. After the first slashing event, audit the slashed validator's recent block production via `/api/mev/observations` and the chain's block index to confirm the slash was warranted. If a false positive slipped through, file a dispute via the operator-dispute endpoint:

   ```bash
   curl -s -X POST http://VALIDATOR_HOST:8080/api/mev/dispute \
     -H 'Content-Type: application/json' \
     -d '{"source_block_height": <H>, "source_observation_idx": <I>, "current_height": <NOW>}'
   # 200 OK — dispute recorded; observation removed from due_refund_txs scan
   # 404 — observation not found
   # 409 — past grace period (dispute window has closed)
   ```

   Disputes only work if the current height is within the grace window of the source observation. After grace, settlement is final and the only recourse is governance.

**Validation tests for this stage:**

- `evaporchain-consensus` 3 tests covering Phase 3.5d:
  - `test_apply_mev_missing_refund_slashes_no_op_when_flag_off`
  - `test_apply_mev_missing_refund_slashes_applies_with_counter_reset`
  - `test_apply_mev_missing_refund_slashes_unknown_validator_graceful`

---

## Operator endpoints reference

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/mev/observations` | Read-only view of the per-block sandwich detector ring buffer. Returns `{count, observations: [{block_height, attacker, victim, target, work_estimate, confidence_score, refund_amount, …}]}` with hex-encoded addresses. |
| `POST` | `/api/mev/dispute` | Operator-dispute endpoint. Body: `{source_block_height, source_observation_idx, current_height}`. Removes the observation from refund-due scan when invoked within the grace window. |
| `GET` | `/api/governance/flags` | Snapshot of all governance flags. The two relevant keys are `crooks_mev_settlement_mode` and `crooks_mev_missing_refund_slash_enabled`. |

---

## Rollback

If at any stage the chain misbehaves (sustained block production dip, observed false positives at high rate, slashing of clearly-honest validators), roll back by reverting the governance flag(s):

```bash
# Stage 1 rollback: enforce → observe
# In code: TendermintConsensus::governance_set_param("crooks_mev_settlement_mode", "observe")

# Stage 2 rollback: slashing on → off
# In code: TendermintConsensus::governance_set_param("crooks_mev_missing_refund_slash_enabled", "false")
```

Rollback is **per-flag**: you can disable slashing while leaving enforce-mode on, or revert to observe-only. Already-slashed stake is not restored automatically; that requires a separate governance action with a corrective transfer.

---

## Cross-references

- `CROOKS_MEV_INTEGRATION_PLAN.md` — full implementation history, Phase-by-Phase
- `INVENTION_STACK.md §A1.3` — Crooks-Singh fluctuation-theorem framing of the refund formula
- `crates/evaporchain-mev-detect/src/lib.rs` — detector implementation (sandwich-shape scan, observation buffer, due_refund_txs)
- `crates/evaporchain-crooks-mev-refund/src/refund.rs` — `compute_refund(work_extracted, delta_f_milli)` substrate
- `crates/evaporchain-consensus/src/tendermint.rs` — governance allowlist (search `crooks_mev_`), `apply_mev_missing_refund_slashes`, `validate_block_refunds`
- `docs/THREAT_MODEL.md` §4.x — MEV attack-surface analysis
- Companion runbook: `docs/runbooks/disaster-recovery.md` for general validator failure modes
