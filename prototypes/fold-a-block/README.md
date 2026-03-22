# Fold-a-Block: EvaporChain Go/No-Go Prototype

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
