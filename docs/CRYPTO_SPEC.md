# EvaporChain Cryptographic Specification

This document specifies all cryptographic primitives used in EvaporChain.
It is intended for security auditors and protocol implementors.

## 1. Hash Functions

### 1.1 BLAKE3

- **Library:** `blake3` v1.8.3
- **Output:** 256-bit (32 bytes)
- **Usage:** General hashing, MMR internal nodes, ghost record data commitments,
  deterministic constant derivation
- **No custom implementation** — direct library usage

### 1.2 Poseidon (Custom Implementation)

**File:** `crates/evaporchain-crypto/src/hash.rs`

**Parameters:**
| Parameter | Value |
|-----------|-------|
| Field | Pallas base field (Fp), p = 0x40000000000000000000000000000000224698fc094cf91b992d30ed00000001 |
| Width (t) | 3 |
| Rate | 2 |
| Capacity | 1 |
| Full rounds (R_F) | 8 (4 head + 4 tail) |
| Partial rounds (R_P) | 56 |
| Total rounds | 64 |
| S-box | x^5 (quintic) |
| MDS matrix | 3×3 Cauchy: M[i][j] = 1/(x_i + y_j), x={0,1,2}, y={3,4,5} |
| Round constants | BLAKE3("EvaporChain_Poseidon_RC_{r}_{j}") → Fp |

**Permutation Algorithm:**
```
state ← [capacity_element, rate_element_0, rate_element_1]

// 4 full rounds
for r in 0..4:
    state[i] += round_constants[r][i]  ∀i
    state[i] ← state[i]^5              ∀i    (full S-box)
    state ← MDS × state

// 56 partial rounds
for r in 4..60:
    state[i] += round_constants[r][i]  ∀i
    state[0] ← state[0]^5                    (partial S-box, index 0 only)
    state ← MDS × state

// 4 full rounds
for r in 60..64:
    state[i] += round_constants[r][i]  ∀i
    state[i] ← state[i]^5              ∀i
    state ← MDS × state
```

**Sponge Construction:**
- Input split into 31-byte chunks (< field modulus when zero-padded to 32)
- Absorption: XOR chunks into rate elements, permute
- Squeeze: Output state[1] as 32-byte canonical field representation

**Field Element Conversion:**
```
bytes_to_field(b: [u8; 32]) → Fp:
    b[31] &= 0x3F          // clear top 2 bits to ensure < p
    return Fp::from_repr(b)  // little-endian interpretation
```

**Audit Notes:**
- Round constants are deterministic (reproducible from BLAKE3 seeds)
- MDS matrix uses Cauchy construction (provably MDS)
- The x^5 S-box is the standard choice for Poseidon over prime fields with gcd(5, p-1) = 1
- **Field mismatch note:** This Poseidon operates over Pallas Fp. The Nova proving system
  uses BN254 scalar field. These are used independently — Poseidon for native hashing,
  Nova for proof constraints. The Nova circuit has its own field arithmetic.

---

## 2. Signature Schemes

### 2.1 ML-DSA (Dilithium3) — Transaction Signing

- **Library:** `pqcrypto-dilithium` v0.5.0
- **Standard:** NIST FIPS 204 (ML-DSA), Security Level 3
- **File:** `crates/evaporchain-crypto/src/signatures.rs`

| Parameter | Size |
|-----------|------|
| Public key | 1,952 bytes |
| Secret key | 4,032 bytes |
| Signature | 3,293 bytes |

**Hardness Assumptions:**
- Module-LWE (Learning With Errors)
- Module-SIS (Short Integer Solution)
- Claimed security: ~192-bit classical, ~128-bit quantum

**Usage:** Every user transaction (Transfer, CreateObject, Refresh, DeployContract,
CallContract, DeployScript, CallScript) is signed with the sender's ML-DSA key.

### 2.2 BLS12-381 — Consensus Attestations

- **Library:** `blst` v0.3.16
- **File:** `crates/evaporchain-crypto/src/signatures.rs`

| Parameter | Size |
|-----------|------|
| Public key | 48 bytes (compressed G1) |
| Secret key | 32 bytes |
| Signature | 96 bytes (compressed G2) |
| Aggregate signature | 96 bytes (constant) |

**Domain Separation Tag:** `BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_NUL_`

**Operations:**
- `sign(sk, msg) → σ ∈ G2`
- `verify(pk, msg, σ) → bool` (pairing check: e(pk, H(msg)) = e(G1, σ))
- `aggregate([σ₁, ..., σₙ]) → σ_agg ∈ G2` (point addition)
- `aggregate_verify([pk₁, ..., pkₙ], msg, σ_agg) → bool`

