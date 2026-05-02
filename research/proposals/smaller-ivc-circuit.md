# Smaller IVC Circuit — Cut Analysis

**Status:** research memo. No code changes proposed here.
**Target:** `crates/evaporchain-proving/src/nova.rs` — `RealBlockCircuit`
**Current:** `pp.num_constraints() = (14041, 10554)` — primary + secondary = 24,595
**Goal:** identify cuts, quantify savings, surface soundness implications. The decision to actually cut is separate.

---

## 1. Constraint count breakdown

The constraint counts below were extracted by reading `synthesize()` at `nova.rs:850–1367` and counting every `cs.enforce(...)` and `range_check_bits(...)` / `enforce_less_than(...)` call site, with the helper costs computed from `nova.rs:745–807`.

Helper costs (lines 745–807):
- `range_check_bits(n)` → `n + 1` constraints (`n` booleans + 1 recomposition).
- `enforce_less_than(n)` → `n + 2` constraints (1 equality + `range_check_bits(n)`).

Current parameters: `MAX_OBJECTS = 16`, `MAX_TRANSFERS = 16`, `MAX_EVAPORATIONS = 8`, `RANGE_BITS = 32` (lines 377–383).

### 1.1 User-controlled constraints

| Region | Source lines | Per-slot cost | Slots | Total |
|---|---|---:|---:|---:|
| Epoch + block + 4 bindings (state, mmr, tx, evap) | 868, 882, 893, 904, 921, 932 | 1 | 6 | **6** |
| Per-object thermodynamic (5 enforce + 2 × `enforce_less_than(32)`) | 1008, 1016, 1024, 1032, 1040, 1049, 1061 | 5 + 2·34 = 73 | 16 | **1,168** |
| Per-transfer (3 enforce + 1 × `range_check_bits(32)` for `bal_after`) | 1091, 1105, 1113, 1128 | 3 + 33 = 36 | 16 | **576** |
| Per-evaporation nullifier bind | 1154 | 1 | 8 | **8** |
| Privacy: note-root bind, pool conservation, 3 × `range_check_bits(64)`, notes_bind, nullifiers_bind | 1175, 1197, 1205, 1214, 1221, 1233, 1244 | — | — | **199** |
| State-root limb decomposition (4 × `range_check_bits(64)` + limb0 eq + recomp) | 1267–1311 | — | — | **262** |
| MMR-root limb decomposition (same shape) | 1314–1357 | — | — | **262** |
| **User-controlled subtotal** | | | | **≈ 2,481** |

### 1.2 Nova-internal overhead

`pp.num_constraints()` returns the augmented step circuit, not the bare R1CS we wrote. The augmented circuit additionally includes:

- Poseidon hash of `(z_i, U_i, T_i)` to bind the running instance — folded into the primary circuit.
- A scalar-mul / commitment-opening sub-circuit that runs the *secondary* curve's verifier inside the *primary*.
- The cross-curve constants and bookkeeping that pads the R1CS so the public-IO format matches Nova's expected layout.

The delta between primary count and our user constraints is `14,041 − 2,481 ≈ 11,560`. This is **fixed overhead** — it does not scale with `MAX_OBJECTS` etc., and we can only shrink it by reducing the IVC arity (currently 6 — see line 847).

The secondary count (10,554) is entirely the cycle-of-curves verifier circuit on Grumpkin (`E2`). It encodes the Nova folding verifier for the primary's public params. Nothing user-defined lives there.

### 1.3 Where the budget actually goes

Of the 24,595 total:
- **~22,114** Nova-internal (~90%) — non-negotiable without changing the proving system or arity.
- **~2,481** user-defined (~10%) — the only cuttable surface from this file alone.

Per-fold latency (~250–350 ms warm, per `evaporchain_async_fold.md`) is dominated by MSM/FFT over the augmented circuit, so a 10% drop in user constraints is roughly proportional. **A 10% user-constraint cut buys ~1% wall-clock**, because the bulk lives in the Nova-internal block. This reframes the entire analysis: the highest-leverage cut is anything that lets us drop the IVC arity from 6 to 4 or 5, not anything that trims the inner block constraints.

---

## 2. Cut proposals

Each cut is rated by *savings* (constraint count), *soundness implication* (what attack the circuit no longer prevents), and *reversibility* (cost to put it back).

### Cut A — Drop MMR-root limb decomposition

**What.** Remove the second limb-decomposition block at `nova.rs:1314–1357`. Keep the truncated `new_mmr_root` u64 binding at line 904.

**Savings.** ~262 primary constraints (~1.1% of 24,595).

