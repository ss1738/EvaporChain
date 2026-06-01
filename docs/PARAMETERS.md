# EvaporChain — Operational Parameters

**Source of truth:** Rust source constants and `genesis-mainnet.json`. This table consolidates everything an operator, auditor, or integrator needs at a glance. Each row cites file:line so it stays grounded; if the value changes in source, update this table in the same PR.

**Last refreshed:** 2026-06-01.

**See also**:
- [`docs/MAINNET_LAUNCH.md`](MAINNET_LAUNCH.md) §5 — governance-flag mainnet defaults requiring explicit operator decisions before launch (`block_source_mode`, `parent_acceptance_mode`, `crooks_mev_settlement_mode`, `lambda_fold_mode`).
- [`docs/AUDIT_SCOPE.md`](AUDIT_SCOPE.md) — auditor-engagement scope; every parameter here is in audit scope.
- `evaporchain_types::chain_ids` — typed chain-id constants (canonical source for `MAINNET`/`TESTNET`/`DEVNET`); referenced in §6 below.

## 1. Block & consensus

| Parameter | Value | Where | Notes |
|---|---|---|---|
| Block interval (target) | 2000 ms | `genesis-mainnet.json` `chain_params.block_interval_ms` | Tunable via genesis |
| Block gas limit | 500,000 | `genesis-mainnet.json` `chain_params.block_gas_limit` | Tunable via governance, **bounded** to `[10_000, 100_000_000]` — see §8 |
| Max txs per block | 10,000 | `genesis-mainnet.json` `chain_params.max_txs_per_block` | Hard cap |
| Max tx size (bytes) | 1,048,576 (1 MiB) | `genesis-mainnet.json` `chain_params.max_tx_size` | Tx-level cap |
| Max blob size (bytes) | 131,072 (128 KiB) | `crates/evaporchain-execution/src/lib.rs:142` `MAX_BLOB_SIZE` | Per blob inside a tx |
| Grace period (epochs) | 5 | `crates/evaporchain-node/src/main.rs:163` `GRACE_PERIOD` | Storage-rent grace before evaporation |
| Max rounds per height | 10 | `crates/evaporchain-consensus/src/tendermint.rs:65` `MAX_ROUNDS_PER_HEIGHT` | After 10, round resets to 0 |
| Round timeout multiplier | `2^min(round, 6)` | `crates/evaporchain-consensus/src/tendermint.rs` (view-change) | Capped at 64× base |
| Quorum threshold | `signing_stake × 3 ≥ total × 2` | `crates/evaporchain-consensus/src/bridge.rs:79` | Stake-weighted 2/3 |
| Equivocation slash | 10% of stake | `validator_set.slash_equivocation` | Per offence |
| Finality records cap | 10,000 | `crates/evaporchain-consensus/src/finality.rs:126` | LRU-pruned |

## 2. Validator economics

| Parameter | Value | Where |
|---|---|---|
| Min validator stake | 100,000 | `genesis-mainnet.json` `chain_params.min_validator_stake` |
| Unbonding period (epochs) | 100 | `genesis-mainnet.json` `chain_params.unbonding_period` |
| Initial validator count | 4 | `genesis-mainnet.json` `validators` (alpha, beta, gamma, delta) |
| Initial validator stake (each) | 250,000 | `genesis-mainnet.json` `validators[].stake` |

## 3. Tokenomics

