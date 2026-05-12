# Sui Foundation Grant Application — EvaporChain

## Project Name
Decay-Native Smart Contract Patterns: Lifecycle Hooks Inspired by Move

## Description
EvaporChain is an independent Layer-1 blockchain whose smart contract VM (EvaporScript) implements thermodynamic state decay as a first-class language primitive — every on-chain object carries an explicit energy budget that depletes over time, and contracts react via lifecycle hooks (`on_grace`, `on_refresh`, `on_evaporate`). EvaporScript is **not** Move-compatible — it is a custom 44-opcode integer-arithmetic VM. The grant funds research demonstrating lifecycle patterns Move could adopt to extend its object model with temporal types.

We are not asking Sui to depend on EvaporChain. We are asking Sui to fund the research that prototypes patterns Move-the-language could choose to adopt later — an independent reference implementation showing that temporal validity can live at the type level, with empirical performance + correctness data the Move language designers can use as input.

## What We Built (Independent Implementation, Not a Move Extension)
- **EvaporScript**: a custom non-Turing-complete VM with 44 gas-metered opcodes. Includes `EpochNow`, `BlockNum`, `EnergyOf`, `RequireEpochRange`, `ComputeDecay` opcodes that bind temporal validity into the type system. Not binary-compatible with Move; demonstrates the *concept* Move could later adopt.
- **8 contract templates** with explicit `caller != creator` access control + lifecycle hooks (mortal NFTs, refresh markets, expiring tokens, etc.).
- **Live integer-arithmetic decay** verified by Coq mechanization (`research/coq/EnergyDecayMonotonicity.v`, exit-0 under Rocq 9.1.1).
- **25,435 tests** across 147 workspace crates; live 2-Mini Tailscale testnet at h=940+ lockstep.

## How This Benefits the Sui / Move Ecosystem
1. **Empirical reference for temporal-type patterns Move could adopt**: lifecycle hooks (`on_evaporate`, `on_grace`, `on_refresh`), energy-budget object metadata, deterministic integer-decay arithmetic — all implemented and tested. If Move-the-language considers extending its object model with temporal validity in a future RFC, EvaporChain provides the working reference.
2. **Move-adjacent research without Move-stack dependency**: Sui's Move stack does not have to absorb EvaporChain code; the contribution is research papers, conceptual patterns, and case studies that Move language designers can cite.
3. **Independent design space exploration**: a chain whose VM was *built around* temporal validity (rather than retrofitted) gives Move designers an existence proof for design choices Move's authors haven't faced yet.

## Amount Requested: $30,000

## Deliverables
1. **Research paper** on temporal-type design patterns in object-oriented chain VMs. Submitted to AFT or Financial Cryptography.
2. **Open-source EvaporScript reference implementation** (already MIT-licensed; this funds polish + docs + Move-side concept-mapping appendix).
3. **Developer documentation** with side-by-side concept mapping: "what an EvaporScript lifecycle hook would look like as a Move ability or extension proposal".
4. **Presentation** at one Move ecosystem event (MoveCon, Sui Builder House, or similar).

## Honest Scoping (Important)

This grant funds **independent research that may inspire Move language extensions** — it does **not** fund a Move-compatible binary, a Move-stack integration, or anything that would let Sui chain code call into EvaporChain code. Anyone reviewing this grant should understand:

- EvaporScript and Move are two different languages with two different binary formats.
- The 44 EvaporScript opcodes do not map 1:1 to Move bytecode.
- "Move-compatible" is **not** a claim we make. The deliverable is concept-level research.
- The intended audience for the deliverables is Move language designers + Sui ecosystem researchers, not Sui application developers.

## Team
Solo founder + developer (Computer Science graduate, 2026, University of Leicester). Built EvaporChain from architecture through implementation; written 38KB whitepaper with 8 academic citations; mechanized 5 Coq proof files (Rocq 9.1.1 exit-0). Independent of any existing chain ecosystem.

## Timeline
3 months from grant receipt to all deliverables.

## Open Source Commitment
All research artefacts (paper drafts, EvaporScript reference, concept-mapping docs) shipped under MIT license. Grant outputs cross-posted to the Sui ecosystem channels alongside arXiv preprint.