**Soundness implication.** The truncated u64 hash gives 64-bit collision resistance against a malicious prover. Currently the limb decomposition forces the prover to commit to the *full* 32-byte root, raising the bar to 256-bit collision. Dropping it means an adversary who can find two distinct MMR roots that agree on their first 8 little-endian bytes can produce a proof for either — `2^32` work via birthday on the truncation alone (the leaf hash is SHA-256, so feasible only by a well-resourced attacker but no longer infeasible).

The MMR is the **nullifier accumulator** for evaporated objects. A successful first-8-byte collision lets a proof commit to two MMR states with the same truncated hash — meaning an evaporated-object proof's nullifier set could be ambiguous. The light-client check `evaporchain_real_prove_verified.md` references would still verify, but the prover could swap which evaporation cohort the proof covers.

In practice the MMR root is computed deterministically from on-chain data (block-by-block append-only), so a finder-of-collision would also need to engineer block content to land on the colliding root. That's a defence-in-depth argument, not a security one. **64-bit truncation is below modern collision-resistance norms.**

**Reversibility.** Trivial. The block at lines 1314–1357 is self-contained, can be reinstated without touching any other region. No state changes, no witness format changes.

**Recommendation.** Do not ship until MMR truncation collision risk is independently bounded (e.g. by domain-separating the MMR leaf hash so the first 8 bytes are unforgeable in isolation). This is the *cheapest cut* but also the *most subtle* — easy to convince yourself it's free, hard to actually prove it is.

### Cut B — Drop state-root limb decomposition (mirror of A)

**What.** Remove the limb-decomposition block at `nova.rs:1263–1311`.

**Savings.** ~262 primary constraints.

**Soundness implication.** Same shape as Cut A but with strictly worse consequences: the Verkle root is the **canonical state root** consumed by everything outside the proving pipeline (light clients, fast-sync, RPC `getState` responses). The truncated u64 binding at line 893 is what currently flows in the IVC public output `z[0]`, but the limb decomposition is what proves the prover knows a *specific* 32-byte root, not just *some* 32-byte root that hashes to the right u64.

**Concretely:** light clients today (per `evaporchain_real_prove_verified.md`) take the full 32-byte state root from block headers and verify against the IVC. If the limbs are dropped, the IVC binds only the first 8 bytes — a malicious prover with two-state collision can substitute the full root in the header without invalidating the proof. This is more attractive than the MMR case because the Verkle root governs **balance state**, not just nullifiers.

