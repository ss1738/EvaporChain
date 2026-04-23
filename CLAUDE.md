# EvaporChain

Novel L1 blockchain with 4,159+ tests. All 7 development phases complete. Audit-ready.

## Tech Stack
- Rust (Cargo workspace)
- WASM (evaporchain-crypto-wasm)
- Protobuf (network layer)

## Structure
- `core/` — core blockchain logic
- `crates/` — workspace crates:
  - `evaporchain-consensus` — consensus engine (priority)
  - `evaporchain-node` — node implementation
  - `evaporchain-network` — P2P networking
  - `evaporchain-state` — state management
  - `evaporchain-execution` — transaction execution
  - `evaporchain-contracts` — smart contracts
  - `evaporchain-crypto` — cryptography
  - `evaporchain-proving` — ZK proving
  - `evaporchain-da` — data availability
  - `evaporchain-types` — shared types
  - `evaporchain-cli` — CLI tool
  - `evaporchain-mcp` — MCP integration
  - `evaporchain-script` — scripting
- `sdk/` — SDK
- `wallet/` + `wallet-sdk/` — wallet implementations
- `mobile-wallet/` — mobile wallet
- `dapps/` — decentralized apps
- `extension/` — browser extension
- `tests/` — integration tests
- `scripts/` — utility scripts
- `research/` — research docs

## Commands
- Build: `cargo build`
- Test: `cargo test`
- Full test suite: `make test` (check Makefile)
- Clippy: `cargo clippy`
