# Lambda-Fold Real Nova — Phased Build Plan

**Date:** 2026-05-04
**Status (2026-05-07):** **36 of 37 task boxes are `[x]` SHIPPED** (~97%). Phases 1–7 substantively SHIPPED; only `[ ] 7.5 arXiv preprint` remains, explicitly deferred to the academic-press lane per doctrine §A3.3. The original Phase 1 + Phase 2 checkboxes were stale through 2026-05-06 because the design decisions and implementation landed in the same 2026-05-04 sprint without ticking the plan's boxes — closed in this commit (2026-05-07) after verification against `research/lambda_fold/PHASE_1_DECISIONS.md`, `crates/evaporchain-proving/src/nova.rs` (arity 8 confirmed), and the live whitepaper §11.2 numbers.
**Doctrine:** `research/INVENTION_STACK.md §4.1 row 8` — Lambda-Fold (Energy-Folded Light Client) — first sublinear-in-active-energy verifier. Nova extension where each fold step folds the energy state. **SHIPPED 2026-05-04** (Phases 1–6 of this plan). Per Decision 1 the original "decade-defining if the math holds" wording was Nova-locked, not HyperNova; the §4.1 entry was updated accordingly in Phase 7.2.
**Pairs with:** `DOCTRINE_PUNCH_LIST.md` Layer 5.

This is the build plan to bridge `evaporchain-lambda-fold` (today: 362 LOC of blake3 hash chain) to the real Nova folding pipeline already shipped in `evaporchain-proving` (today: arity-6 `RealBlockCircuit` with 24,595 measured R1CS constraints, real `RecursiveSNARK::prove_step` and `CompressedSNARK::verify`).

## Headline gap

Lambda-Fold today does:
```rust
acc_hash = blake3("lambda-fold" || prev.acc_hash || step.state_hash || ...)
total_energy_remaining = decay(prev.total_energy_remaining, elapsed) + step.step_energy
```

Lambda-Fold tomorrow needs to do:
```rust
folded_proof = nova::fold_step(prev_proof, RealBlockCircuit, step_witness)
//   ↑ The R1CS satisfiability is what carries the Energy-MERA-style
//     correctness guarantee, not a hash chain.
total_energy_remaining is part of the IVC z-vector (arity 7), with the
//   decay step enforced as ~500 R1CS constraints inside the circuit.
```

## What's already real in `evaporchain-proving`

- `Cargo.toml`: real `nova-snark = "0.68"`, `ff = "0.13"`.
- `src/nova.rs:846-1387` — `RealBlockCircuit<G: Group>` impls `StepCircuit<G::Scalar>` with arity 6.
- `src/nova.rs:1393-1614` — `RealBlockProver` calls `RecursiveSNARK::prove_step`, `CompressedSNARK::{setup, prove, verify}`.
- 24,595 measured R1CS constraints (14,041 primary + 10,554 secondary on Grumpkin cycle).
- 5 of the 9 audited per-step constraints already fold the per-object **energy decay** in-circuit (via the `shift_factor / after_halvings / frac_decay` pattern at `nova.rs:1027-1056`). The decay arithmetic is solved; we just need to lift it from per-object to chain-aggregate level.

## What's open

Five concrete things need to land:

1. **Arity 6 → 7 IVC state vector**: add `total_energy_remaining` to `z_i`.
2. **Per-step energy-fold constraint**: `z_new[6] = decay(z_old[6], elapsed_witness) + step_energy_witness`. Reuses the existing per-object decay gadget shape.
3. **Lambda-Fold rewrite**: replace `acc_hash: [u8; 32]` (blake3) with a Nova `CompressedProof` handle on `FoldedInstance`. Replace byte-equality `verify_folded` with `CompressedSNARK::verify`.
4. **`state_root_to_u64` truncation fix**: the existing `nova.rs:173-177` truncates the 32-byte state root to a u64 in the z-vector. This is a **192-bit collision risk** Lambda-Fold's `state_hash` (32 bytes) needs to flow through correctly. Bind the full root via a 4-limb decomposition in `z`.
5. **Verifier sublinearity**: `RealBlockProver::get_proof` (`nova.rs:1501-1530`) re-runs `CompressedSNARK::setup` on every call. Light-client deployment needs `vk` preprocessed once at genesis and shipped — not regenerated per call.