| Parameter | Value | Where | Notes |
|---|---|---|---|
| Total supply | 1,000,000,000 | `genesis-mainnet.json` `tokenomics.total_supply` | Sum of `accounts[].balance` matches |
| Block reward (initial) | 100 | `genesis-mainnet.json` `tokenomics.block_reward` | Halves every `reward_half_life` blocks |
| Reward half-life (blocks) | 1,000,000 | `genesis-mainnet.json` `tokenomics.reward_half_life` | Tunable via governance, **bounded** to `[100, u64::MAX]` — see §8 |
| Fee burn rate | 50% | `genesis-mainnet.json` `tokenomics.fee_burn_rate` | Remaining 50% to stakers |
| Staker fee share | 50% | `genesis-mainnet.json` `tokenomics.staker_fee_share` | Of non-burned fees |
| Target staking APY | 5% | `genesis-mainnet.json` `tokenomics.target_staking_apy` | Calibration target |
| Storage rent | 1 / byte / epoch | `crates/evaporchain-types/src/lib.rs:191` `STORAGE_RENT_PER_BYTE_PER_EPOCH` | Enforced once per epoch via `last_rent_epoch` cursor in StateDB. Closes punch-list 6. |
| Min storage deposit | 1,000 | `crates/evaporchain-types/src/lib.rs:192` `MIN_STORAGE_DEPOSIT` | Per object create |

### Initial supply distribution (genesis)

| Account | Balance | % of supply |
|---|---|---|
| Foundation Treasury | 350,000,000 | 35% |
| Ecosystem Development | 200,000,000 | 20% |
| Core Contributors | 150,000,000 | 15% |
| Community Airdrop | 100,000,000 | 10% |
| Validator Operators (×4) | 50,000,000 each | 20% combined |

⚠ **Centralization note (status 2026-06-01):** Foundation Treasury alone holds 35% of supply. The "Foundation passes anything solo" path is now closed in code: governance enforces stake-weighted vote-weight (`min(balance, stake)`), a quorum threshold, parameter range validation against §8 floor bounds, and a timelock between proposal pass and apply. The supply-distribution centralization itself remains an operational concern for genesis ceremony — see [`docs/AUDIT_SCOPE.md`](AUDIT_SCOPE.md) §6.1 and [`docs/THREAT_MODEL.md`](THREAT_MODEL.md) §4.9 (Governance Layer). The Foundation-share % is also revisited by the [tokenomics ceremony](TOKENOMICS.md) Q's before genesis.

## 4. Execution + smart contract limits

| Parameter | Value | Where | Notes |
|---|---|---|---|
| `MAX_CALL_DEPTH` (execution) | 64 | `crates/evaporchain-execution/src/lib.rs:141` | Reentrancy guard |
| `MAX_CALL_DEPTH` (script-internal) | 8 | `crates/evaporchain-script/src/lib.rs:215` | EvaporScript-only |
| `MAX_STACK_DEPTH` | 1,024 | `crates/evaporchain-script/src/vm.rs:34` | VM stack |
| `MAX_LOOP_ITERATIONS` | 100,000 | `crates/evaporchain-script/src/vm.rs:36` | Per-loop cap |
| `MAX_STATE_KEYS` | 10,000 | `crates/evaporchain-script/src/vm.rs:44` | Per-contract storage |
| `MAX_MEMORY_BYTES` | 4,194,304 (4 MiB) | `crates/evaporchain-script/src/vm.rs:54` | Per-VM heap |
| `DEFAULT_GAS_LIMIT` | 10,000,000 | `crates/evaporchain-script/src/vm.rs:51` | Per script invocation |
| `GAS_USER_OP` | 30,000 | `crates/evaporchain-execution/src/lib.rs:132` | UserOp base |
| `GAS_UPGRADE_CONTRACT` | 100,000 | `crates/evaporchain-execution/src/lib.rs:133` | Plus `bytecode_len × 200` |
| `GAS_DEFERRED_SUBMIT` | 75,000 | `crates/evaporchain-execution/src/temporal.rs:55` | Deferred tx |
| `GAS_PER_GUARD` | 5,000 | `crates/evaporchain-execution/src/temporal.rs:57` | Per guard predicate |
| `GAS_SHIELD` | 60,000 | `crates/evaporchain-execution/src/privacy_exec.rs:24` | Privacy: shield |
| `GAS_UNSHIELD` | 80,000 | `crates/evaporchain-execution/src/privacy_exec.rs:25` | Privacy: unshield |
| `GAS_PRIVATE_TRANSFER_BASE` | 100,000 | `privacy_exec.rs:26` | Plus per-input/output |
| `GAS_PRIVATE_TRANSFER_PER_INPUT` | 20,000 | `privacy_exec.rs:27` | |
| `GAS_PRIVATE_TRANSFER_PER_OUTPUT` | 15,000 | `privacy_exec.rs:28` | |
| Note tree depth | 20 | `privacy_exec.rs:21` `NOTE_TREE_DEPTH` | 2²⁰ ≈ 1M notes |

