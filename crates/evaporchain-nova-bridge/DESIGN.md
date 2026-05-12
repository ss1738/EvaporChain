# evaporchain-nova-bridge — design rationale + open research questions

**Phase 2.1 of T0.10 Path A.** Created 2026-05-12.

## TL;DR

Replace the IPA-in-Groth16 wrapper architecture (PRs #50-#53,
`ethereum-bridge/wrapper`) with a Nova-folding-then-Groth16 architecture.
The chain ALREADY produces a `RecursiveSNARK<Bn256EngineKZG, GrumpkinEngine, …>`
accumulator (see `evaporchain-proving::nova`). This crate's job is to
verify that accumulator inside a Groth16-on-BN254 R1CS circuit.

The constraint budget for the IPA-in-Groth16 path was ~80× over the
2^18 Powers-of-Tau ceremony; Nova folding moves the per-round IPA
verification *outside* the Groth16 circuit. The wrapper only verifies
the final accumulator state — a ~30-100k constraint circuit
(empirically TBD in Phase 2.2). Fits in 2^17 with headroom.

## Three sub-paths considered

### A1 — Wrap `RecursiveSNARK::verify` directly

Verify the running Nova accumulator as-is. No Spartan compression.

- **Pro:** simplest wrapper circuit (~30-100k constraints empirical estimate)
- **Pro:** the chain already produces `RecursiveSNARK` natively
- **Con:** `RecursiveSNARK` is large on the wire (~MB-scale). Relayer
  must transmit it from chain → L1 bridge service.

### A2 — Wrap `CompressedSNARK::verify` (Spartan layer)

The chain's *compressed* output. Currently used by the chain's light
clients (`evaporchain-light-client-{cli,http,wasm}`).

- **Con:** verifying CompressedSNARK requires in-circuit
  HyperKZG-evaluation check (BN254 pairing, ~6M constraints) + IPA
  evaluation check on Grumpkin side (non-native ops, ~1-3M) + Spartan
  sumcheck (~few hundred k). Total ~7-9M; needs 2^23 ceremony.
- **Pro:** smaller on-wire (~10KB).

### A3 — RECOMMENDED: relayer re-proves via Groth16 directly

Chain emits `RecursiveSNARK`. Bridge relayer (off-chain service):
1. Receives `RecursiveSNARK` from chain.
2. Runs an arkworks Groth16 prover whose circuit body is the A1-style
   verifier of the RecursiveSNARK.
3. Emits a 256-byte Groth16-on-BN254 proof for L1.

