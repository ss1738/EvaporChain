# Section 2 sponge framing — port approach spike

**Status:** research note (spike). No code lands in this PR. The
following PRs in the Section 2 sponge-framing arc will implement
the approach recommended below.

**Goal:** close the `assert_ne!` canary at
`section2_gadget::tests::fully_aligned_gadget_byte_parity_with_neptune`.

## What the canary actually documents

The canary takes:

- our compressed-ARK (byte-correct vs neptune `crc[0..259]` per
  the constants substrate landed in #130–#142, #138),
- neptune's real MDS (extracted via `neptune_dump_parser`),
- the input absorb sequence `[pp.digest, num_steps, z0[0], zi[0],
  ri_primary]`,

feeds them into both **arkworks's `PoseidonSpongeVar`** (with a
`PoseidonConfig` whose `ark`/`mds` fields are the neptune-byte-
correct constants and matrix) and **neptune's
`Poseidon::hash_optimized_static`** (via the
`crate::neptune_reference` oracle), and asserts the 32-byte LE
output reprs **differ**. They differ because the two
permutations are mathematically equivalent but
**operationally inequivalent** under the substituted constants.

## Precise gap (read from source on 2026-05-12)

| | **arkworks `PoseidonSpongeVar::permute`** (constraints.rs:83–103) | **neptune `Poseidon::hash_optimized_static`** (poseidon_inner.rs:315–340) |
|---|---|---|
| Initial ARK | implicit (first `apply_ark`) | explicit `add_round_constants()` separately |
| Per full round | ARK(plain) → S-Box(full) → MDS(plain) | S-Box-fused(post-key) → MDS(plain OR pre_sparse OR sparse) |
| Per partial round | ARK(plain) → S-Box(state[0] only) → MDS(plain) | S-Box-fused(post-key) on state[0] only → MDS(varies, see below) |
| Final full round | same as other full rounds | special `full_round(true)` — S-Box only, **no post-key, no MDS** |
| Constants source | `pk[N×width]` plain | `crc[N]` compressed (~6.5× fewer entries via SBOX-trick) |
| MDS during partial rounds | same plain MDS every round | `pre_sparse_matrix` at boundary, then `sparse_matrices[i]` per partial round |

### The SBOX-trick fusion

Neptune's `quintic_s_box(l, None, post)`
(`nova-snark-0.68/src/frontend/gadgets/poseidon/mod.rs:67`) does:

```rust
l = l^5      // (5 multiplications via two squarings)
l = l + post // post-round-key folded into S-Box output
```

vs arkworks's `apply_ark` then `apply_s_box`:

```rust
state[i] = state[i] + ark[round][i]   // ARK first
state[i] = state[i]^5                 // S-Box second
```

Mathematically these match if the constants are shifted by one
round. That's exactly what neptune's `compress_round_constants`
does to produce `crc` — it pre-shifts the constants so the
fused tail of round k becomes the head of round k+1.

### Sparse-matrix partial-round fast path

After the first `half_full_rounds`, neptune switches MDS:

- Round `half_full_rounds - 1`: use `pre_sparse_matrix`
- Rounds `half_full_rounds`...`half_full_rounds + partial_rounds - 1`:
  use `sparse_matrices[round - half_full_rounds]`
- Rounds after: back to plain `mds_matrix`

The sparse matrices were factored from `M = M' · M_sparse` to
make the partial-round MDS multiplication O(1) instead of
O(width²). neptune's `factor_to_sparse_matrixes` does the
factoring at construction time; we already extract these
matrices via `neptune_dump_parser::extract_mds_{m_hat, m_hat_inv,
m_prime, m_double_prime}`.

arkworks's `apply_mds` always uses the full matrix — no fast
path. This is **NOT** a security issue (the two are
mathematically equivalent), but it IS why feeding neptune's
compressed constants into arkworks's pure `PoseidonSpongeVar`
yields a different output.

## Why feeding compressed constants into arkworks's gadget can't work

Numerically:

- arkworks reads `ark[round * width + col]` for `round ∈ [0,
  full_rounds + partial_rounds)`. Needs `width × (full + partial)
  = 25 × 67 = 1675` constants.
- Our compressed `crc` has 259 entries (= `full × width + partial`
  = `8 × 25 + 59`).

Arkworks indexes off the end of the array and panics, OR (more
likely) gets fed a padded array and reads the wrong constant
positions — producing a bit-incorrect hash.

The arithmetic ALSO disagrees:

- arkworks: `state[i] := (state[i] + ark) ^ 5`, then MDS.
- neptune: `state[i] := state[i] ^ 5 + post_ark`, then MDS.

These two recurrences match iff the constants are shifted by
exactly one round. Our `crc` IS that shifted form. Feeding
shifted constants into the unshifted recurrence gives the
**wrong hash**, NOT a structurally invalid output. That's why
the canary asserts non-equality silently rather than panicking.

## Port options

### Option A — Write a custom `NeptunePermutationGadget`

**Build a new gadget that mimics `hash_optimized_static` in
arkworks R1CS.** Mirror neptune's structure exactly:

```rust
pub fn neptune_permute<F: PrimeField>(
    cs: ConstraintSystemRef<F>,
    state: &mut [FpVar<F>],
    ark: &[F],                   // compressed crc[0..259]
    pre_sparse: &[Vec<F>],       // 25×25
    sparse_matrices: &[SparseMatrix<F>], // partial_rounds entries
    plain_mds: &[Vec<F>],        // 25×25
    half_full_rounds: usize,     // 4
    partial_rounds: usize,       // 59
) -> Result<(), SynthesisError>
```

- Initial `add_round_constants` step (one ARK, then offset = width).
- For `half_full_rounds` full rounds: for each state cell,
  fp_var.pow(5) then add post-key from `ark[offset+i]`, then
  MDS. Choose MDS matrix per the boundary rule above.
- For `partial_rounds` partial rounds: pow(5) state[0] + add
  post-key, then MDS (sparse).
- For `half_full_rounds - 1` final full rounds: same as initial
  full rounds.
- Final round: pow(5) all cells, NO post-key, NO MDS.

**Pros:** byte-correct hash. The canary becomes `assert_eq!`.

**Cons:** ~200 lines of careful gadget code. Sparse-matrix
encoding needs to choose between (a) materialising as a full
matrix at config time (defeats the constraint savings) or
(b) writing a dedicated sparse-mul gadget. Constraint count
goes up vs arkworks's pure permutation; need to measure.

**First slice for next PR:** scaffold the function signature +
unit-test it against the off-circuit `quintic_s_box` + ARK
output for a single full round on a single state vector. Don't
wire into `section2_gadget` yet.

### Option B — Vendor neptune's permutation as a witness-generator + `gadget` separately

**Run neptune's actual `Poseidon::hash_optimized_static`
off-circuit to compute the witness, then assert in-circuit
that the public-input hash equals that off-circuit value.**

- Off-circuit: `let expected = neptune_hash_primary(...)` (uses
  `crate::neptune_reference`).
- In-circuit: allocate `expected` as a witness, expose as public
  input. The CS check becomes `witness_eq_input(expected_var,
  pp.committed_hash_*)`.

**Pros:** trivial in-circuit gadget. No permutation gadget at
all.

**Cons:** **destroys the soundness purpose of the bridge.** The
whole point of Section 2 is to in-circuit-verify the transcript;
trusting an off-circuit oracle is identical to trusting the
prover's `committed_hash_*` claim. Rejected.

### Option C — Patch arkworks to add a fast-path

**Upstream a fork or PR to `ark-crypto-primitives` adding a
`PoseidonConfig::with_compressed_round_constants` mode.**

**Pros:** widely reusable beyond this crate.

**Cons:** upstream merge timeline; we own the entire patch
maintenance burden until merged. Out of scope for the May–Oct
2026 sprint.

## Recommendation

**Option A.** Custom `NeptunePermutationGadget` written against
arkworks's `FpVar` + `ConstraintSystemRef`, mirroring neptune's
`hash_optimized_static` structure line-by-line. Constraint cost
is acceptable for this verifier circuit (one Section-2 hash per
proof; the bridge proves one Nova accumulator step per chain
advance).

### First-PR slice

1. New file `src/neptune_permutation_gadget.rs`.
2. Single function `enforce_neptune_full_round(cs, state, post_ark, mds) -> Result<(), SynthesisError>`
   that implements one full round: `state[i] = state[i]^5 + post_ark[i]`, then `state := mds · state`.
3. Test against `crate::vendored_neptune_grain` + a hardcoded
   25-element state: synthesise + read witnesses + assert
   bit-equal to the off-circuit `quintic_s_box` + MDS-apply
   result.

No wiring into `section2_gadget` in that PR. Once full-round is
green, follow-up PRs:

- Partial round (with sparse-MDS choice).
- Full permute() function (sequence + boundary handling).
- Section-2 absorb-then-squeeze using the new permutation.
- Replace `section2_gadget::fully_aligned_poseidon_config` callers.
- Flip the canary from `assert_ne!` to `assert_eq!`.

### Estimated work

- Full-round gadget + test: ~1 PR.
- Partial-round gadget + sparse-MDS handling: ~2 PRs (one for
  full-matrix variant, one for sparse-mul gadget if constraint
  count demands it).
- Permute orchestration: ~1 PR.
- Section-2 wire-in + canary flip: ~1 PR.

Total: ~5 PRs, each ~150–250 lines of code, all
bit-correctness-driven against `vendored_neptune_grain` or the
`neptune_reference` oracle.

## What this spike does NOT close

- The **Section 3 RelaxedR1CS** satisfiability gap is orthogonal
  and remains BESPOKE (3–5 days research). Closing Section 2
  alone gets us a sound in-circuit transcript check; Section 3
  is needed for soundness against arbitrary prover-chosen
  `r_W_*` witnesses.

- The **`l_u_secondary` access workaround** (PR #151) is
  unaffected. Upstream-PR-to-nova-snark remains the right
  long-term fix; the serde-reflection extraction is unchanged
  by anything in Section 2.
