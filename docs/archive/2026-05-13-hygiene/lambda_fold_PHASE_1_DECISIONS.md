# Lambda-Fold Real Nova — Phase 1 Decisions

**Date:** 2026-05-04
**Pairs with:** `LAMBDA_FOLD_NOVA_PLAN.md` Phase 1.

This file locks the four design choices that gate Phase 2+. Re-litigating them mid-implementation will derail the build; the doctrine punch list's "stopping conditions" force a re-do of Phase 1 if these turn out wrong.

## Reconnaissance findings (informing the decisions)

Verified against the live codebase before locking:

1. **`nova-snark = "0.68"` is the only Nova dep.** No HyperNova, no arkworks-nova, no bellpepper. `evaporchain-proving::nova.rs` uses `nova_snark::frontend::ConstraintSystem`, `nova_snark::nova::{CompressedSNARK, PublicParams, RecursiveSNARK}`, and the `Bn256EngineKZG` / `GrumpkinEngine` cycle pair. Switching to HyperNova would mean a new dep + full circuit rewrite — not justified.

2. **Per-object decay gadget at `nova.rs:1027-1056` is the reusable pattern.** The 5-equation shape:
   - `(a) after_halvings * shift_factor = old_energy - shift_remainder` (integer-division step)
   - `(b) new_energy + frac_decay = after_halvings` (frac correction)
   - `(c) after_halvings * remainder_epochs = product_ar` (intermediate)
   - `(d) frac_decay * two_half_life = product_ar - frac_remainder` (frac formula)
   - `(e) is_evaporated * new_energy = 0` (evaporation gate, not needed for chain total)
   
   Reusing this for the chain-aggregate energy fold takes ~4 `cs.enforce` calls (we drop equation `(e)` since chain-total never gates on a per-block evaporation flag) plus 2 `range_check_bits` for the new witness fields. **Estimated ~70-100 constraints**, well under the 500-constraint estimate in the plan.

3. **State_root limb decomposition exists but binds incompletely.** `nova.rs:~1280` already does a 4-limb decomp with `range_check_bits(64)` per limb and a `full_root` recomposition. **But:** only `limb[0]` (the truncated u64) is bound to `z[0]` (the IVC state). `limb[1..3]` are per-step witness only. **An adversary can vary `limb[1..3]` while keeping `limb[0]` fixed → produce two distinct `full_root`s agreeing on `z[0]` → swap which 32-byte state root the IVC commits to.** This is the 192-bit collision risk the audit flagged.

4. **`evaporchain-mera` is dead** (per the 2026-05-03 VERKLE verdict locked in commit `2053a86`). State commitments come from Energy-Verkle Trie (already in `evaporchain-state`). Anything Lambda-Fold needs to bind that's "the chain's state commitment" should bind the Verkle root, not anything MERA-shaped.

---

## Decisions locked

### Decision 1 — Nova (not HyperNova). LOCKED.

**Rationale:** Single existing dep is `nova-snark = "0.68"`. Energy-fold gadget fits R1CS cleanly (no need for CCS or multifold). The HyperNova "decade-defining" wording in `lambda-fold/src/lib.rs:9` predates the Nova-only architecture and will be cleaned up in Phase 7 docs.

**Side effect for Phase 2+:** all witness types use `G::Scalar` (BN256 scalars, ~254 bits). All gadgets composed from `nova_snark::frontend::ConstraintSystem` ops.

**Side effect for Phase 7 docs:** drop "HyperNova" from `lambda-fold/src/lib.rs:9` and `INVENTION_STACK.md §4.1 row 8`.

### Decision 2 — Single `u128` field element for `total_energy_remaining`. LOCKED.

**Choice:** store `total_energy_remaining` as a single `AllocatedNum<G::Scalar>` field element. Value semantically u128 but lives in a 254-bit field. Range-check via `range_check_bits(128)` — costs ~130 R1CS constraints (each bit is a constraint).

**Rationale:** The 2-limb alternative would mean splitting at every read/write, doubling the gadget surface for negligible benefit. Single field element is simpler, the constraint cost is small, and the existing per-object decay gadget already operates on single field elements (its `old_energy`, `new_energy` are `AllocatedNum` instances — same shape).

**Side effect:** witness type `RealBlockWitness` (Phase 2.1) gains a single `prev_total_energy: u128` field, not a tuple.

### Decision 3 — IVC z-vector layout: arity bumps from 6 → 8 (NOT 7). LOCKED.

**Old z (arity 6):** `[state_hash_truncated, mmr_root_truncated, epoch, block_number, note_tree_root_truncated, pool_balance]`

**New z (arity 8):**
```
z[0] = state_root_poseidon_hash           // NEW: Poseidon(4 limbs of full state root)
                                           // replaces state_hash_truncated; closes the
                                           // 192-bit collision risk
z[1] = mmr_root_truncated                  // unchanged
z[2] = epoch                               // unchanged
z[3] = block_number                        // unchanged
z[4] = note_tree_root_truncated            // unchanged
z[5] = pool_balance                        // unchanged
z[6] = total_energy_remaining              // NEW (Lambda-Fold core)
z[7] = step_count_or_anchor_epoch          // NEW (light-client convenience: tells the
                                           // verifier how many steps were folded
                                           // without re-deriving from the SNARK)
```

**Why arity 8 not 7:** arity-7 would either (a) skip the state_root collision fix to a later phase, or (b) fold step_count into another slot. (a) splits a security-grade change across phases (bad); (b) buries a useful light-client value. Arity 8 is the cleanest IVC contract.

**Why Poseidon for z[0]:** see Decision 4.