**Usage:** Validators sign block proposals. Signatures are aggregated for
space-efficient consensus certificates. Finality requires ≥ 2/3 stake weight.

**Audit Note:** The `fast_aggregate_verify` path assumes all signers signed
the same message. This is correct for consensus (all validators attest to the
same block hash). Mixed-message aggregate verification is not used.

---

## 3. Commitment Schemes

### 3.1 Verkle Trie (Custom Implementation)

**File:** `crates/evaporchain-crypto/src/verkle.rs`

**Parameters:**
| Parameter | Value |
|-----------|-------|
| Curve | Pallas (Ep) |
| Branching factor | 256 |
| Max depth | 32 (one byte per level) |
| Generators | 257 Pallas points (G₀ ... G₂₅₆) |

**Generator Derivation:**
```
for i in 0..=256:
    seed = "EvaporChain_Verkle_Gen_{i}"
    hash = BLAKE3(seed)
    scalar = bytes_to_scalar(hash)
    G_i = scalar × G_base       (Pallas base generator)
```

**Commitment Function (Pedersen Vector Commitment):**
```
commit(children: BTreeMap<u8, Node>) → Ep:
    C = O (identity point)
    for (idx, child) in children:
        h = node_hash(child)
        s = bytes_to_scalar(h)
        C = C + s × G_idx
    return C
```

**Node Hashing:**
- Empty node: `[0u8; 32]`
- Leaf node: `BLAKE3(key || value)`
- Internal node: serialize `commit(children)` to 32 bytes via affine coordinates

**Proof Structure:**
```
VerkleProof {
    key: [u8; 32],
    value: Option<[u8; 32]>,      // None for non-existence
    depth: usize,
    commitments: Vec<[u8; 32]>,   // commitment at each level
    path_indices: Vec<u8>,         // child index at each level
    siblings: Vec<Vec<(u8, [u8; 32])>>,  // (index, hash) per level
}
```

**Proof Verification:**
Bottom-up reconstruction: starting from leaf hash, at each level reconstruct
the Pedersen commitment using the path child and siblings, then compare the
top-level result against the expected root.