## 5. Mempool

| Parameter | Value | Where |
|---|---|---|
| `MAX_MEMPOOL_SIZE` (txs) | 10,000 | `crates/evaporchain-consensus/src/mempool.rs:6` |
| `MAX_TX_SIZE_BYTES` | 131,072 (128 KiB) | `crates/evaporchain-consensus/src/mempool.rs:9` |
| `MAX_TXS_PER_ACCOUNT` | 64 | `crates/evaporchain-consensus/src/mempool.rs:12` |
| `MAX_MEMPOOL_BYTES` | 268,435,456 (256 MiB) | `crates/evaporchain-consensus/src/mempool.rs` `MAX_MEMPOOL_BYTES` | Enforced at admission — closes punch-list 5 |
| TTL eviction | implemented | `mempool.rs:181-203` | Specifics TODO |

## 6. Identifiers

| Field | Value |
|---|---|
| Mainnet chain_id | `evaporchain-mainnet-1` (`evaporchain_types::chain_ids::MAINNET`) |
| Testnet chain_id | `evaporchain-testnet-1` (`evaporchain_types::chain_ids::TESTNET`) — currently live on the public devnet at `89.167.52.40:8099` |
| Devnet chain_id | `evaporchain-devnet-1` (`evaporchain_types::chain_ids::DEVNET`) — recommended for local development to avoid wallet-token cross-replay with the public testnet |
| Genesis time (current placeholder) | `2026-10-01T00:00:00Z` — **must be replaced before launch**. Per [`docs/AUDIT_SCOPE.md`](AUDIT_SCOPE.md) §10 target is Q4 2026 / Q1 2027. |
| `is_canonical(chain_id)` helper | `evaporchain_types::chain_ids::is_canonical` — returns true iff the supplied id matches one of the three constants above. Useful for early-startup pre-flight ("don't boot with an unrecognised chain-id"). |

## 7. Cryptography

| Primitive | Library | Version | DST / domain |
|---|---|---|---|
| ML-DSA Dilithium3 | `pqc_dilithium` | 0.2.0 | (NIST FIPS 204) |
| BLS12-381 | `blst` | 0.3.16 | `BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_NUL_` (sig)<br/>`BLS_POP_BLS12381G2_XMD:SHA-256_SSWU_RO_POP_` (PoP) |
| BLAKE3 | `blake3` | 1.x (workspace) | per-context keyed derivation |
| Poseidon | bespoke | — | Custom constants — **audit-priority 1** (`docs/AUDIT_SCOPE.md` §3.1; per-constraint R1CS review still pending external auditor) |
| XChaCha20-Poly1305 | `chacha20poly1305` | 0.10.1 | wallet-key encryption only |
| Nova IVC | `nova-snark` | 0.68.0 | HyperKZG over BN254 |
| RocksDB | `rocksdb` | 0.22.0 | 5 column families |
| libp2p | `libp2p` | 0.54.1 | GossipSub, Kademlia, Noise |

## 8. Governance-tunable parameter floor bounds

Every parameter set via the governance pipeline is validated against the
constitutional floor bounds defined in
`crates/evaporchain-execution/src/lib.rs:validate_governance_param`. These
bounds are **immutable except by hard fork** — governance can tighten
them via a `DecayingDAO` contract's `param_bounds`, but never widen them
past these constants.

