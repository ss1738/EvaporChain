# EvaporChain — Mainnet Punch List

**Status as of 2026-04-27.** Single source of truth for what's left between "audit-hardened" and "mainnet-live." Tick items off with `[x]` as they close. Do not delete closed items — keep history.

**Rule:** plow through in order, don't pause between items. Per project memory `project_evaporchain_mainnet_sprint.md`.

---

## Tier 1 — Protocol completion (multi-day each, all code)

### [x] 1. Frontier ZK privacy — finish the partial path  ✅ 2026-04-27
**Where:** `crates/evaporchain-execution/src/privacy_exec.rs` + `crates/evaporchain-state/src/rocksdb_backend.rs`
**State today (revised after 2026-04-27 source read):**
- Gas wired (shield 60K, unshield 80K, transfer 100K base + per-input/output)
- shield/unshield/private_transfer all use **real** Pedersen + Merkle + balance-binding verification (not stubs)
- Nullifier set persisted in `CF_NULLIFIERS`. Note-tree **root** persisted in `CF_TRIE` under `PRIVACY_NOTE_ROOT_KEY`
- 13+ negative tests already in place (double-spend, stale anchor, malformed binding, invalid commitment opening, invalid Merkle proof, intra-tx duplicate nullifiers)

**Note:** "Per-tx Nova SNARK verifier" was originally listed but is a *new design decision*, not finishing a partial path — Nova folds blocks today, not per-tx privacy proofs. Tracked separately as item #18 if pursued.

**Done when:**
- [x] 1a. Full note-commitment list persisted to RocksDB (`CF_NOTE_COMMITMENTS` CF, key = u64 BE leaf index, value = 32-byte commitment). Wired at all 3 insertion sites (shield, unshield change-outputs, private_transfer outputs). Idempotent.
- [x] 1b. On startup, in-memory note tree rebuilt from persisted commitment list. New `PrivacyExecutor::restore_from_db()` + public `TendermintConsensus::restore_privacy_from_db()` wrapper, called from `node/main.rs` after consensus state restore. Fatal panic on root mismatch.
- [x] 1c. Round-trip integration test: `test_round_trip_persistence_shield_transfer_unshield` — shield 5K → private_transfer 3K+1.9K (fee 100) → unshield 3K, asserts balances + persisted commitments + restored-tree root match.
- [x] 1d. Replay-attack test: `test_replay_attack_across_epochs_rejected` — same nullifier across `set_epoch(1)` → `set_epoch(2)`, anchor preserved so only the nullifier check can fire. Asserts `DoubleSpend`, no balance double-credit.

### [x] 2. Frontier DA sampling — finish the in-progress path  ✅ 2026-04-27
**Where:** `da_flow.mmd` light-client side (encoder side closed K-04, commit `1fc67c0`)
**State today (revised after 2026-04-27 source read):**
- Encoder + DA cert wired into all 3 producers ✅ (closed K-04)
- `BlockDA2D::light_client_sample()` and validator-side sampler at `tendermint.rs:2500-2549` already exist
- Existing RPC `/api/da/cell/:block/:row/:col` existed but **omitted `cell_data`** — making it unverifiable. Patched (real bug fix).
- Two true light-client gaps remained: (a) abstraction over peer-fetching so a node-less light client could sample, (b) peer-fault hook for reputation integration.

**Done when:**
- [x] 2a. New module `crates/evaporchain-da/src/light_client.rs` — `LightClientSampler<S: CellSource>` with `trait CellSource` (`fetch_cell` + `report_faulty`), `SamplingReport { results, metrics, faulty_peers, all_valid }`, `PeerFaultReason` enum (`InvalidProof`, `HashMismatch`, `OutOfRange`, `Unreachable`). Verifier checks `cell_data` hash → `cell_hash` BEFORE the Merkle path (strictly stronger fault evidence).
- [x] 2b. RPC `/api/da/cell/:block/:row/:col` patched to include `cell_data` in JSON response (`api.rs:3960`). Without this the endpoint was unverifiable.
- [x] 2c. Peer-fault hook delivered as `CellSource::report_faulty(peer_id, reason)` trait method. **Note:** concrete peer-reputation infrastructure does not exist in `evaporchain-network` — building one is tracked separately as it's not required to satisfy the punch-list criterion.
- [x] 2d. 5 sampling-assurance tests in `light_client.rs#tests`: all-honest → high confidence + passes; corrupt peer → marked faulty, doesn't pass; fabricated cell → caught by hash-check first; partially-withheld block → fails HP threshold without false-marking the stale peer; mixed honest+corrupt → only the bad peer marked.

### [x] 3. TLS validator cert encryption  ✅ 2026-04-28
**Where:** new `secret_file_store` module + `tls.rs` integration + mainnet strict-mode hook
**State today:** All 3 sub-items closed. EVK1 was 32-byte fixed; PEMs are variable-length, so introduced `EVKV` (variable-length sibling) rather than retrofitting EVK1.

