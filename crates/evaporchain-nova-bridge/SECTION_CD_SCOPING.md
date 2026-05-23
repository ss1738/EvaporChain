# Sections C + D — Scoping Document (NO-OP COLLAPSE)

**Date:** 2026-05-20 · **Branch:** `s4-grumpkin-config` · **Parents:**
[`B1_B2_AUDIT_DOSSIER.md`](B1_B2_AUDIT_DOSSIER.md) §6b (trust-model
decision) + §7 step 2 + [`SECTION_B_SCOPING.md`](SECTION_B_SCOPING.md)
(delegation pattern).

## 1. Finding (source-grounded)

`CompressedSNARK::verify` (nova-snark 0.68
`src/nova/mod.rs::909-1025`) after Section B's hash checks runs:

```rust
// ── Section C: NIFS folds + derandomize ────────────────────
let r_Uf_secondary = self.nifs_Uf_secondary.verify(
    &vk.ro_consts_secondary,
    &scalar_as_base::<E1>(vk.pp_digest),
    &self.r_U_secondary,
    &self.l_u_secondary,
)?;
let r_Un_secondary = self.nifs_Un_secondary.verify(
    &vk.ro_consts_secondary,
    &scalar_as_base::<E1>(vk.pp_digest),
    &r_Uf_secondary,
    &self.l_ur_secondary,
)?;
let r_Un_primary = self.nifs_Un_primary.verify(
    &vk.ro_consts_primary,
    &vk.pp_digest,
    &self.r_U_primary,
    &self.l_ur_primary,
)?;
let derandom_r_Un_primary = r_Un_primary.derandomize(
    &vk.dk_primary,
    &self.wit_blind_r_Wn_primary,
    &self.err_blind_r_Wn_primary,
);
let derandom_r_Un_secondary = r_Un_secondary.derandomize(...);

// ── Section D: final SNARKs (primary HyperKZG + secondary IPA) ──
let (res_primary, res_secondary) = rayon::join(
    || self.snark_primary.verify(&vk.vk_primary, &derandom_r_Un_primary),
    || self.snark_secondary.verify(&vk.vk_secondary, &derandom_r_Un_secondary),
);
res_primary?;
res_secondary?;

Ok(self.zn.clone())  // ← THE ONLY EXTERNAL RETURN
```

**Every intermediate value is consumed internally.** `r_Uf`,
`r_Un_*`, `derandom_r_Un_*`, `res_primary`, `res_secondary` are
all local bindings that feed the next step or short-circuit on
error via `?`. Only `self.zn` escapes.

## 2. Implication for the delegation model

Per dossier §6b: on-chain Groth16 binds only Section A's MSM; the
off-chain adapter is the binding gate for everything else. Since
Sections C and D produce no externally-observable values that need
binding, **Sections C and D collapse to NO-OP in the delegation
model.** The off-chain adapter's `CompressedSNARK::verify` call
runs all the C/D checks internally; if it accepts, every Section
C/D gate has passed.

**No additional public inputs.** No additional in-circuit
allocation. No additional Foundry tests beyond Section A + B.

## 3. What this means for the dossier §7 step 2

The rev-1 plan had three more iterations (Sections C, D each with
adapter + end-to-end + EVM test). Per this analysis, the actual
remaining work for "Sections C + D" is:

1. **Upgrade the adapter from `RecursiveSNARK::verify` to
   `CompressedSNARK::verify`.** The current
   `assemble_section_b_pi_bundle` calls `RecursiveSNARK::verify`,
   which does NOT include Sections C/D (RecursiveSNARK is the
   pre-compression form). The mainnet path uses CompressedSNARK
   — its `::verify` is what runs Sections C/D internally.
2. **No new struct fields, no new PIs, no new circuit changes,
   no new Foundry tests.** Sections C and D are fully covered by
   the upgraded adapter.

## 4. Adapter upgrade scope

A new function `assemble_section_b_pi_bundle_from_compressed_snark`:

