# EvaporChain

Novel L1 blockchain with **25,435+ tests** across **147 workspace crates**. Audit-ready (see `AUDIT_2026_05_06.md`); all 7 historical development phases complete + doctrine-layer arc Phases 0–7 substrate-shipped (see `DOCTRINE_PUNCH_LIST.md`).

> Earlier versions of this file said "5,531+ tests" — that number is from an April snapshot and was off by ~4.6× per `AUDIT_2026_05_06.md` headline. Fixed 2026-05-07.

## Tech Stack
- Rust (Cargo workspace, 147 crates)
- WASM (`evaporchain-crypto-wasm`)
- Protobuf (network layer)

## Structure

The crate list below is the load-bearing core; the workspace has many more crates (substrate primitives, V2 hardenings, dApps, etc.). Full inventory at `Cargo.toml` + `DOCTRINE_PUNCH_LIST.md`.

- `crates/` — workspace crates (147 total). Core ones in priority order:
  - `evaporchain-consensus` — consensus engine
  - `evaporchain-node` — node implementation
  - `evaporchain-network` — P2P networking
  - `evaporchain-state` — state management
  - `evaporchain-execution` — transaction execution
  - `evaporchain-contracts` — smart contracts
  - `evaporchain-crypto` — cryptography
  - `evaporchain-proving` — ZK proving (Nova IVC)
  - `evaporchain-da` — data availability
  - `evaporchain-types` — shared types
  - `evaporchain-cli` — CLI tool
  - `evaporchain-mcp` — MCP integration
  - `evaporchain-script` — scripting (EvaporScript VM, 65 opcodes)
  - V2-hardened: `*-v2` variants of bell-beacon, evap-fork-cert, ib-validators, light-cone, singh-attractor, singh-inequality
- `sdk/` — SDK
- `wallet/` + `wallet-sdk/` — wallet implementations
- `mobile-wallet/` — mobile wallet
- `dapps/` — decentralised apps
- `extension/` — browser extension
- `tests/` — integration tests (`evaporchain-integration-tests`, 286+)
- `scripts/` — utility scripts
- `research/` — research docs (`INVENTION_STACK.md`, `whitepaper.md`, Coq + TLA+ proofs, paper drafts)

## Commands
- Build: `cargo build`
- Test: `cargo test`
- Full test suite: `make test` (check Makefile)
- Clippy: `cargo clippy`

## Doctrine + audit pointers

Authoritative state-of-the-chain docs (read these BEFORE starting work):

- `DOCTRINE_PUNCH_LIST.md` — layered build plan, current Layer 0–7 status
- `research/INVENTION_STACK.md` — Tier-0 frontier primitives
- `AUDIT_2026_05_06.md` — most recent end-to-end audit (7/7 CRITICAL closed, 4/4 HIGH closed, 5/5 MEDIUM substrates closed)
- `CHANGELOG.md` — session-by-session ship log
- `IMPOSSIBLE_RESEARCH_STACK.md` — research-track roadmap (Decay-BFT mechanization)
