# EvaporChain Tokenomics

**Status: ~55% complete (up from 30% earlier today).** Structure exists in code, parameters are wired and functional. Three of six §2 items closed in the 2026-05-07 build session — vesting (§2.6), emission schedule (§2.4), and MEV settlement (§2.3 reconciled). Three §2 items remain ceremony-blocked: §2.1 recipient policy, §2.2 commission split, §2.5 staking-APY controller.

**Audience:** Satyawan Singh + future tokenomics advisor. The "ceremony questions" sections are the punch list of decisions you must make before mainnet launch.

**Pairs with:** `genesis-mainnet.json` (current allocation snapshot, now with placeholder vesting schedules), `research/INVENTION_STACK.md` (line 216 explicitly flags "tokenomics ceremony question"), `research/whitepaper.md` (which has zero tokenomics section — gap to close).

**Last updated:** 2026-05-13 — §0.3 doc-drift fix syncing the allocation table with the actual placeholder vesting schedules already present in `genesis-mainnet.json` (the "zero vesting / unshippable" claim was stale).

**Prior update:** 2026-05-07, after the build session that landed: TOKENOMICS.md (`9827ce1`), VestingLock primitive (`b666fe7`), genesis-mainnet vesting placeholders (`bcbb9b0`), EmissionParams dispatch (`fd1b580`), and MEV reconciliation. First end-to-end ML-DSA-signed external transactions on the running 5-node WAN cluster were tonight too (TX hashes `22fc15c...`, `0801743...`, `7c74142...`).

---

## 0. What's actually wired and observable

These are functioning today on `evaporchain-testnet-1` (the running cluster) and `genesis-mainnet.json`:

### 0.1 Genesis tokenomics block (6 parameters)

```jsonc
{
  "total_supply":         1_000_000_000,    // mainnet cap
  "block_reward":         100,              // EVP per block (initial)
  "reward_half_life":     1_000_000,        // blocks until half — DEPRECATED
  "fee_burn_rate":        0.5,              // 50% of fees burned
  "staker_fee_share":     0.5,              // 50% of fees to active stakers
  "target_staking_apy":   0.05              // 5% — NOT WIRED, see §2
}
```

`reward_half_life` is superseded by `crates/evaporchain-execution/src/emission.rs::EmissionParams` (Constant / Halving / LinearDecay shapes). Kept in genesis only for backwards compatibility.

### 0.2 Chain params (operational, not economic)

```jsonc
{
  "chain_id":              "evaporchain-testnet-1",  // running cluster's actual ID
  "block_interval_ms":     2000,                     // testnet; mainnet TBD
  "block_gas_limit":       500_000,
  "max_tx_size":           1_048_576,                // 1 MiB
  "max_txs_per_block":     10_000,
  "min_validator_stake":   100_000,                  // EVP
  "unbonding_period":      100                       // epochs
}
```

### 0.3 Genesis allocation (mainnet, 1B EVP)

Vesting placeholders are applied to 7 of 8 buckets in `genesis-mainnet.json` (cliff in epochs at `block_interval_ms=2000` → 12 months = 15,768,000 epochs). They are **placeholders pending counsel + tokenomics-advisor review** per the `_vesting_placeholder_warning` field in the genesis file itself; they are NOT final legal/economic decisions.

| Bucket | EVP | % | Cliff | Linear release | Source |
|---|---:|---:|---:|---:|---|
| Foundation Treasury (`0xa0...`) | 350,000,000 | 35% | 12 mo | 48 mo | placeholder |
| Ecosystem Development (`0xb0...`) | 200,000,000 | 20% | 6 mo | 24 mo | placeholder |
| Core Contributors (`0xc0...`) | 150,000,000 | 15% | 12 mo | 36 mo | placeholder |
| Community Airdrop (`0xd0...`) | 100,000,000 | 10% | — | — | TGE 100% (industry-standard for airdrops) |
| Validator Alpha (`0x01...`) | 50,000,000 | 5% | 12 mo | 24 mo | placeholder |
| Validator Beta (`0x02...`) | 50,000,000 | 5% | 12 mo | 24 mo | placeholder |
| Validator Gamma (`0x03...`) | 50,000,000 | 5% | 12 mo | 24 mo | placeholder |
| Validator Delta (`0x04...`) | 50,000,000 | 5% | 12 mo | 24 mo | placeholder |
| **Total** | **1,000,000,000** | 100% | | | |