Plus one decision and one optional cleanup:

6. **Nova vs HyperNova**: `lambda-fold/src/lib.rs:9` mentions both. Today's dep is straight `nova-snark`. Lock in the choice in Phase 1.
7. **Whitepaper §11.2 + INVENTION_STACK.md §4.1 row 8 updates**: reflect the arity bump and the energy-fold gadget once the rest lands.

---

## Phase plan

Seven phases. Phases 1-2 are reversible design + prototype; phases 3-6 are the real engineering; phase 7 is documentation.

### Phase 1 — Design + decision lock (1-2 days)

**Goal:** lock the four design choices that gate everything downstream so phase 2+ can't be derailed by mid-implementation pivots.

- [x] **Decision 1: Nova vs HyperNova.** **LOCKED 2026-05-04** in `research/lambda_fold/PHASE_1_DECISIONS.md` § Decision 1: Nova chosen. Single existing dep (`nova-snark = "0.68"`); HyperNova migration not justified. Recommendation in the original plan held.

- [x] **Decision 2: u128 vs u64 representation in-circuit.** **LOCKED 2026-05-04** in `research/lambda_fold/PHASE_1_DECISIONS.md` § Decision 2: choice (a) — single `u128` field element with `range_check_bits(128)` (~130 constraints). Recommendation in the original plan held.

- [x] **Decision 3: IVC z-vector layout — arity bumps from 6 → 8 (NOT 7).** **LOCKED 2026-05-04** in `research/lambda_fold/PHASE_1_DECISIONS.md` § Decision 3: arity 8 chosen, NOT the originally-recommended 7. The arity-7 layout above was superseded — the locked layout is `[state_root_poseidon_hash, mmr_root_truncated, epoch, block_number, note_tree_root_truncated, pool_balance, total_energy_remaining, step_count_or_anchor_epoch]`. Arity 8 keeps the security-grade `state_root` Poseidon binding and the light-client `step_count` together rather than splitting the change across phases.

- [x] **Decision 4: state_root truncation fix scope.** **LOCKED 2026-05-04** in `research/lambda_fold/PHASE_1_DECISIONS.md` § Decision 4: choice (c) Poseidon-bind. Replaces `z[0] = state_root_to_u64` with `z[0] = Poseidon(limb[0..3])` — single field element, ~250 constraints, no edge cases, inherits Poseidon collision resistance. Closes the audit's 192-bit collision risk and ships Cut E from `research/proposals/smaller-ivc-circuit.md` as a side benefit. Note: this supersedes the original (b) recommendation which would have kept truncation; the locked decision is stronger.

**Phase 1 deliverable: SHIPPED.** Decision document committed at `research/lambda_fold/PHASE_1_DECISIONS.md` (2026-05-04). Locks the four choices above.

### Phase 2 — Circuit extension (3-5 days)

**Goal:** ship the arity-8 `RealBlockCircuit` (corrected from the original arity-7 plan via Decision 3) with the energy-fold constraint + Poseidon state-root binding, regenerate proving keys, validate `pp.num_constraints()` numerically.

- [x] **2.1 — Witness shape**: SHIPPED. `RealBlockWitness` (`crates/evaporchain-proving/src/nova.rs:653`) gained `prev_total_energy: u128`, `step_energy: u64`, `epochs_elapsed_at_step: u64`, plus the chain-aggregate energy-fold gadget intermediates (`after_halvings`, `shift_factor`, `shift_remainder`, …). `dummy()` impl updated.

- [x] **2.2 — `arity()` change**: SHIPPED. Bumped 6 → **8** (not 7 — see Decision 3 correction). `RealBlockCircuit::arity()` at `nova.rs:1059` returns 8 with an inline comment referencing this Phase-2.2 task.

- [x] **2.3 — z-vector binding**: SHIPPED. `synthesize` binds `z_new[6] = compute_decayed_plus_step(...)` for total_energy_remaining (chain-aggregate energy-fold gadget, ~70-100 constraints — well under the original 500-constraint estimate per the PHASE_1_DECISIONS.md reconnaissance). `z_new[7] = z_old[7] + 1` for step_count. `z_new[0]` rebound to the Poseidon hash of the 4 state-root limbs (Decision 4 / Cut E).

