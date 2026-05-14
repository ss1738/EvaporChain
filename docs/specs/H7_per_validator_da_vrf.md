# H7 Stage B — Per-Validator VRF for DA sampling

**Status:** spec (no code) — pending design approval before implementation.
**Closes:** AUDIT_2026_05_13 H7 Stage B (architectural follow-up).
**Touches:** `evaporchain-da`, `evaporchain-crypto`, `evaporchain-consensus`, `evaporchain-node`.
**Type:** hard fork (attestation format changes).

---

## 1. Problem statement

Stage A (PR #65, merged) made the DA-sample seed unbiasable by the
block producer. Concretely the seed is

```
seed = b"da-sample" || block.number_LE || validator_id_LE
```

and the producer no longer controls any byte of that input. So a
producer cannot pick block content that drives a specific
validator's subset.

What Stage A did **not** fix: the seed is still **globally
predictable**. Anyone who knows `(block.number, validator_id)` can
compute it offline. That gives a withholding attacker — one who is
sitting on incomplete erasure-coded rows — a free pre-targeting
oracle: they can compute, for every validator `V_i`, exactly which
6 cells `V_i` will sample, and decide whether to withhold those
specific cells.

Example attack flow:
1. Producer publishes block header → opponents learn `block.number`.
2. Attacker (a Sybil running 1 honest-looking shard-server) computes
   the sample subset for each `V_i` from the public formula.
3. Attacker withholds rows that NONE of the honest validators are
   going to sample, but a smaller fraction of cells overall. The
   block looks "available" to every validator's spot-check while
   actually having un-recoverable rows.
4. After 2f+1 validators attest "DA OK", the block finalizes.
5. The missing rows can never be reconstructed — light clients
   that try to reconstruct see the matrix is corrupted.

This is the canonical Celestia / Avail "data availability sampling"
attack model. The mitigation is well-known: per-validator VRF.

---

## 2. Goal

Make each validator's sample subset **unpredictable to any third
party until that validator publishes its attestation**. Specifically:

- (a) Before validator `V_i` attests, no one (not even another
  validator) can compute which cells `V_i` will sample.
- (b) After `V_i` publishes its attestation, anyone can verify that
  the published sample-cell list is the unique deterministic
  derivative of `V_i`'s VRF output for `(height, beacon)`.
- (c) The VRF output for `(height, beacon, V_i)` is reproducible
  exactly once — `V_i` cannot "rotate" subsets per-attestation to
  retry until they happen to hit a clean subset.

(a) + (b) + (c) jointly forbid the pre-targeting attack: the
attacker doesn't know which cells to corrupt, and the validator
can't roll for a clean subset.

---

## 3. Design

### 3.1 VRF input shape

New input builder mirrors the M2 `leader_vrf_input_v1` pattern:

```rust
/// crates/evaporchain-crypto/src/vrf.rs
pub fn da_sample_vrf_input_v1(
    chain_id: &str,
    block_number: u64,
    beacon: &RandomnessBeacon,
    validator_id: u64,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    buf.extend_from_slice(b"evaporchain:da-sample-vrf:v1\0");
    buf.extend_from_slice(chain_id.as_bytes());
    buf.push(0);
    buf.extend_from_slice(&block_number.to_le_bytes());
    buf.extend_from_slice(&beacon.value);    // 32 B chain-randomness
    buf.extend_from_slice(&validator_id.to_le_bytes());
    buf
}
```

- `chain_id` → cross-chain replay defense (matches M2).
- `beacon` → chain-controlled randomness that the validator cannot
  predict before block N. Provided by `evaporchain-bell-beacon`
  (already in production at every committed block).
- `block_number` → per-height uniqueness.
- `validator_id` → per-validator uniqueness.
- `\0`-terminated DST → workspace convention.

### 3.2 Subset-derivation function

Deterministic mapping from a 32-byte VRF output to `K` distinct
sample cells over a 2D `rows × cols` erasure-coded matrix:

```rust
/// crates/evaporchain-da/src/per_validator_vrf.rs
pub fn subset_from_vrf(
    vrf_output: &[u8; 32],
    rows: u32,
    cols: u32,
    k: usize,
) -> Vec<(u32, u32)> {
    let mut out = Vec::with_capacity(k);
    let mut seen = std::collections::HashSet::with_capacity(k);
    let mut counter: u64 = 0;
    while out.len() < k {
        // 16-byte (row, col) coordinate from a fresh blake3 stretch.
        let mut h = blake3::Hasher::new();
        h.update(b"evaporchain:da-cell-pick:v1\0");
        h.update(vrf_output);
        h.update(&counter.to_le_bytes());
        let bytes = h.finalize();
        let r = u32::from_le_bytes(bytes.as_bytes()[0..4].try_into().unwrap()) % rows;
        let c = u32::from_le_bytes(bytes.as_bytes()[4..8].try_into().unwrap()) % cols;
        counter += 1;
        if seen.insert((r, c)) {
            out.push((r, c));
        }
    }
    out
}
```

- DST-versioned (`v1\0`) — future migrations bump the suffix.
- Counter rolls past collisions so the output is exactly `k`
  distinct cells.
- Bounded worst-case: collision probability is `k / (rows×cols)`
  per draw, so expected draws ≤ `k × rows×cols / (rows×cols − k)`.
  At `k=8, rows=cols=64`, ~9 draws total.

### 3.3 Attestation format change

Extend `evaporchain_da::block_da::DAAttestation` (or whatever the
per-validator attestation struct is named — needs verification
during implementation; the consensus side currently builds these
inline at `tendermint.rs:7562`):

```rust
pub struct DAAttestation {
    pub block_number: u64,
    pub validator_id: u64,
    /// NEW (Stage B): VRF output bytes — 32 B.
    pub vrf_output: [u8; 32],
    /// NEW (Stage B): VRF proof — Dilithium3 sig over the VRF
    /// input, verifiable against the validator's registered
    /// ML-DSA pubkey. ~3293 B.
    pub vrf_proof: Vec<u8>,
    /// Cell-level sample responses; cells now derived from
    /// `vrf_output` via `subset_from_vrf`.
    pub responses: Vec<SampleResponse>,
    /// Validator's BLS signature over the (block_number, vrf_output,
    /// merkle_root_of_responses) tuple. Still required for the
    /// 2f+1 aggregation.
    pub bls_signature: Vec<u8>,
}
```

Bytes added per attestation: ~32 + 3293 = ~3325 B. At ~10
attestations per block × current 2-second block time ≈ ~16 KiB of
extra gossipsub traffic per minute per validator. Acceptable.

### 3.4 Verifier path

Replaces the seed-derived check at three call sites
(`main.rs:4595`, `main.rs:5622`, `main.rs:7193`):

```rust
fn verify_da_attestation(
    chain_id: &str,
    beacon: &RandomnessBeacon,
    validator_vrf_pk: &VrfPubKey,
    package_header: &BlockDAHeader,
    att: &DAAttestation,
) -> Result<(), DAAttError> {
    // 1. VRF proof check
    let input = da_sample_vrf_input_v1(
        chain_id, att.block_number, beacon, att.validator_id,
    );
    vrf_verify(validator_vrf_pk, &input, &att.vrf_output, &att.vrf_proof)?;

    // 2. Re-derive the subset the validator MUST have sampled
    let expected = subset_from_vrf(
        &att.vrf_output,
        package_header.rows,
        package_header.cols,
        DA_SAMPLES_PER_VALIDATOR,
    );

    // 3. Confirm the attested responses match exactly (in canonical
    //    order — sort by (row, col))
    let mut got: Vec<(u32, u32)> =
        att.responses.iter().map(|r| (r.row, r.col)).collect();
    got.sort_unstable();
    let mut want = expected.clone();
    want.sort_unstable();
    if got != want {
        return Err(DAAttError::SubsetMismatch);
    }

    // 4. Per-cell Merkle proofs against the row/column roots
    //    (unchanged from Stage A)
    for resp in &att.responses { resp.verify_against(package_header)?; }

    // 5. BLS sig over the digest
    let digest = blake3::Hasher::new()
        .update(&att.block_number.to_le_bytes())
        .update(&att.vrf_output)
        .update(&att.merkle_root())
        .finalize();
    bls_verify(&att.bls_signature, digest.as_bytes(),
        &validator_set.bls_pk(att.validator_id))?;
    Ok(())
}
```

### 3.5 Governance flag + migration

Add governance param:
```
da_sample_mode = "stage_a"  (default, current behaviour)
              | "stage_b"   (per-validator VRF enforced)
              | "observe"   (stage_b accepted but not enforced)
```

Migration:
1. Land code + tests + flag, flag default `"stage_a"`.
2. Devnet sweep with `"observe"` → validators publish VRF + subset
   alongside the legacy attestation. Verifiers cross-check but
   don't reject.
3. Public testnet sweep with `"observe"` for 1 week.
4. Hard-fork governance vote to flip `"stage_a"` → `"stage_b"` at
   a future activation height. Old-format attestations rejected
   from that height forward.

---

## 4. What this does NOT solve

- **Sybil sub-quorum.** If the attacker controls ≥ f+1 stake, they
  can sign off on DA-unavailable blocks regardless of any sampling
  scheme. This is a stake-distribution problem, not a sampling one.
- **Withholding by the producer of cells that no honest validator
  samples.** If `rows × cols × k_per_validator × honest_validators
  < total_cells`, there exist cells nobody samples. With production
  parameters (k=8, 50 validators, 64×64 matrix) we cover
  8×50/(64×64) ≈ 9.7% of the matrix. The defense relies on
  *redundancy* (any 50% of cells reconstructs the full matrix
  under 2D RS coding) plus per-validator unpredictability so the
  attacker can't pick "the 91% nobody samples". Stage B closes
  the unpredictability side.
- **VRF key compromise.** If a validator's VRF secret leaks, the
  attacker can pre-compute that validator's subset. Mitigation:
  same as leader VRF — operators rotate keys at epoch boundaries.
  Out of scope for Stage B itself.

---

## 5. Test plan

Unit (`per_validator_vrf.rs`):
- `subset_from_vrf` is deterministic on `(output, rows, cols, k)`.
- Distinct VRF outputs → distinct subsets (with overwhelming
  probability; assert no collision over 1000 random outputs).
- Cell coordinates are bounded by `rows × cols`.
- Exactly `k` distinct cells returned.

Integration (`evaporchain-da/tests/per_validator_vrf_integration.rs`):
- Validator produces attestation → verifier accepts.
- Tamper with `vrf_output` → reject (VRF proof fails).
- Tamper with one `(row, col)` in `responses` → reject
  (subset mismatch).
- Swap two validators' VRF outputs → reject (VRF proof fails
  against the wrong pubkey).
