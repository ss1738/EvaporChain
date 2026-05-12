# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Before starting any session

Read exactly these five state-of-chain docs in order. Stop after the top 2-3 entries of journal files (`SESSION_PROGRESS.md`, `CHANGELOG.md`) — full read is not needed.

1. **`SESSION_PROGRESS.md`** — most recent operational state. Newest entry at top. Read FIRST to know where the build is.
2. **`MAINNET_READINESS.md`** — lane-claim board (🟡 OPEN / 🟢 CLAIMED / ✅ DONE). Pick the lane you'll drive in this session here.
3. **`DOCTRINE_PUNCH_LIST.md`** — layered build plan (Layers 0–7), what's already shipped at the doctrine level.
4. **`AUDIT_2026_05_11.md`** — most recent findings (the only audit doc at root; older audits live in `docs/archive/obsolete-audits/`).
5. **`CHANGELOG.md`** — grep for your area; do not read top-to-bottom.

Sprint narrative + doc / dead-crate audit lives in **`MAINNET_SPRINT_PLAN_2026_05_11.md`**. Read it once on your first session; not required thereafter.

Completed plan docs (CROOKS_MEV, LAMBDA_FOLD, LIGHT_CONE, MCC_FULL) live in `docs/archive/completed-plans/`. Older audits in `docs/archive/obsolete-audits/`. Deprecated punch-lists in `docs/archive/deprecated/`. Treat as read-only history.

## Before ending any session

**Append a new entry at the top of `SESSION_PROGRESS.md`** using the template at the head of that file. Required for every session that ships ≥1 commit. The format is:

```
## YYYY-MM-DD (morning|afternoon|evening) — short focus

**Focus:** one sentence
**Commits shipped:** N (first → last)
**Deliverables:** bullet list / table
**Empirical results (if any):** bullets
**Decisions made:** bullets
**What's next:** top 2-3 items for the next session
**Blockers / open questions:** anything needing human judgment
**Cross-references:** CHANGELOG.md / AUDIT / runbooks / specific commits
```

Format consistency over polish. Old entries stay; the file is append-only history.

## Build & test — ALL runs on the M4 Minis via SSH

**Never run `cargo build/test/check` on the MacBook.** Only on the Mini cluster:

```bash
ssh -o IdentitiesOnly=yes -i ~/.ssh/id_ed25519 satyawansingh@100.119.53.101   # Mini 1
ssh -o IdentitiesOnly=yes -i ~/.ssh/id_ed25519 satyawan-mini-1@100.113.253.72  # Mini 2
ssh -o IdentitiesOnly=yes -i ~/.ssh/id_ed25519 satyawan-mini-2@100.103.216.125 # Mini 3
```

```bash
# Full workspace
make build                # cargo build --workspace
make test                 # cargo test --workspace
make test-compile         # compile tests without running (fast regression check)
make lint                 # clippy (no -D warnings; backlog of 1.94 lint hits)
make lint-strict          # clippy -D warnings (post-cleanup target)
make fmt                  # cargo fmt --all
make fmt-check            # dry-run format check
make check                # pre-PR gate: fmt-check + lint + build + test-compile

# Single crate
cargo test -p evaporchain-consensus
cargo test -p evaporchain-consensus -- test_name

# Node
cargo run -p evaporchain-node -- --api --api-port 8080
cargo run -p evaporchain-node -- --api --api-port 8080 --prove   # with Nova IVC

# Coq proofs (on Mini, requires Rocq 9.1.1)
cd research/coq && make clean && make

# EvaporScript pilot
contracts/evaporscript/mortal_message.es   # reference pilot contract
```

**Rust toolchain:** pinned at `1.94.0` in `rust-toolchain.toml`. Do not upgrade without auditing new clippy lints.

## Workspace structure

147 crates, ~1.09M LOC, 25,435+ tests. Two tiers:

**Core stack (18 crates)** — the chain's production hot path:

| Crate | Role |
|---|---|
| `evaporchain-types` | All domain types (`Block`, `Transaction`, `StateObject`, `Account`, `GhostRecord`, 25 tx variants). Energy decay formula lives here. |
| `evaporchain-crypto` | BLAKE3, ML-DSA (Dilithium3 post-quantum), BLS12-381 aggregation, VRF, Verkle trie, Energy-Verkle trie, MMR nullifiers |
| `evaporchain-state` | `StateDB` trait + RocksDB impl; Evaporation Engine (Active→Grace→Ghost); Refresh Engine; WAL crash recovery |
| `evaporchain-contracts` | 8 template contracts + rule engine (triggers/conditions/actions) |
| `evaporchain-script` | EvaporScript: parser → compiler (constant-fold + DCE) → 44-opcode VM with gas metering |
| `evaporchain-execution` | `SimpleExecutor` (sequential) + `BlockStmExecutor` (parallel OCC); PID fee controller; conservation audit gate |
| `evaporchain-consensus` | Tendermint BFT (Propose→Prevote→Precommit→Commit); BLS aggregation; encrypted mempool; epoch manager; Light-Cone DAG substrate; MCC fork-choice; Crooks-MEV pipeline |
| `evaporchain-proving` | Nova IVC recursive proofs (`nova-snark 0.68`); `RealBlockCircuit` arity-8 with energy-fold; Lambda-Fold |
| `evaporchain-network` | libp2p gossipsub; block sync; DA shard sampling; Sybil scoring; chain-id-scoped topics |
| `evaporchain-da` | 2D Reed-Solomon; PoHA; namespaced Merkle tree; BLS DA certificates |
| `evaporchain-consensus-types` | Consensus types extracted for WASM compatibility (no RocksDB dep) |
| `evaporchain-node` | Full node binary: Axum API + dashboard + faucet + persistence |
| `evaporchain-cli` | CLI: genesis ceremony, keygen (BLS+ML-DSA+VRF bundle, EVPL encrypted), validator onboarding |
| `evaporchain-mcp` | MCP server (26 tools, 13 resources, 6 prompts) |
| `evaporchain-oracle` | BFT oracle data feeds |
| `evaporchain-sharding` | Experimental shard assignment + cross-shard messaging |
| `evaporchain-eth-bridge` | Ethereum bridge |
| `evaporchain-fee-controller` | Singh-Lyapunov PID fee controller crate |