The on-chain primitive enforcing these schedules is `VestingLock` in `crates/evaporchain-types/src/lib.rs:437` (cliff_epoch + linear_release_epochs + total_locked, with `locked_at(epoch)` computing the unreleased balance). Counsel/advisor sign-off on the durations is the remaining gate — see §3 ceremony questions Q14–Q17. The Community Airdrop bucket being unvested is intentional (standard pattern); explicit confirmation needed before launch.

**Mainnet–live mismatch:** genesis-mainnet.json names 4 validators (Alpha/Beta/Gamma/Delta), but the running cluster has 5 validators (V1–V5, including 2 Hetzner Helsinki). Mainnet genesis is stale and must be updated before launch.

### 0.4 Singh-Lyapunov fee controller (well-designed)

`crates/evaporchain-fee-controller/src/params.rs`:

| Param | Default | Meaning |
|---|---:|---|
| `target_energy` | 1,000,000 | Equilibrium energy point; fee minimised here |
| `target_gas` | 30,000,000 | Target gas per block (Ethereum-comparable) |
| `fee_response_ppm` | 125,000 (1/8 in ppm) | Proportional gain — matches EIP-1559 responsiveness |
| `base_fee_floor` | 1,000 | Minimum base fee at/below target energy |
| `chain_lambda` | `default_genesis()` | Global decay rate, single-λ principle |

Governance-tunable at runtime via:
- `get_governance_param("base_fee_floor")` → u64
- `get_governance_param("base_fee_ceiling")` → u64
- `get_governance_param("target_gas_utilization")` → f64 in [0.0, 1.0]
- `get_governance_param("block_gas_limit")` → u64
- `get_governance_param("conservation_enforcement")` → "enforce" | "observe"

This is **the most complete piece of the tokenomics stack.** EIP-1559-class adaptive base fee with a real Lyapunov derivation behind it.

### 0.5 Slashing economics (sophisticated, calibration TBD)

`crates/evaporchain-consensus/src/validator_set.rs`:

| Param | Value | Notes |
|---|---:|---|
| `SLASH_EQUIVOCATION_PCT` | 0.10 (10%) | Stake slashed on double-sign |
| `SLASH_DOWNTIME_PCT` | 0.01 / missed block | Linear in missed-block count |
| Downtime jail threshold | 3 missed blocks | Validator removed from rotation |
| `HEALTH_BONUS_CAP` | 0.20 (20%) | Health-score boost ceiling |
| `HEALTH_DECAY_RATE` | 0.01 / epoch | Health drift to 0 |
| `HEALTH_PER_EVAPORATION` | 0.05 | Health gain per evaporation processed |
| `MAX_HEALTH_SCORE` | 1.0 | Health caps |

`crates/evaporchain-sanov-slashing/src/slash.rs` adds entropy-weighted slashing:
```
slash = stake × KL(observed ‖ honest) / 1000     // capped at stake
```

Sanov windows: equivocation 100 slots, downtime 20 rounds.

`crates/evaporchain-entropic-slashing/src/lib.rs` adds Shannon-entropy variant (lower for deterministic patterns, higher for noisy distributions).

**Slash destination:** `RefreshPool` (namespace `b"SLSH"`), not burn — slashed tokens flow into the chain-maintenance covenant via `RefreshEngine`. Conservation triplet preserves total supply across slashes.

### 0.6 Demurrage / energy decay (the chain's namesake — see §3 q1)

`crates/evaporchain-state/src/decay_curves.rs`:

| Curve | Default | Used by |
|---|---|---|
| `Exponential { half_life }` | half_life: **100 epochs** | Default for accounts/objects |
| `Linear { rate_per_epoch }` | configurable | Optional |
| `Asymptotic { floor }` | configurable | Floor-bounded decay |
| `Stepped` | configurable | Step-function decay |
| `Conditional { grace_epochs }` | configurable | Pauses on activity |
| `Custom` | bytecode VM, MAX_STEPS=256, max 1024 bytes | User-defined |

