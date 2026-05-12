# evaporchain-nova-bridge

T0.10 Path A — chain-side Nova accumulator → L1 Groth16-on-BN254 verifier.

See `DESIGN.md` for the architecture rationale (why Path A: Nova folding
outside Groth16 + verifier circuit inside, vs the legacy IPA-in-Groth16
approach the `ethereum-bridge/circuits` crate uses).

## Modules

| Module | Purpose |
|---|---|
| `verifier_circuit` | `NovaVerifierCircuit: ConstraintSynthesizer<Bn254Fr>` + `StructuralValidationError` (off-circuit precondition gate). Phase 2.2 skeleton + Section 1. |
| `recursive_snark_fixture` | `generate_fixture(num_steps)` produces a real `RecursiveSNARK<Bn256EngineKZG, GrumpkinEngine, TrivialIncrementCircuit>`. |
| `scalar_adapter` | nova↔arkworks scalar conversion: `bn256::Fr ↔ ark_bn254::Fr` (same field, byte-lossless) + `grumpkin::Fr ↔ ark_bn254::Fr` (lossy cross-field LE-bytes-mod-order). |
| `l_u_secondary_extract` | Extracts the two committed hashes `l_u_secondary.X[..2]` from a `RecursiveSNARK` via `serde_json` reflection (nova-snark v0.68's `l_u_secondary` field is private; the JSON path is brittle but pinned by `debug_dump_l_u_secondary_json_shape`). |
| `circuit_builder` | `build_circuit_from_fixture(rs)` — full witness orchestration. Combines `scalar_adapter` + `l_u_secondary_extract` into a populated `NovaVerifierCircuit`. |
| `groth16_wrapper` | `setup` / `prove` / `verify` + `public_inputs_for` (load-bearing slice ordering: hash_primary, hash_secondary, z0[..], zi[..]). |
| `eip197` | 256-byte EIP-197 wire-format conversion of `Proof<Bn254>` for the L1 pairing precompile. **Includes the Fq2 (c1, c0) swap.** |
| `section2_gadget` | Section 2 (in-circuit Poseidon) gadget. `placeholder_poseidon_config` + `neptune_aligned_poseidon_config` + `fully_aligned_poseidon_config`. Sponge-framing canary `assert_ne!` documents the residual BESPOKE gap. |
| `neptune_reference` | `neptune_hash_primary(&[Scalar]) -> Scalar` — calls nova-snark's `PoseidonRO` for ground-truth oracle hashes + pinned reference vectors. |
| `neptune_dump_parser` | JSON parser for `dump-neptune-constants`'s output. Extracts `mds.{m, m_inv, m_hat, m_hat_inv, m_prime, m_double_prime}`, `crc`, `rf`, `rp`. |
| `grain_lfsr` | Port of neptune's Grain-80 LFSR for Poseidon round-constant generation. **Byte-correct against neptune `round_keys(0)`.** |
| `vendored_neptune_grain` | Verbatim copy of neptune's `round_constants.rs` algorithm — regression net for `grain_lfsr` (differential debugging). |
| `mds_linalg` | `left_apply_matrix`, `vec_add`, `matrix_mul`, `identity_matrix` over `ark_bn254::Fr`. |
| `compress_ark` | Port of neptune's `compress_round_constants` SBOX-trick. **Byte-correct against neptune `crc[0..259]`.** |

## Binaries

| Binary | Purpose |
|---|---|
| `dump-neptune-constants` | Extracts neptune's `PoseidonConstants::<bn256::Scalar>::default()` to `/tmp/neptune-bn256-standard.json` via serde. Unblocks 8 `#[ignore]`-gated real-data tests. |
| `dump-our-compressed-ark` | Emits our `compress_full` output in `{ "crc": [...] }` JSON shape (matches neptune dump for `diff`-based audit). |
| `check-neptune-parity` | Single-shot CI gate: exit 0 on `259/259 crc entries match byte-for-byte`, exit 1 with diagnostic on mismatch. |
| `dummy-proof-emit` | Full pipeline as CLI on the **dummy** witness — setup → prove → eip197 encode. Stdout: 512 hex chars; stderr: timing diagnostics. |
| `fixture-proof-emit` | Full pipeline on a **real** Nova fixture (`--steps N`) — produces a proof bound to a specific accumulator state via real `l_u_secondary.X[..2]`. Optional `--vk-out` + `--public-inputs-out`. |

## Integration tests

| File | Purpose |
|---|---|
| `tests/section2_parity.rs` | Section 2 constants byte-parity against neptune (requires `/tmp/neptune-bn256-standard.json`). |
| `tests/full_pipeline.rs` | Dummy-witness pipeline: setup → prove → encode → decode → verify. |
| `tests/real_fixture_pipeline.rs` | Real-fixture pipeline + binding pin: a proof from fixture A rejects fixture B's public inputs. |

## Operator quick-checks

### Section 2 byte-parity

```bash
# 1. Extract neptune's actual constants
cargo run -p evaporchain-nova-bridge --bin dump-neptune-constants
#   wrote ./neptune-bn256-standard.json (568 KB)
ln -sf "$PWD/neptune-bn256-standard.json" /tmp/neptune-bn256-standard.json

# 2. Verify our impl matches
cargo run -p evaporchain-nova-bridge --bin check-neptune-parity -- \
    --neptune /tmp/neptune-bn256-standard.json
# → check-neptune-parity: PASS — 259 of 259 crc entries match byte-for-byte
```

### Emit an L1-paste-ready proof

```bash
# Dummy witness (fast)
cargo run -p evaporchain-nova-bridge --bin dummy-proof-emit -- --seed 0
# → 512 hex chars on stdout

# Real fixture (binds to a specific Nova accumulator state)
cargo run -p evaporchain-nova-bridge --bin fixture-proof-emit -- \
    --steps 3 --seed 7 \
    --vk-out /tmp/fixture-vk.bin \
    --public-inputs-out /tmp/fixture-pi.txt
# → real committed hashes printed on stderr; 512 hex chars on stdout
```

The stdout hex is `verifyProof`'s first argument in Solidity; `/tmp/fixture-pi.txt` (newline-separated hex, 4 entries) is the public-inputs second argument.

## Current scaffold status

`SCAFFOLD_VERSION = "phase-2.5-operational"` — proof emission is operationally complete on `main`.

- ✅ `phase-2.2-starter` — fixture generator
- ✅ `phase-2.2-skeleton` — verifier circuit skeleton + public-input wiring
- ✅ `phase-2.2-section-1` — off-circuit `validate_structurally` gate
- ✅ `phase-2.2-section-2-constants` — neptune compressed-ARK byte-correct end-to-end
- ✅ `phase-2.3-operational` — scalar adapter + circuit_builder + real `l_u_secondary` extraction
- ✅ `phase-2.4-operational` — Groth16 setup / prove / verify wrappers
- ✅ `phase-2.5-operational` — EIP-197 codec + real-fixture pipeline + operator binaries
- ⬜ `phase-2-complete` — Section 2 sponge framing + Section 3 RelaxedR1CS

What's left for **cryptographic soundness** (vs operational completeness):

- **Section 2 sponge framing** — close the `assert_ne!` canary in `section2_gadget::tests::fully_aligned_gadget_byte_parity_with_neptune` by porting neptune's SBOX-trick partial-round fusion into arkworks's `PoseidonConfig` (or vendoring neptune's permutation). Multi-day BESPOKE.
- **Section 3 RelaxedR1CS satisfiability** — in-circuit `is_sat_relaxed × 2 + is_sat × 1`. 3-5 day BESPOKE research deliverable.

Until those close, `fixture-proof-emit` produces a proof that BINDS to a specific accumulator state (verifiable on L1) but does not yet enforce Nova-soundness in circuit.
