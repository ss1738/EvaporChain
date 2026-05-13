# `research/` — canonical research artifacts

Updated 2026-05-13 alongside the doc-hygiene pass (see `../docs/archive/2026-05-13-hygiene/README.md`).

This directory holds the canonical, live research surface for EvaporChain. Historical phase-decision logs and gate-result artifacts have been moved to `../docs/archive/2026-05-13-hygiene/`; only live research stays here.

## Canonical strategic doctrine

- **`INEVITABILITY_STRATEGY.md`** — the master strategic doctrine. Read first for any non-tactical decision.
- **`INVENTION_STACK.md`** — the locked list of 5 invention-stack primitives + supporting Tier-2/Tier-3 substrates.
- **`APPLICATION_UNIVERSE.md`** — 12-category application taxonomy for what fits the energy-decay primitive, including the unique-value-prop filter, Satyawan's personal build queue, and Foundation grants priorities. Built 2026-05-13.

## Formal papers

- **`whitepaper.md`** — the public technical whitepaper.
- **`papers/paper_1_mechanism.md`** — Paper 1: energy-decay state management as a protocol primitive.
- **`papers/paper_2_state_economics.md`** — Paper 2: economic argument that infinite-state chains are unsustainable.
- (Paper 3 — the empirical benchmark — pending future build session per the 2027 milestone in `INEVITABILITY_STRATEGY.md`.)

## Frontier sub-papers

`frontier/` holds three formal sub-papers + their proof companions:

- `01-poha-decaying-da.md` + `01-poha-decaying-da-proof.md` — PoHA Decaying DA layer
- `02-energy-verkle-trie.md` + `02-energy-verkle-trie-proof.md` — Energy-annotated Verkle trie subtree compression
- `03-rule-based-consensus.md` + `03-rule-based-consensus-proof.md` — Rule-Based Consensus anchor scheme

## Formal verification

- `tla/` — TLA+ specifications (RuleBasedConsensus etc.)
- `coq/` — Coq mechanized proofs
- `proofs/conservation_proof_notes.md` — conservation-property proof notes

## Active proposals

- `proposals/smaller-ivc-circuit.md` — research memo on RealBlockCircuit constraint reduction. Still relevant for Section 3 RelaxedR1CS work in the bridge crate (see `crates/evaporchain-nova-bridge/SECTION_3_RELAXED_R1CS.md`).

`proposals/energy-stamped-mev-resistance.md` was archived in the 2026-05-13 hygiene pass — chain already has encrypted-mempool MEV defense per `docs/architecture.md` section §evaporchain-consensus.

## Empirical research data

- `causal-chsh/` — Bell-inequality / quantum-randomness verification data (CSVs + Python scrape script). The README documents the methodology.
- `mera-gate/` — MERA-gate research data (Python script).

## Going-forward rule

Per `meta_strategic_question_flow.md` (memory): write strategic-thinking output into `INEVITABILITY_STRATEGY.md` (or one of the existing canonical files), not into new `research/` sub-docs. The "decision sediment" pattern (PHASE_N_DECISIONS.md, multiple GATE_RESULT.md files at different paths) led to a hygiene pass; the going-forward rule prevents it recurring.

Empty subdirectories (`crooks_mev/`, `lambda_fold/`, `light_cone/`) after the 2026-05-13 hygiene pass can be removed when convenient — git doesn't track them so they appear/disappear with the filesystem state.