**100-epoch default half-life is what we observed eating Validator-1's balance** during the 2026-05-07 demo (V1 lost 189,372 EVP in 30 minutes of inactivity). At ~8s blocks, 100 epochs ≈ 13 minutes — **extremely aggressive** for a validator-operator account. Almost certainly mis-calibrated.

**Demurrage redirect destination is unspecified in code.** When energy decays to 0, the EVP appears to be destroyed (not routed to RefreshPool, not credited to validators). Conservation invariant is preserved by treating demurrage as net-burn. **This is a major design question** — see §3 q2.

### 0.7 Gas costs (per tx type)

`crates/evaporchain-execution/src/lib.rs:262–286`:

| Tx Type | Gas |
|---|---:|
| Transfer | 21,000 |
| Delegate | 40,000 |
| Undelegate | 40,000 |
| Validator Stake | 50,000 |
| Create Object | TBD |
| Refresh | TBD |
| Deploy Contract | TBD |
| Call Contract | TBD |
| Deploy Script | TBD |
| Call Script | TBD |

**These mirror Ethereum 1.0** (transfer=21k). Not derived from EvaporChain's own VM cost model. Calibration question — see §3 q9.

**Observed in production**: TX3 from the demo cost 21,000 gas × 1 EVP/gas = 21,000 EVP fee for a 50-EVP transfer. **420× ratio of fee to value moved.** Functional, but the fee/value ratio is testnet-tier; with the Lyapunov controller at floor (no congestion), this is the minimum.

### 0.8 Slashing/MEV destination accounting

`RefreshPool` accrues from:
- All slashes (equivocation, downtime, Sanov, entropic)
- MEV burns (when Crooks-MEV is in `enforce` mode — see §2.3)
- Conservation: drained over time by `RefreshEngine` to fund chain-maintenance covenants

This is a real, genuinely novel piece — most chains burn slash. EvaporChain's RefreshPool/Patronage architecture turns slashing into a treasury for chain hygiene.

---

## 1. What's wired but uncalibrated (the placeholder zone)

These parameters work; their *values* are arbitrary defaults that have not been justified.

### 1.1 Inflation curve

| Component | Current | Mainnet target? |
|---|---|---|
| `block_reward` | 100 EVP/block | **TBD** — at 8s blocks = 394M EVP/year initial = **~40% Y1 inflation** against 1B supply |
| `reward_half_life` | 1,000,000 blocks | **TBD** — at 8s = ~92 years to halve once. Almost certainly miscalibrated |
| Emission shape | NOT chosen | `Constant` / `Halving` / `LinearDecay` — must pick |

### 1.2 Demurrage half-life

| Component | Current | Mainnet target? |
|---|---|---|
| Default account half-life | 100 epochs (~13 min @ 8s blocks) | **TBD** — kills inactive validators in <1h |
| Per-account override | not in genesis | Should genesis allow custom? |
| Demurrage destination | implicit burn | Burn / RefreshPool / Treasury — must decide |

### 1.3 Slashing rates

| Component | Current | Mainnet target? |
|---|---:|---|
| Equivocation slash | 10% | OK as testnet, validate against capex / opportunity cost |
| Downtime slash per block | 1% | Linear scaling = 100% slashed in 100 blocks (~13 min) — too aggressive? |
| Jail threshold | 3 missed blocks | Validator rotation stability question |
| Sanov / Entropic slash | active | Calibration of `/1000` denominator is arbitrary |

### 1.4 Validator economics

| Component | Current | Mainnet target? |
|---|---|---|
| `min_validator_stake` | 100,000 EVP | OK if 1B total = 0.01% min stake |
| Max validator stake | none | Should there be a cap? Concentration risk |
| Self-bond requirement | none | Validators can stake on behalf of others |
| Validator commission | not implemented | **No commission rate anywhere.** Validators take 0% from delegators today |

### 1.5 Delegation economics