**Substrate (~60+ crates in `crates/`)** — doctrine primitives, VM paradigms, launch dApps. Each is independent with its own tests. Full list in `Cargo.toml`.

Key substrate groups:
- **Invention-stack primitives**: `evaporchain-light-cone` (Causal-set DAG), `evaporchain-bell-beacon`, `evaporchain-singh-attractor`, `evaporchain-evap-fork-cert`, `evaporchain-ib-validators`, `evaporchain-causal-chsh` (Tier-0; Bell cartel detection, gate PASS on real Ethereum 2026-05-04), `evaporchain-lambda-fold` (Nova IVC accumulator)
- **App templates pipeline**: `evaporchain-app-templates-{deploy,materialise,engine,bind,fees,receipt,eventlog}` — complete dApp deploy round-trip
- **VM paradigms (Tier-2 substrate triplet)**: `evaporchain-total-evaporscript` (total programming, no infinite loops), `evaporchain-cap-decay-vm` (KeyKOS/seL4 ocap with energy-decay), `evaporchain-dp-native-vm` (differential-privacy-native)
- **Tier-3 specialized**: `evaporchain-epa-mmr`, `evaporchain-thermal-stm`, `evaporchain-plc`, `evaporchain-ew-twap`
- **Launch dApps (SDDC pattern)**: `evaporchain-sddc`, `evaporchain-sfsv`, `evaporchain-shlm`

## Two unifying invariants (enforce in every session)

### 1. Energy routes through `energy_at_epoch` only

**Never** use `>>` on energy values outside `evaporchain-types`. All decay logic — in every crate — must call `evaporchain_types::energy_at_epoch`. The Coq-verified canonical formula lives there. Using raw bit-shifts elsewhere silently breaks the conservation invariant and will be caught by the Layer 0 CI lint.

### 2. New business logic = EvaporScript contract first

New on-chain business logic goes in an EvaporScript contract (`.es`). TypeScript/frontend is a thin view layer only. Reference pilot: `contracts/evaporscript/mortal_message.es`.

## Governance flags

Doctrine-grade behaviors are gated behind governance flags so the cluster stays bit-compatible until explicitly flipped:

| Flag | Default | Doctrine behaviour |
|---|---|---|
| `conservation_enforcement` | `"observe"` | `"enforce"` rejects blocks with energy violations |
| `block_source_mode` | `"fifo"` | `"antichain"` enables antichain mempool drain |
| `parent_acceptance_mode` | `"linear"` | `"mcc"` enables MCC Boltzmann fork-choice |
| `crooks_mev_settlement_mode` | `"observe"` | `"enforce"` settles MEV refunds on-chain |
| `light_cone_state_branches_enabled` | `false` | `true` enables per-fork state materialization |
| `lambda_fold_mode` | `"hash_chain"` | `"nova"` switches to real Nova IVC accumulator |

Flags are set via `POST /api/governance/param` and read via `GET /api/governance/flags`.

## Formal verification

5 Coq proofs in `research/coq/` + `research/proofs/LLSAInvariantPreservation.v`, all zero-Admitted under Rocq 9.1.1. CI `coq` job runs `make` in `research/coq/` on every PR. Do not add `Admitted` — fix the proof instead.

5 TLA+ specs in `research/tla/` (`EvaporChainBFT`, `ConservationInvariant`, `PoHA`, `RuleBasedConsensus`, `EnergyVerkleTrie`). All `.cfg` files have `CHECK_DEADLOCK FALSE` — the bounded model reaches a terminal state at MaxHeight, which TLC flags as deadlock by design.

## Key non-obvious conventions

- **BLS key format**: loaded via `detect_bls_key_format` magic-byte dispatch. EVPL plaintext format: `format_plaintext_for_disk`. BLS keys live in `~/.evaporchain-tailscale-data/bls_key.bin` on cluster nodes; preserve them before wiping data dirs.
- **DA input**: always produce block DA inputs via `build_block_da_inputs(txs)`. Encoding block bytes directly produces a `data_root` that diverges from serving-time roots.
- **Gossipsub topics**: chain-id-scoped. mDNS is off by default (`mdns: false` in config). Cross-testnet topic contamination was the source of the Lane R.* cluster-freeze bug.
- **Sybil scoring**: disconnected peers must not accumulate idle-tick penalties. `record_disconnect` clears the score; `record_connect` fresh-slates it. Check `ghost_count` from `GET /api/network/scores` — `> 0` is the freeze-class early-warning signal.
- **Nova state root**: read 8 bytes from the hash for `u64` conversion (`state_root_to_u64`). Reading 4 bytes silently broke every cluster proof.
- **MERA crate**: retained as research artefact only. Gate ran on real Ethereum (R²=0.66 vs threshold 0.85) → VERKLE verdict locked. Do not treat `evaporchain-mera` as a production crate.

## Doctrine tests

Every doctrine primitive must ship with:
1. Source comment citing `INVENTION_STACK.md §X.Y` and the original theorem
2. An adversarial test (not just a happy-path test)
3. An end-to-end integration test against a non-trivial fixture
