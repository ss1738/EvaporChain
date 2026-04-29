# Architecture Decision Records (ADRs)

Short records of significant architectural decisions, including context, alternatives, and consequences.

| ADR | Title | Status |
|-----|-------|--------|
| [ADR-001](ADR-001-consensus-algorithm.md) | Tendermint BFT as Consensus Algorithm | Accepted |
| [ADR-002](ADR-002-state-evaporation-model.md) | Energy-Decay State Evaporation Model | Accepted |
| [ADR-003](ADR-003-post-quantum-signatures.md) | ML-DSA (CRYSTALS-Dilithium) as Primary Signature Scheme | Accepted |
| [ADR-004](ADR-004-parallel-execution.md) | Block-STM Parallel Execution | Accepted |
| [ADR-005](ADR-005-verkle-trie.md) | Verkle Trie as State Commitment Structure | Accepted |

## Template

```markdown
# ADR-NNN: Title

**Status:** Proposed | Accepted | Deprecated | Superseded by ADR-NNN
**Date:** YYYY-MM-DD
**Deciders:** names

## Context
Why this decision was needed.

## Decision
What was decided.

## Alternatives considered
Table of alternatives and why they were rejected.

## Consequences
What changes as a result.
```
