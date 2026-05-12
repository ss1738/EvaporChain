# evaporchain-nova-bridge

T0.10 Path A — chain-side Nova accumulator → L1 Groth16-on-BN254 verifier.

See `DESIGN.md` for the architecture rationale (why Path A: Nova folding
outside Groth16 + verifier circuit inside, vs the legacy IPA-in-Groth16
approach the `ethereum-bridge/circuits` crate uses).

## Modules

| Module | Purpose |
|---|---|
| `verifier_circuit` | `NovaVerifierCircuit: ConstraintSynthesizer<Bn254Fr>` + `StructuralValidationError` (off-circuit precondition gate). Phase 2.2 skeleton + Section 1. |
| `recursive_snark_fixture` | `generate_fixture(num_steps)` produces a real `RecursiveSNARK<Bn256EngineKZG, GrumpkinEngine, TrivialIncrementCircuit>` for the verifier circuit to consume. |
| `groth16_wrapper` | `setup` / `prove` / `verify` / `prepare_vk` / `verify_prepared` against `NovaVerifierCircuit`. Plus `public_inputs_in_alloc_order` (the load-bearing public-input ordering contract). |
| `canonical_io` | Canonical `ark-serialize` compressed bytes for `pk`, `vk`, `proof`. Persistence layer for operator pipelines. |
| `eip197` | 256-byte EIP-197 wire-format conversion of `Proof<Bn254>` for the L1 pairing precompile. **Includes the `Fq2 (c1, c0)` swap.** |
| `scalar_adapter` | `nova↔arkworks` scalar conversion (`bn256::Fr ↔ ark_bn254::Fr`, `grumpkin::Fr ↔ ark_bn254::Fq`). Used by all downstream gadget paths. |
| `poseidon_transcript` | Typed `TranscriptSlot` + `absorb_order(side, z_arity)` builder pinning nova-snark's exact absorb sequence from `RecursiveSNARK::verify`. |
| `poseidon_budget` | Empirical Poseidon constraint-cost probe (arkworks-default). |
| `section2_gadget` | Section 2 (in-circuit Poseidon) gadget. `placeholder_poseidon_config` + `neptune_aligned_poseidon_config` + `fully_aligned_poseidon_config`. |
| `neptune_reference` | `neptune_hash_primary(&[Scalar]) -> Scalar` — calls nova-snark's `PoseidonRO` directly for ground-truth oracle hashes. |
| `neptune_dump_parser` | JSON parser for `dump-neptune-constants`'s output. Extracts `mds.{m, m_inv, m_hat, m_hat_inv, m_prime, m_double_prime}`, `crc`, `rf`, `rp`. |
| `grain_lfsr` | Port of neptune's Grain-80 LFSR for Poseidon round-constant generation. **Byte-correct against neptune `round_keys(0)` per PR #97.** |
| `vendored_neptune_grain` | Verbatim copy of neptune's `round_constants.rs` algorithm. Used for differential debugging. |
| `mds_linalg` | `left_apply_matrix`, `vec_add`, `matrix_mul`, `identity_matrix` over `ark_bn254::Fr`. |
| `compress_ark` | Port of neptune's `compress_round_constants`. **Byte-correct against neptune `crc[0..259]` per PR #103.** |

## Binaries

| Binary | Purpose |
|---|---|
| `setup-keys` | Trusted-setup CLI: runs `Groth16::circuit_specific_setup` with OS randomness; writes `pk.bin` (~1.4 KB) + `vk.bin` (~400 B); smoke-tests by verifying a fresh proof. |
| `prove-and-verify` | Companion: loads pk + vk, builds a circuit, proves under fresh randomness, verifies, writes `proof.bin` (128 B canonical compressed). Optional `--fixture-out path.json` emits Solidity-test-vector JSON. |
| `dump-neptune-constants` | Extracts neptune's `PoseidonConstants::<bn256::Scalar>::default()` to JSON via serde (PR #80). |
| `dump-our-compressed-ark` | Emits our `compress_full` output in `{ "crc": [...] }` JSON shape (matches neptune dump for `diff`-based audit). |
| `check-neptune-parity` | Single-shot CI gate: exit 0 on `259/259 crc entries match byte-for-byte`, exit 1 with diagnostic on mismatch. |

## Section 2 byte-parity quick-check

```bash
# 1. Extract neptune's actual constants
cargo run -p evaporchain-nova-bridge --bin dump-neptune-constants -- \
    --out /tmp/neptune.json

# 2. Verify our impl matches
cargo run -p evaporchain-nova-bridge --bin check-neptune-parity -- \
    --neptune /tmp/neptune.json
# → PASS — 259 of 259 crc entries match byte-for-byte
```

## Current scaffold status

`SCAFFOLD_VERSION = "phase-2.2-section-2-constants"` (PR #104).

- ✅ `phase-2.2-starter` — fixture generator (PR #55)
- ✅ `phase-2.2-skeleton` — verifier circuit skeleton + public-input wiring (PR #56)
- ✅ `phase-2.2-section-1` — off-circuit structural-validation gate (PR #64)
- ✅ `phase-2.2-section-2-constants` — neptune compressed-ARK byte-complete (PR #103)
- ⬜ `phase-2.2-section-2` — full hash byte-parity (residual sponge framing port)
- ⬜ `phase-2.2-section-3` — RelaxedR1CS in-circuit (BESPOKE)
- ⬜ `phase-2.2-complete` — all sections wired through

The remaining gap to `phase-2.2-section-2` is the per-round operation
in arkworks `PoseidonSpongeVar` vs neptune's `Poseidon::hash_optimized_static`
(which uses SBOX-trick-fused partial rounds). PR #98's parity canary
captures the residual hash divergence; PR #100's analysis narrows it to
sponge framing.
