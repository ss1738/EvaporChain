# Section 3 RelaxedR1CS satisfiability — port approach spike

**Status:** research note (spike). No code lands in this PR. Mirrors
the structure of `SECTION_2_SPONGE_FRAMING.md` — characterise the
gap, evaluate options, pick one, slice the work.

**Goal:** close the Section 3 TODO at
`verifier_circuit.rs::generate_constraints` (originally documented
as "BESPOKE, ~3-5 days research"). Once this lands, `NovaVerifierCircuit`
enforces the three R1CS-satisfiability checks that close cryptographic
soundness against an adversarial prover.

## What the Section 3 TODO actually requires

`RecursiveSNARK::verify` (`nova-snark-0.68/src/nova/mod.rs:634-665`)
runs three checks in parallel via `rayon::join`:

```rust
r1cs_shape_primary.is_sat_relaxed(ck_primary, &r_U_primary, &r_W_primary)
r1cs_shape_secondary.is_sat_relaxed(ck_secondary, &r_U_secondary, &r_W_secondary)
r1cs_shape_secondary.is_sat(ck_secondary, &l_u_secondary, &l_w_secondary)
```

Each call expands to two sub-checks per `r1cs/mod.rs:447-540`:

### `is_sat_relaxed(ck, U, W)` (lines 447-491)

1. **Sparse-R1CS satisfaction.** Build `z = [W.W ‖ [U.u] ‖ U.X]`,
   compute `Az = A·z`, `Bz = B·z`, `Cz = C·z` via sparse-matrix
   multiplication, then check:
   ```text
   ∀ i ∈ [0, num_cons): Az[i] * Bz[i] == U.u * Cz[i] + W.E[i]
   ```

2. **Commitment opening verification.** Recompute Pedersen
   commitments and compare to the witnessed commitments:
   ```text
   U.comm_W == CE::commit(ck, &W.W, &W.r_W)
   U.comm_E == CE::commit(ck, &W.E, &W.r_E)
   ```

### `is_sat(ck, U, W)` (lines 493-535)

Identical to `is_sat_relaxed` except `u = 1` (constant), no `E`
slack vector, and only one commitment to check (`comm_W`).

## Precise cost contributors at the chain's parameters

Realistic numbers per `r1cs_shape_primary` (the larger side, since
the secondary R1CS is the constant-size cyclefold-style mini-shape):

| Quantity | Typical | Cost impact |
|---|---|---|
| `num_cons` (R1CS constraint rows) | ~30k–100k for a chain-block step circuit | Each row's check = 1 mult + 1 add ≥ 1 R1CS constraint |
| `num_vars` (witness length) | ~30k–100k | Commitment is Pedersen on ~num_vars scalars × generator points |
| Sparse `A`/`B`/`C` non-zero density | ~3-10 per row | SpMV cost per row ≈ 3-10 mults + adds |
| Commitment scheme | Pedersen on BN254 G1 (primary) / Grumpkin G1 (secondary) | Each commitment opening = num_vars-point fixed-base scalar mults |

Naive in-circuit cost:
- **R1CS row check** (3 SpMV dot products + 1 mult + 1 add per row): ~5–15 R1CS constraints per row × 100k rows = **1.5M constraints** for row checks alone, across the three checks.
- **Commitment verification** (one scalar-mult per generator): ~10 R1CS constraints per scalar-mult × 100k scalars × 3 commitments = **3M constraints** for commitments alone, naively.