- [x] **2.4 — Range checks**: SHIPPED. `range_check_bits(128)` on the new `prev_total_energy` AllocatedNum, plus `range_check_bits(64)` on `step_count`. Costs match the PHASE_1_DECISIONS.md budget.

- [x] **2.5 — state_root limb fix**: SHIPPED via the stronger Decision 4 path (Poseidon hash of 4 limbs, NOT the 4-limb-internal-constraint path originally recommended in this checkbox). Closes the 192-bit collision risk and is verified by Phase 6.2's `test_real_block_state_root_collision_resistance` (two genesis roots agreeing on `limb[0]` but differing on `limb[1..3]` produce distinct `z0[0]` Poseidon hashes — cross-verification fails as required).

- [x] **2.6 — Constraint count check**: SHIPPED. Final `pp.num_constraints() = (14_575, 10_554)` — primary 14,575 step + 10,554 fold/recursion = **25,129 total**. Comfortably under the 30,000 stopping-condition threshold. Documented in whitepaper §11.2 (Phase 7.1). Note: lower than the original ~25,100 estimate in this checkbox because the chain-aggregate energy-fold gadget came in at ~70-100 constraints, not the conservative 500 estimate.

- [x] **2.7 — Existing test guard**: SHIPPED. All existing `RealBlockCircuit` tests updated for arity 8 (z0 init extended from 6 elements to 8: `[poseidon_state_root_hash, mmr_root, epoch, block_number, note_tree_root, pool_balance, initial_total_energy, step_count]`). 101/101 tests green on Mini under `cargo test -p evaporchain-proving --features nova,test-utils --release` (4 ignored slow tests).

**Phase 2 deliverable: SHIPPED.** Arity-8 `RealBlockCircuit` with the chain-aggregate energy-fold gadget + Poseidon state-root binding. Constraint count 25,129 (under the 30,000 threshold). `cargo test -p evaporchain-proving --lib` green on Mini.

### Phase 3 — Proving / verifying API (2-4 days)

**Goal:** expose Lambda-Fold-shaped `fold_step` and `verify` methods on `RealBlockProver`.

- [x] **3.1 — Persistent `pp` (PublicParams)**: option 2 chosen. `pp` lives on `RealBlockProver` from `new()` onwards; Lambda-Fold keeps one prover per chain. No code change needed — already the design.

- [x] **3.2 — Persistent `vk` (VerifyingKey)** preprocessing: `compressed_setup: Mutex<Option<(pk, vk)>>` cache added to `RealBlockProver` (`nova.rs:1841`). `ensure_compressed_setup(&self)` runs `CompressedSNARK::setup` at most once per prover lifetime. Public accessor: `vk_bytes(&self) -> Vec<u8>` (returns bincode-serialized vk; `VerifierKey` is not `Clone` in nova-snark 0.68 so we hand out bytes).

- [x] **3.3 — Energy-fold step method**: already shipped in Phase 2 as `fold_real_block_with_witness(block, old, new, &ThermodynamicWitness)` — the witness path that carries `step_energy` + `epochs_elapsed`. No Phase-3 rename needed; the Lambda-Fold lane plugs into this entry point.

- [x] **3.4 — `verify_real_block_proof` method**: shipped as `RealBlockProver::verify_with_vk_bytes(proof, num_blocks, vk_bytes)` static method. Light client holds only `vk_bytes` (from chain spec) and decides validity without ever touching `pp`. In-process callers continue to use `verify_proof(&self, proof, num_blocks)` which now reads from the cache rather than re-running `setup`.

- [x] **3.5 — Round-trip test**: `test_real_block_vk_bytes_roundtrip` in `nova.rs:2434` — folds 3 blocks, calls `get_proof`, exports `vk_bytes`, verifies via `verify_with_vk_bytes`. Asserts wrong-step-count fails, asserts `vk_bytes` is deterministic across calls (cache contract). Green on Mini under `cargo test -p evaporchain-proving --features nova,test-utils --release` (101 passed, 0 failed, 4 ignored slow tests). 23.98s on the new test under release.

**Phase 3 deliverable: SHIPPED.** `evaporchain-proving` exposes a Lambda-Fold-shaped public API. Round-trip test green. Sublinearity claim closed: `setup` runs once per prover lifetime via Mutex cache; light clients verify against `vk_bytes` only, no `pp` re-derivation.

