# ADR-004: Block-STM Parallel Execution

**Status:** Accepted  
**Date:** 2026-02-10  
**Deciders:** Satyawan Singh (founder)

---

## Context

Sequential transaction execution caps throughput at the speed of a single CPU core processing the slowest transaction. For EvaporChain's target of 7,000+ sustained TPS and 468,000 peak TPS (measured on benchmarks), sequential execution is a hard bottleneck.

Ethereum's EVM is inherently sequential because contract calls can touch arbitrary state. Solana achieves parallelism by requiring callers to declare all accounts touched by a transaction. The design question is whether EvaporChain should follow Solana's ahead-of-time declaration model or use an optimistic concurrent execution model.

## Decision

Use Block-STM (Software Transactional Memory at the block level) optimistic concurrency: execute transactions in parallel using Rayon thread pools; track reads and writes via MVCC (Multi-Version Concurrency Control); abort and re-execute transactions that encounter a read-write conflict. Fall back to serial execution if conflict rate exceeds threshold.

The production executor is `ParallelExecutor` (`crates/evaporchain-execution/src/parallel.rs`). `BlockStmExecutor` (`block_stm.rs`) is a research implementation used in tests.

## Alternatives considered

| Model | Why not chosen |
|-------|---------------|
| Solana-style declared accounts | Shifts cognitive burden to dApp developers; breaks composability (can't call a contract whose touched accounts aren't known at tx-build time) |
| Sui's object-ownership model | Requires all touched objects to be known and owned; doesn't compose with EvaporChain's energy-decay objects which may be evaporated mid-block |
| Full sequential execution | 7K TPS ceiling; insufficient for target throughput |
| CRDT-based execution | Requires all operations to be commutative; not applicable to arbitrary balance transfers and contract calls |

## Consequences

- Transactions that touch the same account are serialized (conflict → abort → retry) with no semantic change from the programmer's perspective.
- Worst case (fully sequential workload) approaches serial execution speed with overhead.
- The `FinalWrites` collect phase must handle all `Location` variants including `AccountStorageDeposit`, `AccountStorageBytes`, and oracle vote state.
- The `apply_writes` phase is single-threaded; this is the correctness guarantee — all parallel tentative writes are reconciled before the next block begins.