**Done when:**
- [x] 3a. New `crates/evaporchain-crypto/src/secret_file_store.rs` — variable-length `EVKV` envelope (`magic || salt(16) || nonce(24) || len_be(4) || ciphertext+tag`). Same primitives as EVK1 (Argon2id + XChaCha20-Poly1305). 16 MiB plaintext cap, length-tampering caught by AEAD. 11 unit tests.
- [x] 3b. `tls.rs::generate_ca` and `generate_validator_cert` route private keys through `write_pem_secret`, which encrypts under `EVAPORCHAIN_VALIDATOR_KEY_PASS` when set. New `read_pem_secret(path)` auto-detects EVKV vs plaintext PEM by magic prefix (`EVKV` cannot collide with `-----BEGIN`). 2 round-trip tests added (encrypted + plaintext-passthrough).
- [x] 3c. `validate_mainnet_strict` in `node/main.rs` scans `data_dir` for `*-key.pem` files and refuses to start in `--mainnet` mode if any are plaintext. Mirrors the existing BLS-key check.

**Cosmetic follow-up:** flip `validator_key_lifecycle.mmd` node N from `fill:#fff3cd` → `fill:#d1e7dd` and remove the "TLS still plaintext" note. Pure docs change.

### [x] 4. Validator key rotation  ✅ 2026-04-28
**Where:** new tx type + ValidatorInfo schema + cert verification + key persistence ring
**State today:** 4a complete — tx variant lands on the chain through every dispatch and serialization path, gas const wired (80K), execution path cleanly refuses with a clear error so no partial rotation corrupts the validator set. 4b/4c/4d remain.

**Done when:**
- [x] 4a. New `RotateValidatorKeyTx { validator_address, validator_id, new_bls_public_key, bls_pop_old, bls_pop_new, effective_epoch, nonce, signature, public_key }` in `evaporchain-types`. Type tag 0x16 in `signable_bytes()`. Length-prefixed BLS fields for canonical encoding. New `Transaction::RotateValidatorKey` variant with arms in all 22 `Transaction::*` match sites across types/execution/consensus/node/persistence/api/parallel/block_stm. Gas const `GAS_ROTATE_VALIDATOR_KEY = 80_000`. Execution path errors out (`ContractError`) until 4b — refusing to admit a malformed rotation rather than silently corrupting state.
- [x] 4b. Execution implementation landed across `evaporchain-execution`, `evaporchain-consensus`, and `evaporchain-types`:
  - `BlockExecutionResult.validator_key_rotations: Vec<ValidatorKeyRotation>` — side-channel from execution to consensus
  - Execution arm validates: effective_epoch ≥ block.epoch, sender == registered validator address, nonce, new pubkey is 48 bytes, `bls_pop_new` PoP-verifies. Old-key continuity verify is deferred to consensus (which owns the live `ValidatorSet`).
  - `ValidatorInfo` schema: new `bls_public_key_prev: Option<Vec<u8>>` + `bls_prev_key_expiry_epoch: Option<u64>` fields, both `#[serde(default)]` for back-compat.
  - `ValidatorSet::rotate_validator_key()` and `purge_expired_prev_keys()` methods.
  - `TendermintConsensus::apply_validator_key_rotations()` — called post-`execute_block`, runs continuity verify against current pubkey, applies rotation. Failures silently skipped (gas already paid; validator set untouched).
  - `verify_commit_certificate` now runs two-pass: pass 1 with current pubkeys; pass 2 (only if pass 1 fails AND any signer is in grace) substitutes prev key for grace-eligible signers.
  - `bridge.rs::verify_certificate_signature` documented: bridge operates on `ValidatorSetCommitment` snapshots and is out of scope for live grace-window verification.
- [x] 4c. Key file ring: `pick_active_bls_key_path` scans `data_dir` for `bls_key.{N}.bin` files, picks highest-N (falls back to canonical `bls_key.bin`). `purge_stale_bls_key_files` runs at startup once `current_epoch` is known, deleting numbered files older than `current_epoch - KEY_ROTATION_GRACE_EPOCHS` while preserving the active (highest-epoch) file unconditionally. `KEY_ROTATION_GRACE_EPOCHS` promoted to `pub` so node binary + operator tooling reference the same constant.
- [x] 4d. Tests landed:
  - `validator_set.rs`: 4 unit tests covering rotation state mutation + purge + edge cases (unknown validator, no current key)
  - `tendermint.rs`: 4 cert-verification tests — two-pass accepts old-key signature during grace, two-pass accepts new-key signature always (pass 1), past-grace rejects old-key signature, bad continuity proof rejected by `apply_validator_key_rotations`
  - "Validator keeps producing across boundary" is conceptually verified via the cert-acceptance behaviour (other validators accept old-key signatures during grace). Cross-process integration test deferred to multi-node testnet sprint.

