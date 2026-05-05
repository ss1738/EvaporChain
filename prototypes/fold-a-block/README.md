# Fold-a-Block: EvaporChain Go/No-Go Prototype

> **Status: HISTORICAL.** This prototype was the original feasibility gate
> for the Lambda-Fold doctrine, run before the production Nova IVC
> integration. The gate was passed. As of 2026-05-04, real Nova IVC ships
> in `crates/evaporchain-proving::nova` and is wired into the consensus
> hot path via `crates/evaporchain-lambda-fold` with the arity-8
> Poseidon-bound state-root + 5-equation chain-aggregate energy-fold
> gadget. Full plan: [`../../LAMBDA_FOLD_NOVA_PLAN.md`](../../LAMBDA_FOLD_NOVA_PLAN.md).
>
> Empirical numbers from the production path supersede the prototype
> targets below: light-client `verify_with_vk_bytes` runs at **23 ms @
> 100 folds** (1.083× of 23 ms @ 10 folds) on M4 release — sublinear
> claim empirically locked. Prototype retained for historical
> reference and for anyone replicating the original gate decision.

## Purpose

This prototype answers the single most important question for EvaporChain:
**Can we fold blockchain state transitions fast enough for real-time block production?**

## What It Does

1. Defines a simplified EvaporChain block as an R1CS circuit (balance transfers + energy decay + state commitment)
2. Folds 1,000 sequential blocks using Nova IVC
3. Extracts and verifies a compressed SNARK proof
4. Reports detailed timing benchmarks

## Run

```bash
cargo run --release
```

## Targets

| Metric | Target | Hard Fail |
|--------|--------|-----------|
| Avg fold time (100 tx/block) | <10ms | >50ms |
| Proof size (compressed) | <2KB | >10KB |
| Verification time | <10ms | >100ms |

## Verdict

- PASS (<10ms): EvaporChain is feasible as designed
- MARGINAL (10-50ms): Feasible with batching (fold every N blocks)
- FAIL (>50ms): Architecture needs fundamental redesign
