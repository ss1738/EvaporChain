# EvaporChain — Session Progress Tracker

Working journal for the build. Each session appends an entry at the TOP. Newest first.

**This is NOT** `CHANGELOG.md` (formal published ship log) or `AUDIT_*.md` (point-in-time audit). This is the operator-level "what we did + what's next + what's blocked" view across sessions.

---

## How to add an entry (read once, then forget)

When you wrap up a session, prepend a new block at the top using this template. Don't bother with prose — bullet-pointed is fine. The discipline is **consistency of format**, not polish.

```markdown
## YYYY-MM-DD (morning|afternoon|evening) — short focus

**Focus:** one sentence

**Commits shipped:** N (first-hash → last-hash). See `CHANGELOG.md` for detailed breakdown.

**Deliverables:**
- bullet
- bullet

**Empirical results (if any):**
- what fired in production
- what was observed

**Decisions made:**
- doctrine call X resolved as Y
- parameter Z changed from A to B with reason

**What's next:**
- top 2-3 items for the next session

**Blockers / open questions:**
- anything that needs human judgment / external info / parallel work

**Cross-references:**
- `CHANGELOG.md` <date>
- `AUDIT_*.md` if a new audit doc landed
- `docs/runbooks/*.md` if a new runbook landed
- specific commits worth highlighting

---
```

The reverse-chronological layout means the most recent session is always at the top. Old entries stay; treat them as historical record. The file grows append-only.

---

## 2026-05-08 (evening) — 8-item bundle: tx-hash fix, eulogy wiring, TOKENOMICS §2.1+§2.2+§2.5, conservation observe-mode fix, MCP hardening

**Focus:** ship a verified-but-undeployed bundle of 8 correctness/observability items. Verify on Mini 1; commit + push; defer cluster deploy to next session pending Hetzner SSH access.

**Commits shipped:** 1 (`a6bc9df`). +383/-49 across 20 files.

**Deliverables:**

| # | Item | Files |
|---|---|---|
| 1 | Demo NFT/HEAT half-life 100 → 1000 | `node/main.rs` |
| 2 | `compute_tx_hash` → `tx.tx_hash()` (canonical signing bytes) — closes "tx vanishes from `/api/tx/<hash>` after ring expiry" | `node/persistence.rs` |
| 3 | Eulogy-trie wiring on every newly-evaporated object (matches §A2.5 "small deaths" doctrine) | `execution/lib.rs` |
| 4 | TOKENOMICS §2.1: `process_block_rewards_v2` 60/40 proposer/attester split, dust to first attester, falls back to v1 when no attesters | `execution/rewards.rs`, `lib.rs`, `parallel.rs` |
| 5 | TOKENOMICS §2.2: `commission_ppm` field on ValidatorInfo (serde-default 100_000 ppm = 10%) | `consensus-types/lib.rs` |
| 6 | TOKENOMICS §2.5: `blocks_per_year` field + `apy_capped_reward` method on Tokenomics; v2 wires the cap. 4 genesis JSONs updated. | `types/genesis.rs`, `genesis-{mainnet,tailscale-3node,tailscale-5node,target}.json` |
| 7 | Conservation §1.2 fix: `minted_this_block` credited into pre-block compartment snapshot before `audit_block_step` so DecayIncreasedTotal stops false-firing on legitimate minting | `execution/lib.rs` |
| 8 | MCP hardening: 3 new validators (`validate_hex_id_field` w/ path-injection guard, tx-hash, block-height); 5 hardened tool handlers; auth default inverted (token present → require auth unless explicitly relaxed) | `mcp/{validation,tools,main}.rs` |

Plus 4 backward-compat fixups for the new struct fields (Tokenomics × 5 literals, ValidatorInfo × 1, Block.post_state_root × 2 in integration tests, dfri-fs MOD_P import).

**Empirical results:**

- `cargo check --workspace` on Mini 1: green.
- `cargo test --workspace --no-fail-fast` on Mini 1: only 4 pre-existing failures remain (`state_sync::test_snapshot_metadata_state_root_mismatch_rejected`, `state_sync::test_tip_discovery`, `state_sync::test_full_sync_flow_with_provider`, `cli::cli_snapshot_create_then_verify`). All 4 are in code untouched by this bundle — regressions from intermediate work between the 2026-05-02 baseline and HEAD. Bundle adds zero new failures.
- Round 2 of the test suite (with my originally-flipped governance defaults) had 11 failures (7 from the flips + 4 pre-existing). Round 3 with the reverted defaults dropped to 4. Confirms the reverts.

**Decisions made:**

- **Doctrine-grade governance flag flips (antichain mempool, Nova IVC, conservation enforce) are NOT ridden in via default change.** The flips happen via `POST /api/governance/param` after a clean stop-the-world deploy, so the binary stays bit-compatible with a running cluster on default settings. Changing the defaults in code would hard-fork any running cluster on the next binary swap — verified via the `governance_flags_snapshot` API surface that returns effective values.
- Demurrage threshold: my session's edit (raise to 100M EVP) was superseded by the better committed work in commit `7bdbfaf` (testnet 250k / mainnet 100M split with goldilocks calibration math). My edit not included in this bundle.
- Conservation enforcement default kept at `"observe"` until in-cluster validation that `minted_this_block` credit fully nullifies DecayIncreasedTotal. Live testnet shows `last_conservation_audit_ok: false` with the violation discriminant unexposed — flipping to `"enforce"` blind would halt the chain.

**Blockers / open questions:**

- **Hetzner SSH access blocks Phase C cluster deploy.** Nodes 100.66.208.20 (`evaporchain-hel-1`) and 100.91.235.22 (`evaporchain-hel-2`) are not reachable for the operator account that has access to the 3 Minis. Stop-the-world is mandatory because `process_block_rewards_v2` changes block-reward distribution semantics — partial deploy would fork the chain. Operator must supply credentials.
- **Cluster heightspread:** at probe time, 3 nodes lockstep at h~12700 (Mini 1 + 2 Hetzners), Mini 2 lagged 296 blocks, Mini 3 lagged 317 blocks (the val-1+val-3 organically tombstoned pair from 2026-05-08 afternoon). They should sync up once block production is steady.
- **Deploy procedure unchanged:** `docs/runbooks/cluster-deploy.md` §3 stop-the-world. After deploy, post-deploy governance flips: `block_source_mode→antichain`, `lambda_fold_mode→nova`, then `conservation_enforcement→enforce` (in that order, with cluster observation between each).
- **Tx-hash forward-only:** old tx receipts in chain_store keyed by JSON-byte hash will be unreachable from new binary lookups (which use canonical hash). Acceptable for testnet; mainnet would want a re-index migration.

**What's next:**

- Get Hetzner SSH credentials → run Phase C stop-the-world deploy on the 5-node cluster.
- Post-deploy: governance-param tx flips for the three doctrine flags (with verification windows between).
- Write tests for `process_block_rewards_v2` 60/40 split (currently exercised only via existing v1 fallback path; v2 split paths uncovered).

**Cross-references:**

- `CHANGELOG.md` 2026-05-08 (evening) — to be appended.
- `docs/runbooks/cluster-deploy.md` — stop-the-world procedure.
- Plan file `~/.claude-account-b/plans/glittery-jumping-cat.md` — verify+deploy strategy.
- Commit `a6bc9df` — the bundle.

---

## 2026-05-08 (afternoon) — death-is-final doctrine + Singh Pool API + decay observability

**Focus:** ratchet the chain's namesake decay thesis from "substrate-shipped" to "empirically operational" across all 5 layers; fully wire Singh Pool AMM HTTP surface.

**Commits shipped:** 19 (`24920e6` → `d906d80`). Full detail in `CHANGELOG.md` 2026-05-08 (afternoon).

**Deliverables:**

| # | Commit | Theme |
|---|---|---|
| 1 | `24920e6` | 6-ratchet death-is-final bundle (DEPLOYED) |
| 2 | `4ec297d` | session-arc audit doc |
| 3 | `0321b50` | pnt 0x-prefix fix |
| 4 | `f8605d7` | light_cone_block_count + MMR docstrings |
| 5 | `a421321` | conservation violation discriminant |
| 6 | `8c79129` | Verkle/DA proof 0x-prefix sweep |
| 7 | `0404d27` | Singh Pool Stage 1 (read endpoints) |
| 8 | `3333dab` | Singh Pool Stage 2 (mutators) |
| 9 | `3b7bc8d` | cluster-deploy runbook |
| 10 | `bc4a956` | CHANGELOG entry |
| 11 | `50a9c40` | Singh Pool Stage 3a (route /api/swap through pools) |
| 12 | `51260a3` | Singh Pool Stage 3b (latent serde_json bug) |
| 13 | `6fa1d61` | bincode fix + 8 helper-fn unit tests |
| 14 | `a23e44a` | top-level README update |
| 15 | `56e9ac1` | Singh Pool smoke-test bash script |
| 16 | `f1bc8c1` | demurrage_preview endpoint |
| 17 | `1cb6677` | empirical correction (demurrage threshold-gated dormant) |
| 18 | `7bdbfaf` | demurrage threshold re-calibration 100M → 250k |
| 19 | `d906d80` | docs/README + dapps/singh-pool/README updates |

**Empirical results on the live 5-node WAN cluster:**

```
Decay-thesis layer       Status (post-deploy of 24920e6)
─────────────────────────────────────────────────
Object Active→Grace→Ghost     fired live (multiple test objects)
HBCT H+1 capacity expiry      fired live (8 positions burned)
Storage-rent → tombstone      fired live (val-1, val-3 organic)
Validator jail-on-tombstone   fired live (R4 ratchet)
Refresh-pool §1.2 absorption  ~155k EVP accrued
Account-balance demurrage     dormant (threshold gating, fixed
                              by 7bdbfaf — fires post-deploy)
```

Cluster reached block 11,000+ during the session under stress (2 jailed validators, 3-of-5 BFT quorum holding).

**Decisions made:**

- **`DemurrageParams::default_genesis()` re-calibrated: threshold 100M → 250k EVP.** Testnet validators sit at 300-600k, far below the original 100M (which was a mainnet-scale calibration assuming 50M+ validator funding). New value: validators above 250k pay ~0.1-1 EVP/epoch (~25× solvency margin vs block rewards). Mainnet calibration preserved as `mainnet_calibration()` constructor for the eventual mainnet genesis.
- **Singh Pool persistence: bincode file at `<data_dir>/singh_pools.bin`.** JSON was attempted (commit `51260a3`) but `serde_json` rejects `HashMap<HolderId, LpShare>` because `HolderId` is `[u8; 32]` and JSON doesn't support array-keyed maps. Caught by the test in `6fa1d61`. RocksDB-backed alternative noted as Stage 4 (Week 1 next session).
- **0x-prefix audit: complete** (6 endpoints fixed across 3 commits). All path-param hex endpoints now consistently accept both `0x`-prefixed and bare hex.
- **`light_cone_block_count` is NOT block height.** Operationally non-monotonic (sliding-window-pruned DAG count). Documented; canonical block height read is `/api/blocks?limit=1`.
- **R3 (dead-producer credit redirect) is doctrine-correct at 0.** R4's jail-on-tombstone preempts R3 in normal flow; R3 is defense-in-depth that fires only on the rare race where proposer-and-tombstoned-validator are the same in the same block.

**What's next (1-month plan from this session's wrap-up):**

Week 1 (mainnet correctness foundations):
- §1.2 conservation doctrine call (fixed-supply emission OR retract — architectural decision)
- Singh Pool Stage 4 (RocksDB persistence + state-root commitment)
- Deploy the 18 follow-up commits to the live cluster (stop-the-world per `cluster-deploy.md`)

Week 2 (observability + DAG):
- Per-tx demurrage receipt full version (Agent 4 Candidate 1 — TxOutcome → receipt store)
- ConcurrentFinality event emission (Agent 4 Candidate 3)

Week 3 (real-world data + flag-flip experiments):
- Real oracle TWAP feeds for `/api/swap`
- HBCT real Elexon BMRS integration
- Governance flag-flip experiments on isolated testnet (`conservation_enforcement: enforce`, `lambda_fold_mode: nova`, `cartel_alarm_mode: alarm`)

Week 4 (mainnet genesis prep + audit engagement):
- Tokenomics ceremony Q&A finalization (§2.1 / §2.2 / §2.5)
- External audit engagement
- Mainnet genesis ceremony rehearsal
- Cluster-health web UI dashboard

**Blockers / open questions:**

- **§1.2 conservation doctrine call** — needs human judgment on direction (fixed-supply emission vs. retract §1.2 wording). Code work depends on this.
- **18 commits accumulated awaiting deploy** — sister or future session needs to roll them via stop-the-world.

**Cross-references:**

- `CHANGELOG.md` 2026-05-08 (afternoon) — full commit-by-commit detail
- `AUDIT_2026_05_08_DECAY_LOOP.md` — pre-bundle audit + empirical correction addendum
- `docs/runbooks/cluster-deploy.md` — stop-the-world deploy procedure
- `scripts/test-singh-pool.sh` — Singh Pool live-cluster smoke test

---

## 2026-05-08 (morning) — Refactor A + Refactor B + cross-backend interop

**Focus:** unblock the WASM Light Client SDK build by extracting `evaporchain-consensus-types` (no RocksDB) and feature-flagging the BLS backend (pure-Rust `bls12_381` for wasm32 vs. native `blst`).

**Commits shipped:** 9. Full detail in `CHANGELOG.md` 2026-05-08 (morning).

**Deliverables:**

- `evaporchain-consensus-types` extracted (Phases 1, 2+4+5, 3a, 3b)
- BLS backend feature-flagged (`bls-native` / `bls-portable`)
- 10 cross-backend interop tests (single-sig + DST + 3-signer aggregate-verify)
- WASM build unblocked: 310 KB `.wasm` + 26 KB ES module + TS declarations

**Cross-references:**

- `CHANGELOG.md` 2026-05-08 (morning)
- `crates/evaporchain-light-client-wasm/README.md`

---

<!-- Future sessions: prepend new entries above this line. -->