Total naive: **4–5M R1CS constraints** in the verifier circuit.
With the 2^18 = 262k Powers-of-Tau ceremony budget the project
targeted in `lib.rs` (the line *"~80× over the 2^18 PoT ceremony
budget"*), naive is **15–20× over budget**. Not viable.

## The non-native arithmetic problem

Pedersen commitments live on the engine's curve (BN254 G1 for
primary, Grumpkin G1 for secondary). The verifier circuit runs
over `ark_bn254::Fr` (BN254 scalar field). To verify a BN254 G1
scalar-mult inside BN254 Fr R1CS, you need **non-native
arithmetic over BN254 Fq** (the base field of the curve, which is
also Grumpkin's scalar field). This costs ~80× more constraints
per scalar-mult than native — the original blocker that motivated
the Path-A pivot.

For the **secondary** side this is partially relieved because
Grumpkin's scalar field IS BN254's base field, and BN254/Grumpkin
form a cycle. Using CycleFold-style delegation, secondary
commitment checks can be deferred to the OTHER side of the cycle.
But the verifier circuit at the end of Nova-folding still has to
verify SOMETHING for both sides.

## Survey of approaches

### Option A — Naïve direct in-circuit verification

Encode all three `is_sat`/`is_sat_relaxed` checks directly in
R1CS: explicit sparse SpMV, explicit Pedersen scalar-mult
verification on every commitment, explicit per-row check.

**Pros:** Conceptually simple. Matches `nova-snark`'s `verify`
source line-by-line. Soundness is obvious.

**Cons:** 4–5M constraints (see above). 15–20× over budget. Single
proof generation would take minutes on a strong machine.
**Rejected.**

### Option B — Sumcheck-protocol-replay

Replace the explicit per-row check
`∀ i: Az[i] · Bz[i] == U.u · Cz[i] + W.E[i]`
with a single random-challenge equality:
```text
sum_i τ^i · (Az[i] · Bz[i] - U.u · Cz[i] - W.E[i]) == 0
```
where `τ` is a Fiat-Shamir challenge. Use sumcheck to reduce this
to a single evaluation at a random point.

**Pros:** O(log num_cons) verifier cost instead of O(num_cons).
Per the constraint estimate: ~17 round messages × ~200 constraints
each = ~3.4k constraints for the row check (vs 1.5M naively).
**~440× cheaper** for the row-check half.

**Cons:** Per-row check shrinks dramatically, but commitment
verification doesn't. Sumcheck doesn't help with the Pedersen
opening — that still needs ~3M constraints. Total improvement is
~30%, not ~99%. Not enough.

### Option C — Hyrax / Spartan-style commitment + sumcheck

Replace Pedersen entirely with a polynomial-commitment scheme
whose opening is cheap to verify in R1CS (e.g., KZG on BN254 G1
for the primary side). Pair with sumcheck for the row check.

**Pros:** KZG opening is ~10 constraints (a single pairing-style
check delegated to the EIP-197 precompile at the L1 layer, or
expressed as a pairing-friendly check in-circuit). Total verifier
cost drops to ~50k constraints — well under the 262k PoT budget.

**Cons:** Requires REPLACING nova-snark's Pedersen with KZG. Two
sub-paths:
- (C1) Fork nova-snark and swap the commitment scheme. Heavy.
- (C2) Use `HyperNova` / `Mova` / `Nebula` — newer schemes that
  natively use KZG-friendly commitments. Requires migrating the
  chain's `evaporchain-proving` crate off vanilla Nova. Largest
  scope, but unblocks Section 3 cleanly.

### Option D — Deferred commitment check (relayer-aided)

The bridge crate's `INEVITABILITY_STRATEGY` sub-paths analysis
(per `lib.rs` module doc — *"A1 = raw RecursiveSNARK, A2 =
CompressedSNARK, A3 = re-prove via relayer"*) flags **A3** as the
recommended path. The relayer pattern:
1. Off-chain relayer takes the `RecursiveSNARK`, runs
   `is_sat_relaxed` natively (cheap), proves to itself it's
   satisfied.
2. Relayer's proof is a Groth16 over a MUCH SMALLER circuit that
   doesn't have to do the full Pedersen verification in-circuit —
   instead it commits to the relayer's claim and signs it.
3. L1 verifies the relayer's much smaller proof.

**Pros:** Minimal in-circuit work. Verifier circuit shrinks to
~10k constraints (just the relayer's claim + signature).
Aligns with the bridge crate's documented A3 recommendation.

**Cons:** Trust the relayer (they could lie about
`is_sat_relaxed` returning true). Mitigation: open relayer set,
slashable bond, or zk-SNARK on the relayer's local verify (which
recursively pushes the cost back to Option A or C).

### Option E — CycleFold for secondary side, native for primary

CycleFold (Kothapalli & Setty 2023) lets the secondary-side
checks be delegated through the BN254/Grumpkin cycle without
non-native arithmetic. The primary side is verified natively in
BN254 R1CS.

**Pros:** No non-native arithmetic. Used in real production
systems (e.g., Sonobe). Mid-complexity.

**Cons:** Requires adding a CycleFold orchestration layer to
`evaporchain-proving::nova`. New cross-crate refactor.
Sumcheck for the row check still needed. Total ~50k constraints
plus cycle-fold orchestration.

## Recommendation

**Option D (relayer-aided, the A3 path)** as the actual ship
target — it matches the crate's own architectural recommendation
in `lib.rs` and `DESIGN.md`. Section 3's role becomes: prove
in-circuit that the relayer's claim + signature is consistent
with the public inputs and the Nova accumulator state. Not "redo
all of `is_sat_relaxed` inside R1CS".

This collapses Section 3 from a 3-5 day BESPOKE research
deliverable into ~5 PRs of orchestration work:

### First-PR slice (Section 3 slice 0)

1. **Relayer protocol spec.** `docs/RELAYER_PROTOCOL.md` —
   message format, signature scheme, slashing condition,
   set-membership rules.
2. **One reference type** in `evaporchain-nova-bridge`:
   `RelayerClaim` carrying `(rs_digest, relayer_pubkey, sig,
   claim_timestamp)`. Cargo-clean compile, no logic yet.

### Slices 1-3

1. **Slice 1: in-circuit signature verification.** Use an existing
   gadget (e.g. arkworks's EdDSA or Schnorr on Jubjub for BN254
   compatibility). ~10k constraints. Tests against a hand-signed
   claim.
2. **Slice 2: relayer-set membership.** Merkle-tree membership
   proof for the relayer's pubkey in a config-time relayer set.
   ~2k constraints. Tests against a hand-built tree.
3. **Slice 3: claim-binding.** Bind the relayer's claim to the
   Nova accumulator's actual digest via the bridge's existing
   `committed_hash_primary` + `committed_hash_secondary` public
   inputs.

### Slice 4

**Wire into `verifier_circuit::generate_constraints`** — replace
the Section 3 TODO with the actual constraint emission. Bridge's
public-input ordering grows by 2-3 entries (relayer pubkey
commitment, claim signature components).

### Slice 5

**Update `circuit_builder::build_circuit_from_fixture`** to fetch
+ embed a real relayer claim. Test end-to-end with the existing
`fixture-proof-emit` binary path.

## What this spike does NOT close

- Section 2 sponge framing remains a separate iteration loop
  (PRs #183, #187 ongoing).
- The `l_u_secondary` private-field workaround (PR #151's
  serde-reflection) is unaffected; relayer claim doesn't need
  access to `r_W_primary` / `r_W_secondary` (those go to the
  relayer, not the L1 verifier).
- The trust model shifts from "purely cryptographic" to
  "relayer-bonded + slashing". This is documented explicitly in
  `INEVITABILITY_STRATEGY.md` ("A3 = re-prove via relayer" as
  *recommended* path) and is acceptable for the bridge's
  mainnet milestone. Pure-cryptographic Section 3 (Option C2,
  HyperNova migration) is the V1.5 upgrade path, not the V1
  ship.

## Estimated work

| Slice | LOC | Estimated PR count |
|---|---|---|
| 0 (this spike) | doc only | 1 (this PR) |
| 1 (signature gadget) | ~200 LOC + ~150 LOC tests | 1 |
| 2 (Merkle membership) | ~150 LOC + ~100 LOC tests | 1 |
| 3 (claim binding) | ~100 LOC + ~150 LOC tests | 1 |
| 4 (wire into verifier_circuit) | ~50 LOC + ~50 LOC tests | 1 |
| 5 (circuit_builder integration) | ~80 LOC + integration test | 1 |
| **Total** | **~1,000 LOC** | **6 PRs** |

vs Option A's ~5,000+ LOC of bespoke sparse-SpMV-in-R1CS
gadgetry. The A3 path is ~5× less code and ~50× fewer
constraints.

## Source

- `nova-snark-0.68/src/r1cs/mod.rs:447-540` (`is_sat`,
  `is_sat_relaxed` definitions)
- `nova-snark-0.68/src/nova/mod.rs:634-665` (the three checks in
  `verify`)
- `crates/evaporchain-nova-bridge/src/lib.rs` module docstring
  (A1/A2/**A3** sub-path analysis)
- `crates/evaporchain-nova-bridge/src/verifier_circuit.rs` (the
  Section 3 TODO this spike addresses)
- `research/INEVITABILITY_STRATEGY.md` (recommends "open
  protocol" path; relayer model is compatible)
- Kothapalli & Setty 2023 — CycleFold (Option E reference, not
  chosen)
- Kothapalli, Setty & Tzialla 2022 — Nova (the scheme being
  verified)