| Key | Type | Floor bounds | Rationale |
|---|---|---|---|
| `block_gas_limit` | u64 | `[10_000, 100_000_000]` | Halts chain at either extreme |
| `block_reward` | u64 | `[0, 1_000_000_000]` | Prevents single-proposal hyperinflation |
| `reward_half_life` | u64 | `[100, u64::MAX]` | Below 100, inflation collapses to 0 too fast |
| `base_fee_floor` | u64 | `[0, u64::MAX/2]` | Leaves headroom for ceiling |
| `base_fee_ceiling` | u64 | `[1, u64::MAX]` | Zero would never let any tx pay |
| `fee_burn_rate` | f64 | `[0.0, 1.0]` | Ratio; NaN/inf rejected |
| `staker_fee_share` | f64 | `[0.0, 1.0]` | Ratio |
| `target_staking_apy` | f64 | `[0.0, 1.0]` | Ratio |
| `target_gas_utilization` | f64 | `[0.0, 1.0]` | Ratio (used by PID controller) |
| (other keys) | — | pass-through | Forward-compatibility default |

Cross-key invariants enforced by `validate_governance_param_against_state`:

- `base_fee_floor < base_fee_ceiling` strictly. When updating either
  side, the OTHER side as currently set in `db.get_governance_param` is
  read and the strict-less-than relation is checked. If only one side is
  set (the other relies on the executor's compiled-in default), the
  cross-key check is skipped.

Validation fires at four entry points: `execute_governance::CreateProposal`
(submission), `execute_governance::CastVote` (apply at vote pass),
`apply_dao_governance` (the DAO bridge), and `finalize_expired_proposals`.
A defense-in-depth consistency check at `apply_governance_params` (the
state-readback that the executor runs each block) skips the apply if the
floor/ceiling pair is somehow inconsistent.

## 8.5 Governance-flag mainnet defaults

The chain currently ships with **universal** defaults across all chains
(per `tendermint.rs::governance_flags_snapshot`). Four flags require
explicit operator calls before mainnet launch — they are the operator-
decision items in [`MAINNET_LAUNCH.md`](MAINNET_LAUNCH.md) §5:

| Flag | Current default | Mainnet decision options | Notes |
|---|---|---|---|
| `conservation_enforcement` | `enforce` | `enforce` (recommended) / `observe` (legacy) | Default is the mainnet posture; no decision needed. |
| `block_source_mode` | `fifo` | `fifo` (recommended at launch) / `antichain` | `antichain` is closer to the doctrine but isn't the default; both are mainnet-safe. |
| `parent_acceptance_mode` | `linear` | `linear` (recommended at launch) / `mcc` | `mcc` enables Boltzmann fork-choice; flip post-launch via governance once soak data accumulates. |
| `crooks_mev_settlement_mode` | `observe` | `observe` / `enforce` | **Decision required.** `enforce` settles MEV refunds on-chain; observe-mode keeps the chain bit-compatible with pre-flag history but means no actual MEV refunds at launch. |
| `light_cone_state_branches_enabled` | `false` | `false` (recommended at launch) | Keep `false`; flip later via governance after soak. |
| `lambda_fold_mode` | `hash_chain` | `hash_chain` / `nova` | **Decision required.** `nova` switches to real Nova IVC accumulator; operationally heavier; conservative launch is `hash_chain`. |
| `cartel_alarm_mode` | (defaults from CHSH track) | (operator confirms with doctrine team) | |
| `script_vm_mode` | (defaults from script track) | (operator confirms) | |

Flags are governance-mutable post-launch via `POST /api/governance/param` + the stake-quorum amendment process. Per-chain-id default selection is **not** yet implemented (Phase B of the mainnet sprint); current defaults apply uniformly across mainnet/testnet/devnet.

## 9. How to update this table

When you change a constant in source:
1. Update the line citation here.
2. Confirm `genesis-mainnet.json` and `genesis-target.json` still match if the parameter is genesis-tunable.
3. Cross-check against [`docs/AUDIT_SCOPE.md`](AUDIT_SCOPE.md) (priority tier + closure rows in §3-§6) so auditors see the same numbers.