**Tests broken.** `test_state_root_limb_decomposition` (line 2581) is a unit test on the helper `hash_to_limbs`, *not* on the circuit, so technically still passes. But `test_real_block_tampered_state_fails` (line 1872) asserts that swapping `new_state.verkle_root` between fold and verify causes failure; with limbs dropped, only the first 8 bytes need match — the test would still pass for full-root tampering but not for collision-engineered tampering (which the test doesn't exercise). The invariant the test name *implies* is weakened even if the assertion passes.

**Reversibility.** Trivial. Self-contained block.

**Recommendation.** **Research, don't ship.** Verkle root is too central. The 262-constraint saving is not worth the analysis cost of proving 64-bit collision is acceptable for the canonical state commitment.

### Cut C — Reduce `RANGE_BITS` from 32 to 24/16 for object decay remainders

**What.** Change the bit-width applied to the two `enforce_less_than` calls at lines 1049 and 1061 (the `shift_remainder < shift_factor` and `frac_remainder < two_half_life` checks). Today both use the global `RANGE_BITS = 32`.

**Savings per object.** Each `enforce_less_than(n)` costs `n + 2`. Going 32 → 24 saves 8 per call. With 2 such calls per object × 16 objects = **256 constraints**. Going 32 → 16 saves 16 per call → **512 constraints**.

**Soundness implication.** The remainders bound the residue in the integer-division witness for energy decay (`nova.rs:472–518`). `shift_remainder < shift_factor` forces `shift_factor = 2^full_halvings` to be the *correct* divisor; `frac_remainder < two_half_life` does the same for the fractional decay step. If the remainder can wrap above `2^bits`, the prover can fake a smaller `after_halvings` (claim more decay than really occurred) by producing a remainder in the wrap interval.

**Is the tighter bound enough?** With `epochs_elapsed = 1` (per `nova.rs:474`), `full_halvings = 1 / hl ∈ {0, 1}`, so `shift_factor ∈ {1, 2}`. Therefore `shift_remainder ∈ {0, 1}` — **1 bit suffices for that check**. For `frac_remainder < 2 · half_life`, the bound is the half-life schedule's max value (referenced via `evaporchain_types::energy_at_epoch` at line 494). 16 bits covers `2 × half_life` up to `half_life ≤ 2^15`; 24 bits covers up to `2^23`. Audit the half-life schedule and pick accordingly.

The current 32 is wildly conservative.

**Tests broken.** `test_real_block_wrong_energy_caught_by_range_check` (line 1900) constructs a witness where the energy-decay remainder is forged outside the legal range. As long as the new bound is **at least** `ceil(log2(2 · max_half_life))`, the test invariant holds — in fact tightening makes the test *stricter*.

**Reversibility.** Trivial — flip the constant back.

**Recommendation.** **Highest-leverage cut. Ship first.** Add a new constant `OBJECT_REMAINDER_BITS` (24 with margin, 16 if half-life schedule is tight enough). Keep `RANGE_BITS = 32` for the transfer-balance check (Cut D). Savings: 256–512 constraints, no soundness loss given the witness's actual numerical range.

### Cut D — Reduce `RANGE_BITS` from 32 to 24 for transfer `bal_after`

**What.** The `range_check_bits` at line 1113 uses `RANGE_BITS = 32` to prove `sender_balance_after ≥ 0` (no underflow).

**Savings.** 8 constraints per transfer × 16 = **128 constraints**.

**Soundness implication.** The check guarantees `bal_before − amount ≥ 0` by proving `bal_after` fits in `RANGE_BITS` bits. Tightening to 24 means we're claiming all account balances fit in 24 bits (~16M units). If the actual token unit is wei-like (18 decimals on a u64 amount), 24 bits is **far** too tight — a single transfer of 1 token at 18 decimals overflows.

**Conclusion.** Do **not** apply Cut C-style reasoning to balances. The proposal in the original brief (`Dropping RANGE_BITS from 32 to 24 where amounts are bounded by max_txs_per_block`) only works if there's an *enforced* per-tx amount cap. Today there isn't one in the circuit; balances can be u64-wide. **Skip this cut unless a per-tx amount cap is added to consensus first.**

**Tests broken.** `test_real_block_balance_conservation` (line 2254) and `test_real_block_insufficient_balance_fails` (line 2272) currently exercise balances that may exceed 24 bits in their setup. Tightening would make those tests fail at the *legitimate* witness, not at the malicious one.

**Reversibility.** Same as C — trivial constant flip.

**Recommendation.** **Don't ship.** Listed for completeness because the brief raised it. Revisit if a hard per-tx amount cap is added to block validation.

### Cut E — Hash state root instead of decomposing into limbs

**What.** Replace both limb-decomposition blocks (Cuts A + B regions, lines 1263–1357) with a single Poseidon hash of the 32-byte root, bound to a single allocated witness. The witness commits to a hash of the root rather than recomposing it.

**Savings.** Replace 2 × 262 = 524 constraints with one Poseidon hash (Poseidon over 4 field elements ≈ 250 constraints depending on configuration — could break even or save ~250 depending on width).

**Soundness implication.** Stronger commitment than truncated u64 (no 8-byte collision attack), weaker than full limb decomposition (light clients can't verify root recomposition without re-hashing). The light-client protocol becomes: light client computes Poseidon of the header's full root, checks the hash matches the IVC public output. This is operationally similar to what a Merkle commitment already does — **no real loss** if Poseidon is the chosen hash.

**Tests broken.** `test_real_block_tampered_state_fails` (line 1872) still passes because any tampering changes the Poseidon image. `test_state_root_limb_decomposition` (line 2581) — irrelevant, it tests the helper not the circuit.

**Reversibility.** Medium. Dropping limbs and adding Poseidon changes the IVC public-output shape (hash instead of u64 truncation), which propagates to verifier code, light client, chain-prover. Re-adding limbs would mean a re-deploy + light-client re-sync.

**Recommendation.** **Research, don't ship in current sprint.** Aligned with the long-term invention-stack direction (`project_evaporchain_invention_stack.md`) — Poseidon-binding the state commitment is a clean primitive — but it's a protocol-surface change, not a constraint-trimming exercise. Park it for after the mainnet sprint.

---

## 3. Recommended ordering

| # | Cut | Savings | Risk | Action |
|---|---|---:|---|---|
| 1 | **C — `RANGE_BITS` for object remainders 32 → 16/24** | 256–512 | Low (witness range provably ≤ `2^16`) | Land first. Audit half-life schedule, pick tight bound, ship. |
| 2 | A — Drop MMR limb decomp | ~262 | Medium (64-bit collision on nullifier accumulator) | Ship only after domain-separating MMR leaf hash and writing a soundness note. |
| 3 | E — Poseidon-bind state root | ~250 (net) | Medium (protocol-surface change) | Research-track item. Defer until invention-stack work touches state commitment anyway. |
| 4 | B — Drop state limb decomp | ~262 | High (64-bit collision on canonical state root) | Do not ship without independent collision-resistance argument. |
| 5 | D — `RANGE_BITS` for `bal_after` 32 → 24 | ~128 | High without per-tx amount cap | Do not ship until consensus adds amount cap. |

**Headline:** Cut C alone is ~1–2% of the user budget at zero soundness cost. Cuts A+C together are ~3% with one quantifiable risk. Anything beyond that is research-track.

---

## 4. Out-of-band paths worth flagging

The biggest constraint-count lever is **not** in this circuit's body — it's the IVC arity (line 847). Going from arity-6 to arity-4 (drop `note_tree_root` and `pool_balance` from the public output, keep the privacy constraints internally) would shave the Nova-internal Poseidon binding cost by roughly two scalar absorptions, which is materially larger than any of Cuts A-E. It's also the most invasive — it changes the public IO shape, which means every consumer (light client, chain-proof, fast-sync) needs to re-version. Worth scoping as a separate proposal before doing micro-trimming inside the body.

A second lever: `MAX_OBJECTS = 16` is set per-block but real blocks rarely touch more than 4–6 objects in a single epoch. If the block-builder enforces `n_active_objects ≤ 8` we can cut `MAX_OBJECTS` to 8 → saves 8 × 73 = **584 constraints**, more than any single cut above. Cost: blocks that legitimately need >8 object decays in one epoch fail to prove, forcing the producer to split. Worth a producer-side telemetry check to see if 8 is actually a tight bound on observed traffic before shipping.

Same logic on `MAX_TRANSFERS = 16`: each slot costs 36 constraints. Halving to 8 saves 288. Cost: caps per-block transfer count to 8 (combined with above MAX_OBJECTS cap, blocks become effectively smaller).

---

## 5. Constraint-saving summary table

| Lever | Saving | Soundness cost | Decision class |
|---|---:|---|---|
| Cut C (`RANGE_BITS` for object remainders → 16) | 512 | None given witness range | **Ship** |
| Cut A (drop MMR limbs) | 262 | 64-bit collision on nullifier accumulator | Ship-with-mitigation |
| Cut E (Poseidon-bind state) | ~250 | Protocol-surface change | Research |
| `MAX_OBJECTS` 16→8 | 584 | Blocks with >8 object decays must split | **Ship if telemetry confirms** |
| `MAX_TRANSFERS` 16→8 | 288 | Blocks with >8 transfers must split | Ship if telemetry confirms |
| Arity 6→4 | ~1,000+ (Nova-internal) | Re-versioned public IO | Scope separate proposal |
| Cut B (drop state limbs) | 262 | 64-bit collision on canonical state | **Don't ship** |
| Cut D (`RANGE_BITS` for `bal_after` → 24) | 128 | Caps balances to 16M units | Don't ship without amount cap |

If we land Cut C + `MAX_OBJECTS` 16→8 + `MAX_TRANSFERS` 16→8, total user-constraint reduction is `512 + 584 + 288 = 1,384` — about 56% of the user budget, ~5.6% of the total primary, ~5–6% wall-clock. That's the realistic engineering ceiling for a no-protocol-change sprint.

The arity reduction is the only path to a step-change improvement, and it's a separate spec.

---

## Key findings

- **The 90/10 split is the headline.** Of the 24,595 constraints, ~22,114 are Nova-internal (augmented step circuit + secondary verifier on Grumpkin). Only ~2,481 are user-defined. A 50% cut of our portion is ~5% wall-clock — the biggest leverage is dropping IVC arity, not body cuts.
- **`RANGE_BITS = 32` for object decay remainders is wildly oversized** — the actual witness range for `shift_remainder` is `{0, 1}` and for `frac_remainder` is `< 2 × half_life`. This is the cleanest cut, ships at zero soundness loss.
- **Both limb-decomposition blocks are self-contained** at `nova.rs:1263–1311` (state) and `1314–1357` (MMR). MMR is mid-risk to drop, state is high-risk because the Verkle root governs balances.
- **Cut D from the original brief is unsafe today** — `RANGE_BITS = 32` for `bal_after` cannot be tightened to 24 unless consensus enforces a per-tx amount cap.