- **Pro:** L1 sees a small 256B proof (same as IPA-in-Groth16 path).
- **Pro:** Wrapper circuit is small (Phase 2.1's empirical question).
- **Pro:** Reuses the *existing* `evaporchain-verkle-wrapper::{setup,prove,verify}`
  Groth16 pipeline — only the `WrapperCircuit` body changes.
- **Con:** Relayer has to run the Groth16 prove every chain advance
  (~seconds). Acceptable for a bridge relayer; runs OOB.

A3 is **structurally identical** to how Polygon zkEVM's final-wrap
layer ships, and to zkSync's bridge prover architecture. It is the
production-grade choice.

## Phase 2 milestones — concrete file-level work

| Step | Deliverable | File / module | Estimate |
|---|---|---|---|
| **2.1** ✅ | This crate scaffold + design doc | `crates/evaporchain-nova-bridge/{Cargo.toml,src/lib.rs,DESIGN.md}` | DONE |
| **2.2** | In-circuit `RecursiveSNARK::verify` PoC | `src/verifier_circuit.rs` — implements `ConstraintSynthesizer<ark_bn254::Fr>`; verifies a *dummy* fold over `Bn256EngineKZG + GrumpkinEngine`. **Output: empirical constraint count.** | 3-5 days |
| **2.3** | Wire to real chain accumulator | `src/adapter.rs` — converts `evaporchain-proving::nova::CompressedProof` bytes back to `RecursiveSNARK<E1, E2, BlockStepCircuit<G1>>` and feeds to the verifier circuit | 2-3 days |
| **2.4** | Groth16 setup + prove + verify roundtrip | `src/prover.rs` — mirrors `evaporchain-verkle-wrapper::prover` shape; produces 256-byte L1 calldata | 2 days |
| **2.5** | Solidity contract smoke test | `ethereum-bridge/contracts/test/NovaBridgeVerifier.t.sol` — consumes the new wrapper's proof + asserts L1 verifies | 1 day |
| **Total** | end-to-end empirical evidence | | ~10-14 days |

## Key open research questions (block Phase 2.2)

### Q1 — Which arkworks gadgets compose the Nova verifier?

Nova's verifier algorithm (per the paper + `nova-snark/src/nova/mod.rs:567`):
1. **Two folding-step checks** — one per curve in the cycle. Each:
   - Re-derive the random oracle challenge `r` via Poseidon over the
     transcript.
   - Recompute the folded RelaxedR1CS instance: `(U + r·u, W + r·w, …)`.
2. **Final accumulator R1CS-satisfaction** — check that the folded
   instance satisfies the step-circuit's R1CS.

Open: which arkworks-side gadgets (existing or new) match each step?
The Poseidon side has well-known gadgets; the RelaxedR1CS instance
recomputation is more bespoke.

### Q2 — Native field mismatch (BN254 Fr vs Grumpkin Fr)

The chain's `E1 = Bn256` uses Fr-of-BN254 as its native field.
The bridge's Groth16 wrapper is over BN254, so ALSO Fr-of-BN254 native.
That side is cheap.

The chain's `E2 = Grumpkin` uses Grumpkin's Fr, which equals BN254's
Fq (the BASE field). In the wrapper circuit, Grumpkin's Fr operations
become NON-NATIVE arithmetic over BN254 Fq. This is exactly the same
shape as the IPA-in-Groth16 path's Pallas-Fq-in-BN254-Fr problem.

**Critical:** the wrapper still has SOME non-native arithmetic. The
question is how much. If the verifier algorithm does ~O(1) Grumpkin
ops per fold step, total cost is ~O(1) non-native ops — manageable.
If it does ~O(log n) ops per step where n is the IPA size, the cost
grows with chain history depth — needs measurement.

### Q3 — Can the existing `evaporchain-verkle-wrapper` primitives be reused?

PRs #50, #51, #52, #53 ship non-native `enforce_g1_add`,
`enforce_g1_doubling`, `enforce_scalar_mul` over Pallas. The Nova
wrapper needs the same shape over **Grumpkin**, not Pallas. The
gadget code can be parameterised by curve, but the actual
non-native gadget instantiation must use `ark-grumpkin::Fq` as the
target field and `ark-bn254::Fr` as the base. Probably a ~few-hours
parameterisation refactor.

### Q4 — What's the actual constraint count?

This is what Phase 2.2 measures empirically. The estimate ~30-100k
is from Nova/MicroNova benchmarks on a single folding step; it scales
linearly with the verifier algorithm's R1CS shape. **No commitment
to that number until 2.2 ships measurement.** If the measurement is
much larger (e.g., ~1M), A1/A3 still beat A2 by ~10× and beat the
IPA-in-Groth16 path by ~20×. So the architecture wins regardless.

## What does NOT change vs IPA-in-Groth16 path

- L1 contract surface — `VerkleProofVerifier.sol` still consumes a
  256-byte Groth16-on-BN254 proof via EIP-197 pairing precompile.
- The 4 public-input anchors — `state_root`, `key`, `value_commitment`,
  `params_fingerprint` remain the L1-visible commitment.
- The chain side's existing Nova IVC stack (`evaporchain-proving::nova`).

## What DOES change

- The wrapper circuit body (now verifies a Nova accumulator, not a
  Halo2 IPA proof).
- The bridge relayer (now reads `CompressedProof` from chain and runs
  Groth16 prove — replacing the previous design that ran Halo2 IPA on
  the relayer).
- The existing `evaporchain-verkle-wrapper` crate becomes legacy /
  alternative-path code. It is NOT deleted in Phase 2; it remains as a
  fallback in case A3 hits an unforeseen blocker. PRs #50-#53 ship
  primitives that may get reused under Path A's Q3.

## Decisions baked in by this commit

- **A3 is the chosen sub-path.** Phase 2.2-2.5 build out the A3
  pipeline. A1/A2 are explicitly deferred (would require their own
  Phase 2 arcs).
- **New crate, not extension of `evaporchain-verkle-wrapper`.** Keeps
  the two architectures cleanly separated during the pivot. Future
  consolidation possible once Path A is operationally proven.
- **2^17 ceremony budget target.** Phase 2.2 measures actual cost; if
  it exceeds, ceremony scales to 2^18 / 2^20 as needed. No upfront
  commitment to a ceremony size beyond "needs to fit Phase 2.2's
  empirical number."