- Replay a prior height's attestation → reject (input includes
  `block_number`).

Adversarial (`evaporchain-consensus/tests/da_sampling_adversarial.rs`):
- Construct a block where rows `[r1, r2, r3]` are zeroed.
- 50 honest validators sample with the Stage B scheme.
- Assert: with probability > 0.99, at least one validator
  samples a zeroed cell and the attestation fails. Mean: the
  withholding attacker can no longer guarantee 2f+1 attestations.

---

## 6. Implementation order

1. **Crypto**: add `da_sample_vrf_input_v1` + `subset_from_vrf` +
   unit tests. (~150 LOC + tests.)
2. **DA**: extend `DAAttestation` (or equivalent) with the new
   fields + serde. (~100 LOC.)
3. **Consensus**: replace the seed-build sites with VRF-build +
   subset-derive paths, gated behind `da_sample_mode`. (~200 LOC.)
4. **Node**: wire validator's VRF keypair into the attestation
   producer. The keypair already exists (leader election uses it).
   (~50 LOC.)
5. **Governance**: add the `da_sample_mode` param with the
   migration ladder. (~30 LOC.)
6. **Tests**: unit + integration + adversarial harness. (~400 LOC
   total.)

Estimated PR size: ~900 LOC including tests. Implementable as a
single PR for the code change, plus a separate PR for the
governance migration.

---

## 7. Open questions

- (Q1) Should `vrf_proof` be optional during the `"observe"` window
  to give validators time to upgrade, or strict-on from the start
  with a deprecated-attestation grace period?
- (Q2) Beacon input: use the per-block `RandomnessBeacon` directly,
  or a delayed-finality version (e.g. `beacon_at(height - delta)`)
  to avoid a circular dependency between attestation and block
  acceptance?
- (Q3) Subset size `k`: keep current `6` per validator, or bump?
  Stage B's per-validator unpredictability shifts the security
  argument; the operator may want a different number to balance
  bandwidth vs. attack resistance.