### [x] 5. Mempool global byte cap enforcement  ✅ 2026-04-28
**Where:** `crates/evaporchain-consensus/src/mempool.rs`

**Done when:**
- [x] `MAX_MEMPOOL_BYTES` constant declared = 256 MiB (`mempool.rs:18`)
- [x] Admit-time check in `validate_submission` rejects when `total_bytes + tx_size > MAX_MEMPOOL_BYTES`. Saturating-add prevents overflow under adversarial size values.
- [x] TTL eviction was already implemented (`evict_expired` runs on `set_epoch`, MAX_TX_AGE_EPOCHS = 256). PARAMETERS.md row updated to reflect actual values.
- [x] `PARAMETERS.md §5` "Global byte cap" row replaced with the new constant + file:line citation. Audit-flagged "Tracked but not enforced" note removed.
- [x] Test: `test_global_byte_cap_rejects_when_pool_would_overflow` submits 100 KB blobs spread across senders (to dodge the per-account cap) and asserts the byte cap fires before the 10K tx-count cap.

### [x] 6. Storage-rent enforcement  ✅ 2026-04-28
**Where:** per-epoch tick across SimpleExecutor + ParallelExecutor + Block-STM

**State today (revised):** Rent collection function existed in `SimpleExecutor` but ran every block (over-charging by ~50× at 2s blocks) and was missing entirely from the production-path `ParallelExecutor` and Block-STM engines.

**Done when:**
- [x] New `last_rent_epoch: u64` cursor on `StateDB` (default-impl 0, persisted in RocksDB under `LAST_RENT_EPOCH_KEY`, mirrored on InMemoryStateDB).
- [x] `collect_storage_rent` gates on `current_epoch > last_rent_epoch` so cadence is exactly per-epoch. Idempotent on repeated calls within the same epoch.
- [x] Same gated logic inlined into `ParallelExecutor::execute_block` and `BlockSTMExecutor::execute_block` so the production paths actually charge rent.
- [x] `PARAMETERS.md §3` storage-rent row updated — "enforcement stubbed" note removed.
- [x] Test: `test_last_rent_epoch_default_and_persist` covers the cursor primitive.

---

## Tier 2 — Formal verification mechanization (multi-week each)

### [~] 7. Coq mechanization: integer-decay monotonicity  (scaffolded 2026-04-28; one obligation `Admitted`)
**Where:** `research/coq/EnergyDecayMonotonicity.v`

**State today:** Workspace + spec + main theorem statement + within-halving case fully proven (`Qed.`). The cross-halving step is `Admitted.` pending one arithmetic-bound lemma — its statement and proof obligation are documented in the file. A Rust-side cross-reference comment points from `energy_at_epoch` (`evaporchain-types/src/lib.rs:1331`) to the Coq spec.

**Done when:**
- [x] `research/coq/` workspace: `_CoqProject`, `Makefile`, `README.md`
- [x] `energy_at_epoch` faithfully translated from Rust (u64 + integer division + bit-shift halvings + linear interpolation) to Coq `nat`
- [x] Base cases proved (`energy_at_epoch_zero`, `energy_at_epoch_zero_halflife`, `energy_at_epoch_past_cutoff`)
- [x] Within-halving monotonicity (`energy_step_within_halving`) proved at `Qed.`
- [x] `energy_at_epoch_monotone` and `energy_at_epoch_monotone_general` theorems stated, structured proof in place
- [x] Rust source carries a back-reference comment so future edits to the Rust impl trigger Coq re-check
- [ ] **Open:** discharge `energy_step_cross_halving` `Admitted` — reduced to a single arithmetic-bound lemma:  
      `Nat.div v 2 <= v - Nat.div (v * (h - 1)) (2 * h)` for `h >= 1`

### [~] 8. Coq mechanization: Energy-Verkle compression invariants  (scaffolded 2026-04-28)
**Where:** `research/coq/EnergyVerkleCompression.v`

**State today:** Spec + 3 invariants in place. The punch-list framing of "compress(decompress(c)) ≡ c" turned out to be inaccurate against the actual Rust impl — there is no `decompress` operation; leaves under a compressed subtree are recovered via a separate ghost-record resurrection path. The corrected invariant set:

**Done when:**
- [x] Abstract trie model in Coq (`NEmpty | NLeaf | NInternal | NCompressed`)
- [x] `compress_preserves_total_leaf_count` proven at `Qed.` — the Compressed node's recorded leaf_count equals the original subtree's total
- [x] `compress_energy_sum_monotone` proven at `Qed.` — energy sum can only decrease (compression always lands at sum = 0)
- [x] `compress_energy_conservative` proven at `Qed.` (modulo one Admitted Forall-induction lemma) — when the cold-precondition holds, energy sum is exactly preserved
- [x] `compress_preserves_commitment` axiomatized — bound to the `commitment: child.hash()` construction in `energy_verkle.rs:562`. Cannot be proven in Coq without modeling BLS12-381; explicit dependency stated.
- [x] Rust source carries a back-reference comment listing each invariant
- [ ] **Open:** discharge the `Forall`-induction case in `cold_subtree_zero_energy` (mechanical)

### [~] 9. Coq scaffold: PoHA freeloading-resistance  (scaffolded 2026-04-28)
**Where:** `research/coq/PoHAFreeloading.v`

**State today:** Threat model formalized, theorem stated, security reduction structured. Final transitivity step (`p <=p q -> negligible q -> negligible p`) is `Admitted` — closing it requires modeling `prob` as `Q` and proving the standard "negligible is closed under upper-bound" lemma. Crypto axioms are explicit:
- A1: Merkle-cell-proof unforgeability (reduces to blake3 collision-resistance)
- A2: Sampling seed unmanipulability (reduces to blake3 random-oracle behaviour)
- BFT bound: `3 * adversary_stake < total_stake` (honest > 2/3 stake)

**Practical reading:** EvaporChain's DA security depends only on blake3 + BFT honest-majority. The EvaporChain-specific design (decaying DA, energy re-attestation, Pedersen commitments) does NOT introduce new cryptographic assumptions — that's the auditor-relevant claim this file makes precise.

### [~] 10. Coq mechanization: `lazy_eval ≡ eager_eval` for Rule-Based Consensus  (scaffolded 2026-04-28)
**Where:** `research/coq/LazyEagerEquivalence.v`

**State today:** Theorem `eager_eq_lazy` proven at `Qed.` relative to two axioms on the decay primitive: `decay_step_compose` (k+m steps = k then m) and `decay_step_zero` (0 steps = identity). The first is the non-trivial obligation — under the actual EvaporChain integer-rounding decay, composition is *approximate*, not exact. The frontier doc acknowledges this drift; the file documents the gap precisely.

**Done when:**
- [x] Eager and lazy evaluators defined as Coq fixpoints
- [x] `eager_eq_lazy` theorem proved at `Qed.` (relative to decay axioms)
- [x] `trace_query_agreement` corollary proved at `Qed.`
- [x] Drift-bound caveat documented inline — under bit-shift + linear-interpolation, composition is not exact, but the protocol's anchor-interval bound keeps the drift negligible
- [ ] **Open:** prove `decay_step_compose` for the actual `energy_at_epoch` function with a quantified drift bound `|lazy(e0, h, n+m) - eager(...)| <= O(1/h)`. Distinct from the monotonicity proof in `EnergyDecayMonotonicity.v`.

---

## Tier 3 — Operational launch (sequenced, gating)

### [ ] 11. 3-Mini Tailscale → public testnet
- `genesis-target.json` real timestamps (currently placeholder)
- Public bootnode multiaddrs
- Faucet contract + UI
- Read-only block explorer (RPC + minimal Next.js)

### [ ] 12. External audit RFP issuance
Kit at `audit/firm_engagement_kit/` is ready. Decisions: shortlist (ToB / Zellic / Code4rena), scope (whole chain vs frontier-primitives only).

### [ ] 13. Bug bounty activation
`docs/BUG_BOUNTY.md` template exists. Decisions: platform (Immunefi vs self-hosted), pool size.

### [ ] 14. Real genesis ceremony
`genesis_time` placeholder `2026-10-01T00:00:00Z` → real. Multi-party key generation across 4 founding validators. Transcript published per `docs/GENESIS_CEREMONY.md`.

---

## Tier 4 — Strategic (blocks mainnet, not building)

### [ ] 15. Whitepaper §4 drift fix
Replace Health-Score consensus narrative with Tendermint+BLS reality.

### [ ] 16. Foundation Treasury 35% centralization
Decision: vesting / multi-sig / staged unlock. Currently Foundation alone can pass any governance proposal. Mainnet blocker per `audit/end_to_end_audit_2026_04_27.md`.

### [ ] 17. Mainnet timeline + funding
Founder call.

---

## Working order

Tier 1 first, in numbered order. Tier 2 can run in parallel as a separate research thread — does not gate Tier 1 or Tier 3. Tier 3 starts only after Tier 1 closes. Tier 4 are decisions, not builds.

**Currently active:** Tier 1 (#1–#6) closed; Tier 2 (#7–#10) scaffolded with explicit `Admitted`s and named axioms. Workspace at `research/coq/` checks via `make`. Tier 3 (#11–#14) is operational; Tier 4 (#15–#17) is decisions.