**Audit Notes:**
- Internal node children stored in BTreeMap for deterministic iteration order
- Commitments are algebraically commutative (order of summation doesn't affect result)
- Binding property relies on ECDLP hardness on Pallas
- Hiding property is not provided (commitments are deterministic)

### 3.2 Merkle Mountain Range (Custom Implementation)

**File:** `crates/evaporchain-crypto/src/accumulator.rs`

**Structure:** Append-only accumulator of perfect binary Merkle trees.

**Leaf Format (EnergyStampedNullifier):**
```
nullifier = BLAKE3(object_id || value_hash || evaporation_epoch_le64 || energy_at_death_le64 || owner)
```

**Internal Node Hash:**
```
parent = BLAKE3(left_child || right_child)
```

**Root Computation (Bagging the Peaks):**
```
peaks = all tree roots, from highest to lowest
root = fold_right(peaks, |acc, peak| BLAKE3(peak || acc))
```

**Leaf-to-Node Position:**
```
node_position(leaf_index) = 2 × leaf_index - popcount(leaf_index)
```

**Proof:** Merkle proof from leaf to its peak, plus all other peaks for root reconstruction.

---

## 4. Zero-Knowledge Proving System

### 4.1 Nova IVC (Incremental Verifiable Computation)

- **Library:** `nova-snark` v0.68
- **File:** `crates/evaporchain-proving/src/nova.rs`

**Curves:**
| Role | Curve | Field Size |
|------|-------|------------|
| Primary | BN254 (Bn256) | ~254 bits |
| Secondary | Grumpkin | ~254 bits |

**Commitment Scheme:** HyperKZG (trusted setup for BN254)

**Circuit: RealBlockCircuit**

`pp.num_constraints()` reports `(14041, 10554)` — primary +
secondary = **24,595 constraints/fold step** as of 2026-05-02.
The breakdown below is the user-defined subset (~2,481
constraints, ~10% of total); the remaining ~22,114 are Nova-
internal augmented-step + secondary-curve verifier overhead, not
controllable from the circuit body.

| Section | Constraints | Purpose |
|---------|-------------|---------|
| Per-object thermodynamic decay (5 enforce + 2 × 32-bit range checks per object × 16 objects) | ~1,168 | E(t) = E₀ × 2^(−t/τ) via bit-shift + remainder bounds |
| Per-transfer (3 enforce + 1 × 32-bit range check per transfer × 16 transfers) | ~576 | Balance conservation + amount range check |
| State-root limb decomposition (4 × 64-bit range checks + recomp + limb0 eq) | ~262 | Bind 32-byte verkle root |
| MMR-root limb decomposition (mirror of state-root) | ~262 | Bind 32-byte MMR root |
| Privacy state (note-tree binding + pool conservation + 3 × 64-bit range checks + bookkeeping) | ~199 | Shielded pool / note tree state transitions |
| Per-evaporation nullifier binding | ~8 | One per evaporated object (≤ 8/block) |
| Epoch + block + state/mmr/tx/evap bindings | 6 | IVC public-state transitions |
| **User-defined subtotal** | **~2,481** | Constraints we control directly |
| Nova augmented step (Poseidon binding + scalar-mul + commitment verifier) | ~11,560 | Primary-curve overhead, fixed by arity 6 |
| Nova secondary verifier (full circuit on Grumpkin) | ~10,554 | Secondary-curve overhead |
| **Total** | **~24,595** | Primary + secondary, per fold step |

See `research/proposals/smaller-ivc-circuit.md` for cut analysis +
reduction proposals.

**Circuit Parameters:**
| Parameter | Value |
|-----------|-------|
| RANGE_BITS | 32 |
| MAX_OBJECTS | 16 |
| MAX_TRANSFERS | 16 |
| MAX_EVAPORATIONS | 8 |

**Range Check (Bit Decomposition):**
```
range_check(cs, value, num_bits):
    for i in 0..num_bits:
        bit_i = alloc((value >> i) & 1)
        enforce(bit_i × (1 - bit_i) = 0)    // boolean constraint
    enforce(Σ(bit_i × 2^i) = value)          // reconstruction
```

**Enforce Less-Than:**
```
enforce_less_than(cs, a, b, num_bits):
    diff = b - a - 1
    range_check(cs, diff, num_bits)           // diff ≥ 0 ∧ diff < 2^num_bits
```

**Performance (Apple M4, release build):**
| Metric | Value |
|--------|-------|
| Fold time | ~18ms |
| Proof size | ~11.6KB (compressed SNARK) |
| Verification | ~5ms |

---

## 5. Encryption

### 5.1 AES-256-GCM (Encrypted Mempool)

- **Library:** `aes-gcm` v0.10
- **Usage:** MEV-resistant commit-reveal for transaction ordering
- **Key derivation:** BLAKE3-based from round randomness
- **Located in:** `crates/evaporchain-consensus/`

---

## 6. Data Availability

### 6.1 Reed-Solomon Erasure Coding

- **Library:** `reed-solomon-erasure` v6.0
- **Usage:** Block data availability sampling
- **Located in:** `crates/evaporchain-da/`

---

## 7. Dependency Audit Matrix

| Crate | Version | Audited By | CVEs | Notes |
|-------|---------|------------|------|-------|
| blake3 | 1.8.3 | Jack O'Connor et al. | None known | CFRG standard |
| sha2 | 0.10.9 | RustCrypto team | None known | NIST standard |
| pasta_curves | 0.5.1 | Zcash / ECC | None known | Used in Halo2 |
| pqcrypto-dilithium | 0.5.0 | PQClean project | None known | FIPS 204 ref impl |
| blst | 0.3.16 | Supranational + EF | Audited (NCC Group) | Ethereum production |
| aes-gcm | 0.10 | RustCrypto team | None known | NIST standard |
| nova-snark | 0.68 | Microsoft Research | None known | Academic origin |
| ff | 0.13.1 | RustCrypto team | None known | Trait crate |
| group | 0.13.0 | RustCrypto team | None known | Trait crate |

---

## 8. Custom Code Requiring Audit Focus

| Component | File | Lines | Risk Level |
|-----------|------|-------|------------|
| Poseidon hash | `crates/evaporchain-crypto/src/hash.rs` | ~170 | High |
| Verkle trie | `crates/evaporchain-crypto/src/verkle.rs` | ~470 | High |
| MMR accumulator | `crates/evaporchain-crypto/src/accumulator.rs` | ~350 | Medium |
| Nova circuit | `crates/evaporchain-proving/src/nova.rs` | ~700 | High |
| Energy decay | `crates/evaporchain-state/src/evaporation.rs` | ~170 | Medium |
| EvaporScript VM | `crates/evaporchain-script/src/vm.rs` | ~500 | Medium |
| Fee controller | `crates/evaporchain-execution/src/fees.rs` | ~100 | Low |