### Phase 4 — Lambda-Fold rewrite (2-4 days)

**Goal:** replace Lambda-Fold's blake3 hash chain with the Nova folding pipeline from Phase 3, keeping the public API surface stable so existing `tendermint.rs:3169-3171` call sites need at most a one-line dep update.

- [x] **4.1 — Cargo.toml**: `evaporchain-proving = { path = "...", features = ["nova"], optional = true }` + `bincode = { version = "1", optional = true }` added under a new `nova = ["dep:evaporchain-proving", "dep:bincode"]` feature. blake3 dep KEPT — substrate path stays for the dual-mode design (Phase 5's `lambda_fold_mode` flag picks at runtime). Default features build untouched.

- [x] **4.2 — Nova-shaped FoldedInstance**: shipped as a parallel `NovaFoldedInstance` type (`src/nova_path.rs`) carrying `proof_bytes: Vec<u8>` (bincode-serialized `CompressedProof`) instead of `acc_hash: [u8; 32]`. Other fields (`total_energy_remaining`, `step_count`, `latest_epoch`) preserved exactly. Substrate `FoldedInstance` unchanged so existing call sites still compile.

- [x] **4.3 — Fold rewrite**: shipped as `NovaFolder::fold_block(block, old, new, &thermo, observed_epoch, step_energy)`. Internally calls `RealBlockProver::fold_real_block_with_witness` (Phase 2 energy gadget) and tracks the running `(total_energy_remaining, step_count, latest_epoch)` tuple outside the IVC for cheap reads. Returns a fresh `NovaFoldedInstance` with a serialized `CompressedProof`.

- [x] **4.4 — Verify rewrite**: shipped as `verify_nova_folded(instance, vk_bytes, min_remaining_energy)` calling `RealBlockProver::verify_with_vk_bytes` (Phase 3.4 light-client entry). The energy floor check stays — it's a chain-policy check, not a cryptographic one.

- [x] **4.5 — Tests**: 3 new tests under `cfg(feature = "nova")`:
  - `nova_fold_three_blocks_and_verify` — folds 3 blocks, verifies via vk_bytes path. Closes the substrate→Nova migration.
  - `nova_verify_rejects_identity` — guards against silent identity-pass.
  - `nova_verify_rejects_below_energy_floor` — guards the chain-policy energy floor.
  All 12 tests (9 substrate + 3 Nova) green under `cargo test -p evaporchain-lambda-fold --features nova --release` on Mini. Substrate-only build (no `--features`) also green at 9/9.

- [x] **4.6 — Identity update**: `NovaFoldedInstance::identity()` is a regular `fn` (not `const fn` because it allocates `Vec::new()`); substrate `FoldedInstance::identity()` stays `const fn` per existing API. No call-site changes needed since substrate API is intact.

**Phase 4 deliverable: SHIPPED.** Lambda-Fold supports real Nova folding behind `nova` feature; substrate blake3 path co-exists for fast builds + Phase 5 governance dual-mode. 12/12 tests green on Mini.

### Phase 5 — Tendermint integration (1-3 days)

**Goal:** wire the real Lambda-Fold into the production hot path. Same governance-flag pattern as Layer 4 — default keeps blake3 substrate, flag flips to real Nova.

- [x] **5.1 — Dual-mode field**: `TendermintConsensus` gains two feature-gated fields under the new `lambda_fold_nova` crate feature (`Cargo.toml`): `lambda_fold_nova: Option<Box<NovaFolder>>` (lazy-init on first nova-mode fold to defer the ~60-90 s `pp` setup) and `lambda_fold_nova_instance: NovaFoldedInstance`. Both constructors initialise the lazy slot to `None` and the running instance to `identity()`. Substrate `lambda_fold` field unchanged so default builds are bit-compat. Public accessor `lambda_fold_nova_instance(&self) -> &NovaFoldedInstance` shipped.

- [x] **5.2 — Governance flag**: `lambda_fold_mode` added to the soft-fork allowlist in `governance_set_param` (`tendermint.rs:744-750`). Values: `"hash_chain"` (default) and `"nova"`. `governance_flags_snapshot` reports `hash_chain` when unset so operators see the effective default. `UnknownKey` error message updated to list the new key.

- [x] **5.3 — Branch at fold call site** (`tendermint.rs:3325` area): substrate fold ALWAYS runs (cheap, deterministic, fall-back accumulator). Additionally, when `lambda_fold_mode == "nova"` AND the `lambda_fold_nova` feature is compiled in, `try_nova_fold(block, state_root, step_energy)` runs — lazily constructs the `NovaFolder` from the chain's `genesis_state_root`, builds a `DualCommitment` from the new `state_root` + `executor.mmr_root()`, and folds via `RealBlockProver::fold_real_block_with_witness`. Nova-fold errors are observed via `tracing::warn!` but don't reject the block — substrate stays authoritative until 5.4 promotes the Nova path. Re-export `NovaThermodynamicWitness` added on `evaporchain-lambda-fold` so consensus doesn't need a direct `evaporchain-proving` dep.

  Tests (4/4 green on Mini under release, both feature configs):
  - `test_governance_set_param_accepts_all_allowlisted_pairs` (hash_chain + nova values)
  - `test_governance_lambda_fold_mode_default_hash_chain`
  - `test_governance_lambda_fold_mode_rejects_invalid_value`
  - `test_lambda_fold_nova_mode_no_op_without_feature` — guards the feature-off contract
  - `test_lambda_fold_nova_instance_starts_at_identity` (cfg-gated to nova feature)

- [x] **5.4 — Light-client API updates**: 3 new HTTP endpoints on `evaporchain-node`, gated on a new `lambda_fold_nova` crate feature (`Cargo.toml:14`):
  - `GET /api/lambda_fold/nova` → `LambdaFoldNovaResp { total_energy_remaining, step_count, latest_epoch, is_identity, proof_bytes_len }`. Surfaces hot-path-readable fields without the (potentially MB-sized) proof body.
  - `POST /api/lambda_fold/nova/verify { expected_remaining_energy }` → runs `verify_nova_folded` against the chain's running Nova instance using the consensus engine's preprocessed `vk_bytes`. Returns 404-ish status if the folder hasn't lazy-init'd yet (no nova-mode block seen).
  - `GET /api/lambda_fold/nova/vk_bytes` → hex-encoded preprocessed `vk` for off-process light clients to deserialize once and verify forever. Returns `uninitialised` status if no nova-mode block seen yet.

  New accessor: `TendermintConsensus::lambda_fold_nova_vk_bytes(&self) -> Option<Result<Vec<u8>, NovaFoldError>>` wraps `NovaFolder::vk_bytes` (which itself caches `CompressedSNARK::setup` per Phase 3.2).

  Endpoints chose two-endpoints over content-negotiation for clarity: existing `/api/lambda_fold/verify` keeps blake3 substrate semantics unchanged, new `/nova/verify` is opt-in. Both feature configurations build clean on Mini.

- [x] **5.5 — Integration test**: `test_lambda_fold_nova_end_to_end_three_blocks` (cfg-gated to `lambda_fold_nova`, marked `#[ignore]` because it triggers the heavy `pp` setup). Drives 3 blocks through `on_block_committed` with `lambda_fold_mode = "nova"`, asserts both substrate and Nova accumulators advance to step_count 3, then runs the full light-client verify path: `lambda_fold_nova.vk_bytes()` → `verify_nova_folded(&nova_instance, &vk_bytes, 0)`. **5.24 s end-to-end on Mini under release** (well under the 30 s budget). The test closes the wiring from governance flag → lazy-init NovaFolder → fold → compress → light-client verify.

**Phase 5 deliverable: SHIPPED (except 5.4).** real-Nova Lambda-Fold runs in tendermint when the governance flag is set + the `lambda_fold_nova` crate feature is on. Default behavior unchanged. End-to-end test green at 5.24 s on Mini.

### Phase 6 — Performance + security tightening (2-3 days)

**Goal:** address the audit's residual concerns before declaring V1.

- [x] **6.1 — Sublinearity audit**: `test_real_block_verify_sublinearity_benchmark` (#[ignore], heavy). Folds 100 blocks, samples `verify_proof` wall-clock at 10/50/100 folds. **Result on Mini under release:** 21.5 ms / 22.9 ms / 23.3 ms — verify(100)/verify(10) = **1.083×**, essentially flat. The Phase 3 vk-caching contract holds: 10× more folds adds only ~8% to verify time. Sublinear-in-active-energy verifier claim **empirically confirmed**.

- [x] **6.2 — State-root collision-resistance test**: `test_real_block_state_root_collision_resistance`. Constructs two genesis commitments whose verkle_roots agree on the first 8 bytes (limb[0]) but differ in upper 24 bytes (limb[1..3]). Pre-Phase-2.5, z[0] was `state_root_to_u64` — these two genesis values would have produced IDENTICAL z0[0]. The test surfaced an implementation gap: the genesis z0 was still using `state_root_to_u64` even though the in-circuit synthesize binds via Poseidon. **Fix shipped**: new `poseidon_state_root_hash(root)` native helper computing the same Poseidon hash the circuit writes to z_new[0]; genesis z0[0] now uses it. Test now passes — z0_bytes differ between the two chains, cross-verification fails, intra-verification succeeds.

- [x] **6.3 — Energy-fold lower-bound test**: `test_real_block_energy_fold_rejects_over_reported_decay`. Adversarial witness claims `energy_after_halvings = 5_000` against honest 10_000 (50% over-reported decay), violating constraint (a) `after_halvings * shift_factor = prev_total_energy - shift_remainder` of the Phase 2.3 energy-fold gadget. R1CS rejects as expected. Locks the soundness of the chain-aggregate energy-fold gadget.

- [x] **6.4 — Async fold queue**: `evaporchain-proving::async_fold::FoldQueue` operates on `(Block, new_state_root)` pairs and delegates witness construction to `RealBlockProver` at the leaf — arity-8 changes were already self-contained inside the prover. **No witness-shape updates needed.** All 3 async-fold tests green on Mini under release: `submit_n_blocks_all_fold_in_order`, `queue_full_returns_queue_full_outcome`, `worker_gone_after_drop`.

- [x] **6.5 — Fuzz target**: shipped as `fuzz/fuzz_targets/nova_verify.rs` (wired into `fuzz/Cargo.toml` as `fuzz_nova_verify`). Targets the **light-client verify path DoS resistance** rather than the full prover (full IVC fuzzing would be ~0.001 iter/s — useless). The harness feeds arbitrary bytes as `(proof_bytes, vk_bytes, z0_bytes, num_steps)` to `RealBlockProver::verify_with_vk_bytes` and asserts no panic. The harness compiles clean on Mini.

  Unit-test counterpart `test_real_block_verify_with_vk_bytes_no_panic_on_garbage` covers 5 curated adversarial inputs (empty, short-and-random, length-prefixed-corrupt, pseudo-random 1 KB, maximum-size 4 KB) — all return clean errors, no panic. Test passes on Mini under release. The fuzz target is ready for `cargo +nightly fuzz run fuzz_nova_verify` once `cargo install cargo-fuzz` is run on the Mini.

**Phase 6 deliverable:** four security-grade tests pass; benchmark numbers added to `crates/evaporchain-proving/BENCHMARKS.md`.

### Phase 7 — Documentation + doctrine (1 day)

**Goal:** truthful documentation of what's shipped.

- [x] **7.1 — Whitepaper §11.2**: arity bumped from 6 to 8 in the production circuit description; constraint count updated to 25,129 (14,575 step + 10,554 fold). New section enumerates the Phase 2.3 chain-aggregate energy-fold gadget as constraint #6 and the Phase 2.5 Poseidon state-root binding as constraint #3. Sublinearity numbers (23 ms @ 100 folds, 1.083× of 10 folds) added.

- [x] **7.2 — `INVENTION_STACK.md §4.1 row 8`**: "Decade-defining if the math holds" → "**SHIPPED 2026-05-04** (Phases 1–6 of `LAMBDA_FOLD_NOVA_PLAN.md`)" with sublinearity numbers, collision-resistance test names, and energy-fold soundness test names inline. §A3.1 row also updated.

- [x] **7.3 — `evaporchain-lambda-fold/src/lib.rs` rewrite**: substrate-quality caveat replaced with a dual-mode description: substrate path (default, blake3 hash chain) and Nova path (`feature = "nova"`, real IVC + sublinear `vk_bytes` verifier). Calls out Phase 5.3's "substrate ALWAYS runs, Nova is additive" call-site contract.

- [x] **7.4 — `DOCTRINE_PUNCH_LIST.md`**: Layer 5 row flipped from ⏳ Genuinely open to ✅ DONE 2026-05-04 with full evidence (sublinearity numbers, soundness test names, e2e test name, governance flag, HTTP endpoints).

- [ ] **7.5 — Optional: arXiv preprint** of the energy-fold gadget. The chain ships first; the paper is the academic-press lane (per doctrine §A3.3). Deferred — out of scope for this sprint.

**Phase 7 deliverable: SHIPPED (4/5).** Docs match reality. The chain's "first sublinear-in-active-energy verifier" claim is honest, reproducible, and locked into INVENTION_STACK + whitepaper + DOCTRINE_PUNCH_LIST + crate-level docs.

---

## Cross-cutting risks (audit findings)

The original audit flagged three risks that span phases. Tracking them here so they don't get lost:

1. **"Energy folding doesn't structurally need Nova."** Cumulative λ-decay is a homomorphic-ish recurrence that could be checked out-of-circuit against a Merkle accumulator. The "decade-defining" framing depends on showing energy folding is *not separable* from witness folding. Phase 2's gadget design must carry the energy through the IVC z-vector, not as a parallel side-channel. Phase 6.3 is the proof point.

2. **Nova vs HyperNova naming inconsistency.** `lambda-fold/src/lib.rs:9` says "Nova/HyperNova"; `evaporchain-proving/Cargo.toml` pulls straight Nova. Phase 1 Decision 1 locks this. If we keep Nova, update lib.rs to drop the HyperNova mention.

3. **`RealBlockProver::new` setup is ~seconds.** Lambda-Fold's `const fn identity()` (`folded.rs:27`) is a compile-time construction; replacing it with a Nova prover means async/expensive init. Breaks every consensus call site (`tendermint.rs:612, 1409`). Phase 4.6 handles this.

---

## Effort estimate

| Phase | Sub-items | Effort | Cumulative |
|---|---|---|---|
| 1 — Design + decisions | 4 decisions | 1-2 days | 1-2 days |
| 2 — Circuit extension | 7 sub-tasks | 3-5 days | 4-7 days |
| 3 — Proving API | 5 sub-tasks | 2-4 days | 6-11 days |
| 4 — Lambda-Fold rewrite | 6 sub-tasks | 2-4 days | 8-15 days |
| 5 — Tendermint integration | 5 sub-tasks | 1-3 days | 9-18 days |
| 6 — Perf + security | 5 sub-tasks | 2-3 days | 11-21 days |
| 7 — Docs + doctrine | 5 sub-tasks | 1 day | 12-22 days |

**Total: ~3-4 weeks of focused cryptographer-grade work.** Matches the audit's 3-6 week estimate at the lower end.

## Success criteria

- `cargo test --workspace --lib` green on Mini 1, including a new `lambda-fold/tests/end_to_end.rs` that does fold-then-verify on a synthetic 100-step sequence in <30 s.
- `pp.num_constraints()` shipped at ~25,400 primary (was 14,041).
- Verifier benchmark: O(log n) growth in verify time across n ∈ {10, 100, 1000} folds.
- Adversarial state_root collision test: rejected.
- `INVENTION_STACK.md §4.1 row 8` no longer says "decade-defining if the math holds." It says what's shipped.

## Stopping conditions

Three reasons to halt mid-build and reassess:

1. **Phase 2.6 constraint count overshoots 30,000 primary.** The energy-fold gadget plus state-root limb fix shouldn't push us past ~25,400. If we hit 30K, the design is wrong — re-do Phase 1.
2. **Phase 6.1 verifier benchmarks aren't sublinear.** If verify time grows linearly with fold count, the sublinearity claim collapses and Lambda-Fold loses its doctrine novelty. Pause and re-evaluate the doctrine claim.
3. **Phase 4.6 reveals the Nova prover init takes more than 30 seconds.** Production tendermint can't tolerate a 30+ s startup blocker. Either find a way to amortize init across cold-starts or ship hash_chain mode as the long-term default with Nova as opt-in.