**Cost in Phase 2:** ~250 R1CS constraints for the Poseidon hash gadget (over 4 field elements), plus the energy-fold gadget (~100), plus range checks (~130 for u128, ~64 for step_count) = **~544 new constraints**, putting Phase 2.6's expected `pp.num_constraints()` at **~14,585 primary** (was 14,041). Comfortably under the 30,000 stopping-condition threshold.

### Decision 4 — Bind state root via Poseidon hash in `z[0]`. LOCKED.

**Choice:** replace the existing `z[0] = limb[0]` (truncated u64) with `z[0] = Poseidon(limb[0], limb[1], limb[2], limb[3])`. The 4 limbs themselves stay as per-step witnesses with the existing `range_check_bits(64)` constraints; the new piece is one Poseidon hash gadget that binds them all into a single field element going into IVC state.

**Rationale:** Three options were on the table:

- **(a) Extend `z` with all 4 limbs (arity → 11+).** Heavy, complicates light-client interop, bloats the public IO format.
- **(b) Pack full root into one BN256 scalar (range check 254 bits, 2 bits unused).** Loses cleanliness — the BN256 modulus isn't exactly 2^254, so range_check_bits(254) leaves a small interval that's technically allowed but represents an invalid root. Edge case to defend against forever.
- **(c) Poseidon hash of the 4 limbs.** Constant-size single field element, ~250 constraints for the hash, no edge cases. Inherits Poseidon's collision resistance (same primitive used elsewhere in the chain — Verkle commitments, MMR leaves). **Picked.**

**Side benefit:** This implements **Cut E** from `research/proposals/smaller-ivc-circuit.md` (the "Poseidon-bind state root" research item that was tabled as research-track until the invention-stack work touched the state commitment). Lambda-Fold gives us a forced reason to ship it.

**Cost:** ~250 R1CS constraints. Already counted in Decision 3's budget.

**Light-client impact:** The light-client protocol changes. Where today the light client reads `z[0]` as `u64` and compares against the truncated header bytes, tomorrow it reads `z[0]` as a Poseidon hash and compares against `Poseidon(header.state_root_limbs)`. Spec change documented in Phase 5 of the build plan.

---

## Implications for Phase 2

Phase 2's sub-tasks update as follows:

- **2.1 Witness shape:** `RealBlockWitness` gains:
  - `state_root_full: [u64; 4]` (the 4 limbs — already exists in some form per `nova.rs:~1280` reconnaissance; verify exact field name in Phase 2)
  - `prev_total_energy: u128`
  - `step_energy: u64`
  - `epochs_elapsed_at_step: u64`

- **2.2 `arity()` change:** 6 → 8 (was 6 → 7 in the plan; corrected by Decision 3).

- **2.3 z-vector binding:** in `synthesize`:
  - Compute `state_root_poseidon = poseidon(limb_vars[0..4])` and bind to `z_new[0]`.
  - Drop the existing `z_new[0] = new_state_hash_truncated` constraint (the limb[0] equality at `nova.rs:~1297` becomes redundant — the Poseidon hash binds all 4 limbs together).
  - Bind `z_new[6] = compute_decayed_plus_step(...)` for total_energy_remaining.
  - Bind `z_new[7] = z_old[7] + 1` for step_count.

- **2.4 Range checks:** `range_check_bits(128)` on the new total_energy AllocatedNum (Decision 2). The 4-limb 64-bit range checks already exist at `nova.rs:~1289`.

- **2.5 STATE_ROOT limb fix:** completed via Decision 4's Poseidon binding (no separate sub-task).

- **2.6 Constraint count:** target ~14,585 primary (was 14,041; +544 new). Stopping condition: 30,000.

- **2.7 Existing test guard:** all `RealBlockCircuit` tests need `z0` extended from arity 6 to arity 8. Field defaults: `z0 = [poseidon_hash_of_genesis_root_limbs, 0, 0, 0, 0, 0, initial_total_energy_at_genesis, 0]`.

---

## Open questions deferred to Phase 2

These are sub-decisions inside the locked design — not big enough to gate Phase 1 but worth documenting:

1. **Poseidon parameters:** which Poseidon-128 instantiation? `nova-snark` ships one in `nova_snark::provider::poseidon`. Phase 2 will use that to avoid adding a new dep. If gas-cost or audit concerns push us elsewhere later, swap is local to the gadget.

2. **`step_count` overflow:** `z[7]` is a single field element holding `u64`-range step counts. Range check `bits=64` keeps it bounded; at chain rates of ~1 block / 2s, u64 saturates after ~10^11 years. No overflow risk in practice, but the range check defends against witness manipulation.

3. **Initial total energy at genesis:** what value seeds `z0[6]`? Two reasonable picks:
   - **Sum of all account+stake balances at genesis** — captures total chain energy budget at t=0.
   - **Zero** — Lambda-Fold's "energy folded so far," starting at zero and accumulating.
   
   Default: **zero**. The "total energy remaining" is the cumulative sum of `step_energy` minus cumulative decay; starting from zero matches the semantic of "this much energy has been observed by the IVC." Phase 2 nails this when the witness format is finalized.

4. **Backward compat for `lambda_fold_mode = "hash_chain"`:** the existing blake3 substrate stays for the soak window. When the chain runs with `lambda_fold_mode = "hash_chain"`, the new IVC slots are unused but the `FoldedInstance` carries them as `Option<...>`. Phase 5 ships the dual-mode dispatch.

---

## Acceptance for Phase 1

This file's existence + commit is Phase 1's deliverable. Phase 2 starts when:

1. This file is committed.
2. The four locked decisions above are not contradicted by any code change between now and Phase 2 start.
3. Cross-checked against `DOCTRINE_PUNCH_LIST.md` and `LAMBDA_FOLD_NOVA_PLAN.md` for consistency — done in this commit.

Phase 2 next session.