```rust
pub fn assemble_section_b_pi_bundle_from_compressed_snark(
    pp: &PublicParams<E1, E2, TrivialIncrementCircuit>,
    vk_compressed: &CompressedVerifierKey<E1, E2, ..., S1, S2>,
    cs: &CompressedSNARK<E1, E2, TrivialIncrementCircuit, S1, S2>,
    num_steps: usize,
    z0_ark: &[ArkFr],
) -> Result<SectionBPiBundle, ExtractError> {
    // 1. Verify (the binding gate — now includes Sections C/D).
    let zn_nova = cs.verify(vk_compressed, num_steps, &z0_nova)?;

    // 2. pp_digest from pp.
    let pp_digest_ark = primary_to_ark_fr(pp.digest());

    // 3. Extract PIs from compressed-snark JSON (same JSON shape
    //    as RecursiveSNARK for the fields we care about — both
    //    are R1CSInstance / RelaxedR1CSInstance with the same
    //    serde derives).

    // 4. Overlay verified zn.
}
```

The CompressedSNARK setup is heavy (~125s on a Mini for the
canonical test circuit, per the earlier `compressed_snark_*`
validation runs). The integration test will be #[ignore]'d or
moved to satyawan-1.

## 5. Cost picture (revised)

Per dossier §6b's table, "HYBRID (chosen): ~41.5M cons, ~1 week
B+C+D adapters". With Section C/D collapsing to no-op:

| Phase | Status |
|---|---|
| Section A in-circuit (~41.5M cons) | ✅ DONE ((d)-1/2/3/4) |
| Section B off-chain adapter | ✅ DONE (Rust + EVM) |
| Section B end-to-end (smoke + EVM 11 PIs) | ✅ DONE |
| Section C (NIFS folds) | **NO-OP** (covered by CompressedSNARK::verify) |
| Section D (HyperKZG pairing) | **NO-OP** (covered by CompressedSNARK::verify) |
| **CompressedSNARK adapter variant** | **PENDING** (mostly mechanical) |
| External audit on pinned revision | PENDING |

The "~1 week" estimate for B+C+D was conservative; the actual
remaining engineering is just the CompressedSNARK adapter variant
(~1-2 days including heavy test scheduling on satyawan-1).

## 6. Audit narrative implication

For external audit, the Section C/D no-op finding is a clean
delegation-model argument:

- **Claim:** Sections C and D are covered by the off-chain
  `CompressedSNARK::verify` call inside the adapter.
- **Evidence:** source mapping above; `CompressedSNARK::verify`
  runs all NIFS / derandomize / spartan-verify gates with `?`
  short-circuit on error.
- **Falsifier:** if a malicious prover could construct a
  CompressedSNARK that passes adapter verify but has a bad NIFS
  fold or bad spartan proof, the trust model breaks. This
  reduces to "is `CompressedSNARK::verify` sound?" — which is
  the cryptographic responsibility of nova-snark 0.68 (well-
  trodden, ecosystem-validated, and orthogonal to our 1C work).

For the audit dossier: the soundness argument is "Section A
in-circuit + Section B PI delegation + CompressedSNARK::verify
covers C/D = full delegation chain." The cryptographic core is
nova-snark's library correctness; our work is the in-circuit
Section A binding + the PI delegation glue.

## 7. Why this scoping doc IS the iteration

Per the framing-error lesson pattern: surface architectural
findings BEFORE iterating on the assumption. The rev-1 dossier
implied Sections C and D each needed their own adapter +
end-to-end + EVM test — that would have been 6+ iterations of
near-duplicate work that produced no soundness gain.

Catching the no-op collapse at scoping level: saves ~4-5 days
of duplicate-iteration work, AND tightens the audit narrative
(fewer moving parts means easier soundness review).

The discipline pattern continues to hold:
1. Don't iterate without re-reading source for the next step.
2. Don't assume "follows the same pattern" without checking
   whether the pattern applies.
3. Surface findings in dossier docs immediately so future
   iterations don't repeat the assumption.
