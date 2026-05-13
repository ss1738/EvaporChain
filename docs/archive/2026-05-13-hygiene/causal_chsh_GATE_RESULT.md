# EvaporChain Causal-CHSH Empirical Gate — Results

**Run (unix epoch):** 1777888451
**Source:** `research/causal-chsh/honest.csv`
**Concurrency window:** 60 s
**Blocks analysed:** 200
**Total ±1 samples:** 985 (~ 246 per setting-pair)
**Reference:** `crates/evaporchain-causal-chsh/src/lib.rs` doctrine block

---

## Gate Verdict

```
VERDICT: PASS — Causal-CHSH SHIPS
```

| Quantity | Value | Threshold | Pass? |
|---|---|---|---|
| S_honest | 0.0120 | < 1.80 | ✓ |
| S_cartel | 4.0000 | > 2.20 | ✓ |
| gap (S_cartel − S_honest) | 3.9880 | > 0.40 | ✓ |

## Doctrine action

All three thresholds passed. **Causal-CHSH SHIPS** as a Tier-0-supporting row in `INVENTION_STACK.md §A1.3`. The inequality empirically discriminates honest from cartel traffic on real Ethereum blocks under the concurrency-window proxy. EvaporChain's first 100% original frontier primitive has earned its slot.

Next steps:
- Reserve the §A1.3 row + cartel-detector cross-reference
- Lane O.4: consensus integration — wire a `cartel_alarm` that runs the gate on rolling windows + emits an alarm event when S exceeds the cartel_floor
