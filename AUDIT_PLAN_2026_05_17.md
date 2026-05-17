# Audit Closure Plan — AUDIT_2026_05_17 (Remaining)

5 sequential steps. Work top-to-bottom, step by step.
Each step is a coherent code class — same crate family or same threat model.

---

## Step 1 — Fix the red test: Verkle DST (CR-1 / CR-2 / CR-3)

**Priority:** CRITICAL × 3. `cargo test -p evaporchain-crypto` is RED on HEAD.

| ID | File | Problem |
|---|---|---|
| CR-1 | `crypto/src/energy_verkle.rs:208-229` | `EnergyNode::hash` produces no-DST hashes; `verify` reconstructs WITH DST → mismatch → proof always fails |
| CR-2 | `crypto/src/verkle.rs:425-505` | `verify` never checks `proof.path_indices[i] == proof.key[i]`; non-existence proof forgeable |
| CR-3 | `crypto/src/energy_verkle.rs:1086-1158` | `verify_multi` reconstructs WITHOUT DST while `verify` uses DST — two sibling verifiers disagree |

**Goal:** green `test_proof_verifies`, consistent DST across `hash`, `verify`, `verify_multi`, path-index binding.

---

## Step 2 — DA cert forgery: Q1 / Q2 / Q3 + Q5 + Q7

**Priority:** CRITICAL × 3 + HIGH × 2. One-commit fix for Q1+Q2+Q3 (same message-signing surface).

| ID | File | Problem |
|---|---|---|
| Q1 | `consensus/tendermint.rs:8019-8104` | `cert.total_stake` + `att.stake` attacker-supplied; never cross-checked → single-key supermajority forgery |
| Q2 | `da/certificate.rs:56-93` | No dedup by `validator_id`; 100 copies of same attestation → 100× stake |
| Q3 | `da/certificate.rs:147-178` | BLS signed message excludes `stake` field → stake rewritable post-sign |
| Q5 | `consensus/tendermint.rs:2463-2493` | Antichain finalization count-weighted (`>= 2f+1`); should be stake-weighted |
| Q7 | `consensus/bridge.rs:126-143` | `StateProof::verify` unsafe sorted-Merkle — no leaf-index, no size, no DST |

**Goal:** `stake` in signed message, cross-check `total_stake` against registered set, dedup attestations, stake-weight antichain quorum, add Merkle binding to StateProof.

---

## Step 3 — Auth surfaces: A1 / A2 / A3 + STATE-2 + SBA-1

**Priority:** CRITICAL × 3. Wallet impersonation + dead governance + broken auction.

| ID | File | Problem |
|---|---|---|
| A1 | `node/api.rs` — `wallet_sign_tx`, `wallet_submit_tx`, `post_settle_demurrage`, 5 Singh-pool handlers | No caller-vs-wallet ownership check; any authenticated session can sign/submit for any wallet |
| A2 | follow-on from A1 | `wallet_sign_tx` impersonation path |
| A3 | follow-on from A1 | `post_settle_demurrage` impersonation path |
| STATE-2 | `state/rocksdb_backend.rs:1351-1384` | 9 trait methods are no-op stubs: governance params, proposals, snapshots, historical queries — all silently dead on the real cluster |
| SBA-1 | `contracts/evaporscript/sealed_bid_auction.es:68-93` | `commit(hash)` ignores `hash`; `reveal()` never verifies — commit-reveal is purely cosmetic |

**Goal:** ownership gate on all wallet handlers, implement the 9 RocksDB stubs, add hash-storage + verify step to sealed_bid_auction.

---

## Step 4 — Remaining HIGHs + critical MEDs

**Priority:** HIGH × 4 + MED × 3.

| ID | File | Problem |
|---|---|---|
| L0-A | `lambda-fold/nova_path.rs:153-157` | IVC decay uses first object's half_life (or hardcoded 100) instead of ChainLambda |
| H-3 | `crypto/accumulator.rs:250-283` | `MMRProof.mmr_size` plumbed but never validated against external commitment |
| H-4 | `crypto/bls_portable.rs:62-118` | `aggregate_verify` no per-key PoP for non-validator callers |
| Q11 | `consensus/tendermint.rs` | Round-state wipe at `MAX_ROUNDS_PER_HEIGHT` not modeled in TLA spec |
| OPCODE-1 | `script/src/vm.rs:826-836` | `Op::VrfDomainRandomness` flat `GAS_STATE_LOAD=5` regardless of domain string length (up to 1 MiB) |
| RULE-1 | `contracts/src/lib.rs:642` | `energy_cost += cost` raw add; near-`u64::MAX` CostEnergy rules wrap silently |
| INV-MED-4 | `light-cone/src/lib.rs` | Crate honest; doctrine §4.1 #1 overclaims fork-choice production status |

**Goal:** ChainLambda in nova_path, MMR size pre-check, PoP assertion at non-validator verify, TLA+Rust alignment note for Q11, scaled gas on VRF domain, checked_add in rule engine, doctrine downgrade for light-cone.

---

## Step 5 — Crypto mediums + doctrine + LOWs

**Priority:** MED + LOW. Polish / hardening class. No exploits, but audit weight is real.

| ID | File | Problem |
|---|---|---|
| M-1 | `crypto/secret_file_store.rs:54` | Argon2id `t=3` vs `t=4` in `bls_key_store.rs` — same threat model, different cost |
| M-2 | `crypto/secret_file_store.rs:82,140` | Argon2id-derived key not in `Zeroizing<>` |
| M-3 | `crypto/verkle.rs:177-200` | `VerkleProof.commitments` filled by `prove`, never read by `verify` — dead field |
| M-4 | `crypto/hash.rs:217-242` | Poseidon sponge: no IV, no capacity tag, no padding DST, no length prefix |
| M-6 | `bridge/contracts/HashToCurve.sol:14-25` | Doc says PoP DST, code uses NUL DST |
| INV-MED-5 | `total-evaporscript`, `cap-decay-vm`, `dp-native-vm` | Tier-2 VMs cite §-ref only in Cargo.toml, not lib.rs head doc |
| INV-MED-6 | `decay-lamport`, `fee-controller`, `llsa`, `sentinel`, `tombstone` | Missing standalone `tests/e2e.rs` against non-trivial fixtures |
| Q12 | `network/service.rs:1394-1401` | Empty chain_id falls back to unscoped legacy gossipsub topics |
| LOW batch | OPCODE-2/3/4, EXEC-1, SUB-1, POOL-1, RULE-2, WAL-1 | Gas asymmetries, micro-DoS hazards, dead module remnants |

**Goal:** uniform Argon2id params, Zeroizing wrapper, strip dead commitments field, harden Poseidon or rename to `poseidon_experimental`, fix HashToCurve.sol doc, add lib.rs citations to Tier-2 VMs, add e2e tests for doctrine primitives, fail-hard on empty chain_id at startup.

---

## Status tracker

| Step | Status | Commits |
|---|---|---|
| Step 1 — CR-1/2/3 Verkle DST | 🔴 NOT STARTED | — |
| Step 2 — Q1/2/3 + Q5 + Q7 DA cert | 🔴 NOT STARTED | — |
| Step 3 — A1/2/3 + STATE-2 + SBA-1 | 🔴 NOT STARTED | — |
| Step 4 — L0-A + H-3/4 + Q11 + MEDs | 🔴 NOT STARTED | — |
| Step 5 — M-1..6 + INV + LOWs | 🔴 NOT STARTED | — |