| Component | Current | Mainnet target? |
|---|---:|---|
| `unbonding_period` | 100 epochs | At 8s blocks = ~13 min. Cosmos-equivalent is 21 days. **Almost certainly too short** |
| Min delegation amount | none | Spam vector? |
| Delegator/validator reward split | not implemented | See §2.2 |

---

## 2. What's NOT wired (must build before mainnet)

These appear in genesis, comments, or design docs but **the hot path doesn't actually use them**.

### 2.1 Block reward distribution recipient policy (still ceremony-blocked)

**Resolved 2026-05-07** (commit `fd1b580`): the actual minting path is `RewardAccumulator::process_block_rewards` in `crates/evaporchain-execution/src/rewards.rs:107`. It uses `Tokenomics::block_reward(epoch, total_minted)` which now dispatches to either `EmissionParams::block_reward_at` (when `emission: Some`) or the legacy `reward_at_epoch_capped` (when `emission: None`). `crates/evaporchain-execution/src/emission.rs` still hosts the test module; the type definitions moved to `evaporchain-types::emission`. **No more dead code or contradiction.**

Still ceremony-blocked: **recipient policy is hardcoded to proposer-only** at `rewards.rs:111-117` (the producing validator's account is credited with the full block reward). This is one valid choice but it should be a deliberate one. See §3 Q6.

### 2.2 Delegator/validator fee split

`staker_fee_share: 0.5` says 50% of fees go to "stakers." But validators have no commission parameter, no on-chain commission contract, and no rule for splitting that 50% between the validator and their delegators.

Without this:
- Delegators have no economic claim on fees
- Validators implicitly take 100% of the "staker share"
- No reason to delegate over self-staking

Mainnet-blocker.

### 2.3 MEV refund settlement (resolved — fully wired, dormant by default)

**Reconciled 2026-05-07.** Earlier audit reported "DOC MENTIONS, NOT WIRED" — that was a false negative. Deeper audit confirms the CHANGELOG is accurate: the Crooks-MEV refund pipeline is end-to-end consensus-integrated, with **11/12 claims fully shipped + 1 partial** (Phase 4.2 victim opt-out via `mev_refund_eligible: Option<bool>` is wired in serialization, but consumer-side honoring of the opt-out is deferred). Verified sites:

- `crates/evaporchain-mev-detect/src/lib.rs` — 1,392 LOC, `scan_block` + 9 detection tests
- `tendermint.rs:5416` — detector wired into `on_block_committed`
- `tendermint.rs:2550-2585` — `due_refund_txs` producer helper
- `tendermint.rs:2590-2612, 4821-4838` — `validate_block_refunds` proposal-validation hook
- `execution/lib.rs:1231-1273, 2938` — `execute_refund` attacker-debit / victim-credit executor
- `tendermint.rs:2159-2195` — `apply_mev_missing_refund_slashes` (gated by `crooks_mev_missing_refund_slash_enabled`)
- `tendermint.rs:1206, 2598-2600` — `crooks_mev_settlement_mode` governance flag (default `"observe"`)
- `api.rs:16231, 16233` — `/api/mev/observations` (GET) + `/api/mev/dispute` (POST)

**Operationally inert by default.** With `crooks_mev_settlement_mode = "observe"`:
- Detection runs every block → observations buffered → visible via `/api/mev/observations`
- `compute_observation_refund` populates `refund_amount` on each observation
- Producers do NOT include `RefundTx` entries
- `validate_block_refunds` short-circuits with `Ok(())` regardless of RefundTx absence
- `execute_refund` never invoked
- Zero economic effect on-chain

**Flipping to `enforce`** (governance amendment, no code change required) activates:
- Strict validation: blocks rejected if missing required `RefundTx` (per `due_refund_txs`)
- `execute_refund` runs: attacker balance reduced, victim balance credited
- `mev_missing_refund_violations` counter accumulates per validator
- Optional second flag (`crooks_mev_missing_refund_slash_enabled = true`) arms entropic slashing of repeat-offender validators

The mechanism is mainnet-ready. The activation timing is a deliberate ceremony decision — see §3 Q28.

### 2.4 Emission schedule selection (resolved 2026-05-07)

**Wired in commit `fd1b580`.** `Tokenomics.emission: Option<EmissionParams>` extends genesis schema. `Tokenomics::block_reward(epoch, total_minted)` dispatches:
- `Some(params)` → `evaporchain_types::emission::block_reward_at` (Constant / Halving / LinearDecay + max_supply cap)
- `None` → legacy `reward_at_epoch_capped` (current chain behavior)

Backwards-compatible: existing genesis files have no `emission` field, so the legacy path stays in effect. Running cluster sees no behavior change. Regression test `test_block_reward_none_emission_matches_legacy` confirms.

Mainnet must still:
- Pick a shape (Halving like Bitcoin? LinearDecay over 50 years? Constant with eventual termination?)
- Set `max_supply` if shape isn't naturally bounded
- Persist the choice in genesis

### 2.5 `target_staking_apy: 0.05`

Stored in genesis tokenomics, **read by nothing in code.** Pure documentation. Either:
- Wire it as a controller target (adjust block_reward to maintain 5% APY for total bonded stake), OR
- Delete it from genesis to avoid confusion

### 2.6 Vesting / cliff / locked balances (resolved 2026-05-07)

**Wired in commit `b666fe7` + applied to genesis-mainnet.json in commit `bcbb9b0`.** `evaporchain-types::VestingLock { cliff_epoch, linear_release_epochs, total_locked }` attached to `Account` via `Account.vesting: Option<VestingLock>`. `Account.transferable_balance(epoch)` gates 7 outflow execution sites (Transfer, CreateObject, DeployContract, DeployScript, ValidatorStake, Delegate, Shield). Migration safety verified: `evaporchain-state::legacy::deserialize_account_with_legacy_fallback` ensures pre-vesting bincode'd Account records load with `vesting: None`.

genesis-mainnet.json now locks **900M EVP of 1B (90%)** under placeholder schedules — only the 100M Community Airdrop is day-one liquid. Schedules are explicitly placeholder (`_vesting_placeholder_warning` field in JSON header); ceremony review required per Q14-Q17 below.

### 2.7 Mainnet genesis sync

`genesis-mainnet.json` lists 4 validators (Alpha/Beta/Gamma/Delta). The running 5-node cluster runs V1–V5 (3 Macs + 2 Hetzners). Mainnet genesis is at least 2 weeks behind the operational architecture.

Before mainnet:
- Update validator-set in genesis to match the actual launch topology (5? 10? 21? validators)
- Update bootstrap peer list
- Re-derive validator allocations (currently 4 × 5% = 20%; with N validators, this needs rebalancing)

---

## 3. Ceremony question punch list

These are the decisions that must be made and signed off before mainnet. **Each is a first-class economic decision, not an engineering one.** Order is not priority — all are blocking.

### Q1. Demurrage half-life for accounts (the foundational decision)

The chain is named for energy evaporation. **What is the right half-life for an ordinary user's balance?**

Today: 100 epochs (~13 min) — kills V1 in <1h. This is wrong.

Suggested derivation: target a useful "effective lifetime" (e.g., 1 year). At 8s blocks, 1 year = 3,942,000 blocks. So `half_life ≈ 4M blocks` if we want a 1-year half-life. **Decide the target half-life in years.**

### Q2. Demurrage destination

When an account's energy decays, where does the EVP go?

Options:
- **Burn** (deflate supply) — pure decay narrative
- **RefreshPool** (chain-maintenance treasury) — already exists, conservation-preserving
- **Treasury** (Foundation account) — funds development
- **Validators** (split with current proposers) — incentive alignment

Currently: implicit burn. Not documented.

### Q3. Block reward initial value

Today: 100 EVP/block, ~394M/year initial = ~40% Y1 inflation against 1B supply.

This is **5–10× higher than any production chain** (Cosmos: ~7%, Ethereum POS: ~0.5%, Bitcoin Y1 was 50 BTC × 52,560 blocks ÷ 10.5M = 25%). Pick:
- Target Y1 inflation rate (recommend 5–8%)
- Solve for `block_reward` given block_interval_ms

### Q4. Block reward shape

`Constant` / `Halving` / `LinearDecay`. Bitcoin did Halving (every ~4 years). Solana does LinearDecay (15% → 1.5% over 10 years). Choose:
- Shape
- Halving interval (if Halving)
- Decay window (if LinearDecay)
- Floor (if any)

### Q5. Max supply cap

`max_supply: Option<u128>` — set or unbounded?

If Halving: naturally bounded. If LinearDecay: bounded once reward = 0. If Constant: must set explicit cap or accept unbounded. Decide.

### Q6. Block reward recipient policy

Proposer-only? Proposer + attesters? Proposer + attesters + delegators?

Cosmos splits proposer ~5% of block rewards as bonus, rest distributed to all validators by stake-weight, then delegators receive (1 − commission) × share. **Decide the split rule.**

### Q7. Validator commission

Validators currently have no commission parameter — they take 100% of the "staker share" of fees and 100% of block rewards distributed to them.

Decide:
- Default commission rate (5%, 10%, ...)
- Min/max commission
- Whether commission is governance-fixed or validator-set
- Commission update frequency limits (Cosmos: 1× per 24h max, max delta 1%)

### Q8. Delegator/validator reward split

Once commission is decided, this falls out: delegators receive `(1 − commission) × pro_rata_share`. Need to **wire this in `process_block_rewards`** + the fee distribution path.

### Q9. Gas cost calibration

Current gas costs mirror Ethereum 1.0. EvaporChain has different opcodes (EvaporScript VM with 44 opcodes). Fix:
- Benchmark each opcode's actual M4-Mini execution cost
- Set gas costs to 1 gas ≈ 1 ns of compute (Ethereum convention)
- Re-derive `GAS_TRANSFER`, `GAS_DELEGATE`, `GAS_VALIDATOR_STAKE`, `GAS_CREATE_OBJECT`, `GAS_REFRESH`, `GAS_DEPLOY_CONTRACT`, `GAS_CALL_CONTRACT`, `GAS_DEPLOY_SCRIPT`, `GAS_CALL_SCRIPT` from real measurements

### Q10. Slashing rate calibration

Today: 10% equivocation, 1%/missed-block downtime. At default block time of 8s, 100 missed blocks (~13 min offline) = 100% slashed. **Probably too aggressive.**

Decide:
- Equivocation slash (recommend 5–20% — Cosmos uses 5%)
- Downtime: linear vs. step? Cap at what %?
- Sanov / Entropic denominator (currently `/1000` is arbitrary)

### Q11. Unbonding period

Today: 100 epochs (~13 min @ 8s). Cosmos uses 21 days. Polkadot uses 28 days. **Real chains have multi-week unbonding.**

Decide based on:
- Long-range-attack window (longer = safer)
- UX (longer = worse delegator experience)
- Recommend 14–28 days

### Q12. Min validator stake (% of supply)

Today: 100,000 EVP / 1B = 0.01% of supply. Compare:
- Cosmos: ~0.05% min
- Ethereum: 32 ETH / ~120M = 0.000027% min
- Polkadot: ~50 DOT / ~1.5B = 0.0000033% min

Decide based on target validator-set size and Sybil cost.

### Q13. Max validator stake (concentration cap)

Today: none. Top validator could accumulate >50% stake → halt + censor.

Decide: cap at e.g., 5% of total bonded stake? (Polkadot does this with NPoS.)

### Q14. Vesting on Foundation Treasury (350M = 35% of supply)

Today: zero vesting. Day-one liquid. Standard practice:
- 12-month cliff
- 36-month linear release thereafter
- Multisig or DAO control over disbursements

**This is the single biggest unboxed risk in the current allocation.** Without vesting, the token is unsellable at any reputable launchpad / CEX.

### Q15. Vesting on Core Contributors (150M = 15%)

Same problem, smaller magnitude. Standard:
- 12-month cliff (no claim before)
- 24- to 48-month linear vesting

### Q16. Vesting on Ecosystem Development (200M = 20%)

If used for grants/partnerships: need a release schedule tied to milestones, not time. Multisig disbursement gate.

### Q17. Community Airdrop distribution (100M = 10%)

Today: just a single address. Decide:
- Eligibility criteria (testnet participation? Snapshot of which chains?)
- Per-recipient cap
- Claim deadline (unclaimed → returns to Foundation? Burned?)

### Q18. Validator bucket size & N validators

Today: 4 validators × 5% = 20%. Live cluster runs 5 validators. Mainnet might run 21 / 64 / 100.

Decide:
- Mainnet validator-set size N
- Total validator allocation (current 20% must rebalance)
- Per-validator allocation (stake equality? auction?)

### Q19. Treasury control structure

Foundation Treasury at `0xa0...` is currently controlled by whoever signs with that address's key. Pre-mainnet:
- Multisig (e.g., Gnosis Safe equivalent) — NOT implemented in EvaporChain today
- DAO governance — `evaporchain-llsa` for amendments, but treasury-spend is separate
- Time-locked transfers — see Q14

Decide who can spend from the treasury and how.

### Q20. Delegator min stake / max delegations per validator

Today: no min, no max. Decide:
- Min delegation amount (1 EVP? 100 EVP? 0.01% of validator stake?)
- Max delegators per validator (memory bound)

### Q21. Inflation toward target staking ratio

Cosmos varies inflation between 7% and 20% to push staking ratio toward 67% (more staked → lower inflation). EvaporChain has `target_staking_apy: 0.05` in genesis but unwired.

Decide:
- Target staking ratio (% of supply that should be bonded)
- Inflation band (min/max)
- Controller gain (how fast inflation adjusts)
- Or: discard adaptive inflation entirely and use a fixed schedule

### Q22. RefreshPool drawdown rate

The pool accrues from slashes + (eventually) MEV burns. Drained by `RefreshEngine` to fund chain-maintenance covenants. Rate?
- Per-block fixed amount?
- Proportional to pool size?
- Triggered by epoch boundary?

### Q23. Patronage covenant beneficiaries

Who receives RefreshPool drawdowns? Validators only? Validators + delegators? Object-maintenance fees ("garbage collection bounty")? Currently undefined in code (`crates/evaporchain-evp` has the substrate; the disbursement rule does not).

### Q24. Demurrage opt-out for treasury / validator-bond accounts

If demurrage burns balances aggressively (Q1), do treasury/validator-bond accounts get an exemption? Otherwise:
- Foundation Treasury would lose 350M to decay over the half-life
- Validator self-bonds would erode

Decide:
- Exempt-account list (treasury, validators?)
- OR: demurrage applies to everyone, and treasury must "refresh" via active management

### Q25. MEV refund economics

Once Crooks-MEV settlement is wired (§2.3):
- What % of detected MEV is refunded to victims vs. burned vs. validator-rewarded?
- Confidence threshold for refund (currently observe-mode, no enforcement)
- Operator dispute window
- Validator stake deduction rate when validator missed-refund

### Q26. Governance turnaround

Today: `GOVERNANCE_TIMELOCK_EPOCHS: 5` (~40s @ 8s blocks). Cosmos uses 14 days. **Way too short for production.**

Decide turnaround time per param-class:
- Fee controller params: short (1 hour?)
- Validator-set membership: medium (2 weeks?)
- Tokenomics constants: long (3 months?)

### Q27. Chain ID stability

Current: `evaporchain-testnet-1` on the running cluster, `evaporchain-tailscale-5node-1` in the genesis file (mismatch). Mainnet:
- Final chain_id (recommend `evaporchain-1` or `evaporchain-mainnet`)
- Migration path from testnet

### Q28. Crooks-MEV activation timing (added 2026-05-07)

The Crooks-MEV refund pipeline is fully wired (§2.3) but defaults to `crooks_mev_settlement_mode = "observe"` — detection runs, no economic settlement happens. Decide:

- Mainnet launch with `observe`? Safer; lets ecosystem observe detection accuracy under real load before tokens move.
- Or launch with `enforce`? Activates attacker-debit / victim-credit at genesis; user-visible MEV protection from block 1.
- Stake-deduction follow-on (`crooks_mev_missing_refund_slash_enabled`) — enable simultaneously with `enforce` or stage later?
- `crooks_mev_beta_mb` (Phase 2 rate-based pmf scale, default 1000) — calibrate against testnet activation data.

Recommended path: testnet runs `enforce` for ≥30 days first to validate proposer-rejection + balance-movement under load, then mainnet launches with `enforce` from genesis if testnet is clean. Slashing flag stays off until at least Q2 post-mainnet with ≥1 month of clean enforce-mode data.

---

## 4. Mainnet genesis fixes required (engineering, not economic)

Beyond the ceremony questions, these are concrete file edits needed:

1. **Update `genesis-mainnet.json` validator count** from 4 → final mainnet N.
2. **Add per-account `vesting` schema** (after Q14–Q17 decided):
   ```jsonc
   {"address": [...], "balance": 350_000_000, "label": "Foundation Treasury",
    "vesting": {"cliff_epoch": 1_576_800, "linear_release_epochs": 5_256_000}}
   ```
3. **Add tokenomics extensions** for items in §1:
   - `block_reward_distribution`: enum
   - `validator_commission_default`: f64
   - `validator_max_stake_pct`: Option<f64>
   - `demurrage_destination`: enum
   - `demurrage_default_half_life`: u64
   - `emission_schedule`: enum
   - `max_supply`: Option<u128>
4. **Update `chain_params`**:
   - `unbonding_period` to multi-week value (Q11)
   - `block_interval_ms` to mainnet target
   - `min_validator_stake` post-Q12
5. **Replace deprecated `reward_half_life`** with `emission_schedule`.
6. **Add `governance_timelock_per_class`** (Q26 — different timelocks per param sensitivity).

---

## 5. Path to "tokenomics 100%"

In rough order:

| Phase | Scope | Output |
|---|---|---|
| **Phase 0: this doc** ✅ | Document current state, define ceremony questions | `TOKENOMICS.md` (this file) |
| **Phase 1: book a tokenomics advisor** | Q1–Q13 (operational economics) signed off | Advisor report + revised parameter table |
| **Phase 2: legal/launch counsel** | Q14–Q19 (allocation, vesting, treasury) signed off | Vesting schedules + legal opinion on token classification |
| **Phase 3: engineering wiring** | Build §2.1–§2.6 in code | New crates: `evaporchain-vesting`, updates to `evaporchain-execution::process_block_rewards` |
| **Phase 4: mainnet genesis ceremony** | All §3 questions answered, §4 fixes applied, §2 wired | Signed `genesis-mainnet-v2.json` ready for launch |
| **Phase 5: external audit** | Tokenomics audit by 3rd party (Trail of Bits / OpenZeppelin economic review) | Public audit report |

Phases 1–2 are the bottleneck. Phase 3 is ~2–4 weeks of engineering once decisions are locked. Phases 4–5 are ceremonial.

---

## Cross-references

- `genesis-mainnet.json` — current allocation (must be updated per §4)
- `genesis-tailscale-5node.json` — testnet variant running on the 5-node WAN cluster
- `crates/evaporchain-fee-controller/` — Singh-Lyapunov adaptive base fee (the well-designed piece)
- `crates/evaporchain-state/src/decay_curves.rs` — demurrage formulas
- `crates/evaporchain-consensus/src/validator_set.rs` — slashing constants
- `crates/evaporchain-execution/src/emission.rs` — block-reward shape (NOT YET WIRED)
- `crates/evaporchain-execution/src/lib.rs:262–286` — gas costs per tx type
- `crates/evaporchain-sanov-slashing/`, `crates/evaporchain-entropic-slashing/` — entropy-weighted slash variants
- `research/INVENTION_STACK.md` line 216 — original ceremony-question flag
- `research/whitepaper.md` — has zero tokenomics section; this doc is provisional fill-in

---

## Honest assessment

This doc reflects: **the chain is technically real (we've moved tokens cryptographically tonight), but the *economic* design is unfinished.** A naïve mainnet launch with the current parameters would result in ~40% Y1 inflation, validators losing 100% of stake to a 13-minute downtime, demurrage destroying treasury balances, day-one founder liquidity, and a fee controller defaulted to its floor with no upward calibration target.

The wiring is mostly there. The numbers aren't.

This is a normal place for a chain to be 5 months before mainnet. It's not a normal place to be 5 days before mainnet. The deliverable from this doc is forcing those decisions onto a calendar, not pretending they're done.
