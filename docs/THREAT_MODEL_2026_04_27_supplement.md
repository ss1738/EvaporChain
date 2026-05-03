# Threat Model Supplement — 2026-04-27 (post-closure annotated 2026-05-03)

> **Status as of 2026-05-03:** §2.1, §2.2, §2.3, §2.5, §2.6 are **closed in code**.
> Closure annotations are inline in each subsection. **All §2.x items now
> closed** as of the 2026-05-03 re-audit pass. §2.7 was already-closed via
> the `fatal_persistence_error` helper + 15 `if let Err(e) = ... { ... }`
> sites in `rocksdb_backend.rs`. §2.4 was already-closed via six layered
> defenses in `FinalityTracker::on_block_finalized_with_active`
> (active-signer guard, duplicate-finalization guard, superseded-floor
> watermark, seen-proposals guard, empty-signer rejection, 2/3 stake
> quorum). The supplement is now fully obsolete and folds into the main
> `THREAT_MODEL.md` per §5 below in the next mainnet doc revision.

This is a **supplement** to `docs/THREAT_MODEL.md`, not a replacement. The base document still defines the system overview, trust assumptions, and adversary model. This supplement adds:

1. Status updates on existing mitigations (which are now real, which are still aspirational).
2. New attack surfaces discovered or sharpened by the audits documented in `audit/end_to_end_audit_2026_04_27.md`.
3. Adversary-capability deltas where 2026-04-27 reality differs from the base threat model's assumptions.

When the base `THREAT_MODEL.md` is rewritten for mainnet, this supplement should be folded in and deleted.

---

## 1. Trust-assumption deltas

| Base assumption | 2026-04-27 reality | Action |
|---|---|---|
| `Slashing conditions for equivocation (planned)` | Implemented for prevote/precommit equivocation; vote-liveness slashing also added (`f9ef6c8`). Equivocating votes are now rejected from quorum tally (verified at `tendermint.rs:1364-1378`) | Update base doc — slashing is no longer "planned" |
| `Validator BLS public keys are correctly registered` | **Closed (2026-05-02)** — PoP enforced at `ValidatorSet::add_validator()` and verified at genesis registration via `verify_pop`; `pop_verified=true` gating in node main.rs. Genesis schema carries `bls_pop` for every validator | Update base doc — rogue-key surface is closed |
| `BLS signatures aggregate over distinct messages` (implicit) | DST domain separation is correct (`BLS_DST` and `BLS_POP_DST` distinct), but DA attestation message canonicalization needs verification | Open audit item — see end-to-end audit §1 |
| `Encrypted mempool (MEV protection) implemented` (per README) | Confirmed in source — AES-256-GCM, end-to-end integrated. Threshold decryption and reveal mechanics still recommended for separate review | Status: implemented; deeper review pending |

---

## 2. New attack vectors discovered (2026-04-27 audit)

Each of these is a **real surface that the base threat model did not previously enumerate**. They are tracked individually in `audit/end_to_end_audit_2026_04_27.md`; the entries here exist so future threat-model maintainers can fold them into the canonical adversary table.

### 2.1 Oracle impersonation (CRITICAL, **CLOSED 2026-05-02**)

> **Closure note:** `oracle/consensus.rs` now invokes `HybridVerifier::verify` against the validator pubkey looked up by `vote.validator_id` from the validator set. Empty-signature short-circuit removed; non-short-circuit verification path enforced. See memory `evaporchain_reaudit_round_3_2026_05_02.md`.


**Adversary capability added:** any party with network access can submit oracle votes claiming to be any validator, with no cryptographic check.

**Why it's not in the base doc:** the base doc lists oracle as "decentralized oracle with BFT consensus" without enumerating the authentication surface. The implementation in `oracle/consensus.rs:183-203` does not actually verify signatures — it byte-compares `signature == vote_hash()`, which is computable by anyone, and skips the check entirely on empty signatures.

**Defense status:** none. Oracle authentication is open as written.

**Fix required:** `HybridVerifier::verify` against a validator pubkey looked up from the validator set by `vote.validator_id`, not trusted from the vote payload.

### 2.2 Governance whale-pass (CRITICAL, **CLOSED 2026-05-02**)

> **Closure note:** governance now enforces stake-weighted voting (vote weight = `min(balance, stake)`), a quorum threshold, parameter range validation at proposal application, and a timelock between pass and apply. Foundation-vote-passes-anything path eliminated. See memory `evaporchain_audit_round_2026_05_02.md`.


**Adversary capability added:** any account holding a plurality of supply can pass any governance proposal alone.

**Why it's not in the base doc:** the base doc lists "Compromise governance" as out-of-scope for the network adversary section, implicitly assuming governance has its own quorum. The implementation in `execution/lib.rs:893-952`:
- Uses vote weight = account balance (not stake)
- Has no quorum threshold (no minimum participation requirement)
- Pass condition is `votes_for > votes_against * 2`

Combined with `genesis-mainnet.json` allocating 35% of supply to a single Foundation Treasury address, **one Foundation vote passes any proposal**.

**Compounding factor:** governance can set arbitrary parameter values. There is no range check at `execution/lib.rs:951`. A malicious or compromised proposal can set `block_gas_limit` to `u64::MAX` or `block_reward` likewise.

**Defense status:** none.

**Fix required:** parameter range validation, vote-weight cap (e.g., `min(balance, stake)`), quorum requirement, optional timelock between pass and apply.

### 2.3 Contract upgrade by anyone (CRITICAL or HIGH, **CLOSED 2026-05-02**)

> **Closure note:** `Transaction::UpgradeContract` handler now reads `governance_approved` and refuses without an executed governance proposal of matching scope. Bytecode swap path implemented behind the gate. See memory `evaporchain_audit_round_2026_05_02.md`.


**Adversary capability added:** any account submitting a `Transaction::UpgradeContract` either silently succeeds-as-noop (broken feature) or upgrades the bytecode without governance approval (privilege escalation).

**Why it's not in the base doc:** contract upgrades were listed as "planned" in the base doc; they were added in commit `d006282`. The handler at `execution/lib.rs:1321-1325` is `Ok(())` and the `governance_approved` field at `types/lib.rs:888` is never read.

**Defense status:** depends on whether bytecode swap happens in a downstream handler. Audit could not locate one.

**Fix required:** decide whether the feature is in or out, then either implement it correctly (with `governance_approved` check) or remove `Transaction::UpgradeContract` until designed.

### 2.4 Finality records pollution (HIGH, **CLOSED** — verified 2026-05-03)

> **Closure note (2026-05-03):** re-audit of `FinalityTracker::on_block_finalized_with_active` (`finality.rs:230-338`) confirmed six layered defenses against record pollution:
>
> 1. **Active-signer guard** (line 245-260) — a backfill cert signed by validators no longer in the active set is rejected with `"cert signed by validators no longer active"`.
> 2. **Duplicate-finalization guard** (line 261) — `if self.records.contains_key(&height) return false` prevents any re-insertion of an existing record.
> 3. **Superseded-floor watermark** (line 268-274) — pruned heights track `self.superseded_floor`; backfill at or below this floor is rejected. The watermark is bumped during LRU pruning (line 332-334).
> 4. **Seen-proposals guard** (line 282-292) — a back-fill at `height < latest_finalized` is only accepted when the height was actually seen in a prior proposal observation, closing the "colluding majority backfills records the cluster never proposed" residual.
> 5. **Empty-signer rejection** (line 293) — `signer_ids.is_empty()` ⇒ reject.
> 6. **2/3 stake quorum** (line 296) — `signing_stake * 3 < total_stake * 2` ⇒ reject.
>
> Together these implement the "non-blocking variant of monotonicity that allows legitimate gap-fill but rejects already-superseded heights" that this section asked for. The supplement was stale; this annotation refreshes it.

**Adversary capability added:** an attacker holding old valid `CommitCertificate` data can backfill `FinalityTracker.records` at gap heights below `latest_finalized`, misleading light clients that look up historical finality.

**Why it's not in the base doc:** finality monotonicity was added in commit `87c8e1c` and immediately removed in `d70ab4c` (one minute later) to allow non-sequential delivery during sync. The replacement check (quorum-only) is not equivalent. `latest_finalized` itself remains monotone (`finality.rs:189`), so head-of-chain isn't rewritable, but the records map is poisonable.

**Defense status:** partial — head is safe, records map is not.

**Fix required:** non-blocking variant of monotonicity that allows legitimate gap-fill but rejects already-superseded heights.

### 2.5 Real DA enforcement is absent (CRITICAL for mainnet, **CLOSED 2026-05-02**)

> **Closure note:** `data_root` is now derived from `build_block_da_inputs(txs)` and is identical at proposal time and serve time (no mutated-block-bytes drift). `BlockDA2D::encode_block()` is wired into `produce_block` for both empty and non-empty blocks; finality is gated on DA attestation over the real `data_root`. `consensus/lib.rs:238` no longer leaves `data_root: None`. Empty-block `data_root` handling remains a code-side audit-backlog item. See memory `evaporchain_da_input_parity.md`.


**Adversary capability added:** a validator can produce a block whose `data_root` is a sentinel rather than a real 2D-erasure commitment. DA attestations finalize over the sentinel. There is no actual data-availability guarantee on the chain today.

**Why it's not in the base doc:** the base doc treats DA as "implemented (2D erasure coding, PoHA, NMT)." The library is implemented; integration into block production is missing. `consensus/lib.rs:238` initializes `data_root: None` and never calls `BlockDA2D::encode_block()`.

**Defense status:** none. Testnet's "DA supermajority" runs over the sentinel.

**Fix required:** wire the encoder into `produce_block` for both empty and non-empty blocks, and gate finality on DA attestation over the real `data_root`.

### 2.6 Validator BLS key extraction via local read (HIGH, **CLOSED 2026-05-02**)

> **Closure note:** `bls_key.bin` is no longer plaintext. Encrypted-Validator-Private-Key-Layout (EVPL) format: Argon2id KDF (named public constants) + XChaCha20-Poly1305 AEAD; magic-byte detection (`detect_bls_key_format`) auto-handles legacy plaintext for one-shot migration. Passphrase delivered via `EVAPORCHAIN_VALIDATOR_KEY_PASS_FILE` (avoids /proc/.../environ exposure). `format_plaintext_for_disk` retained only for migration. Key-rotation runbook published. See memories `evaporchain_reaudit_round_5_2026_05_02.md`, `evaporchain_reaudit_round_6_2026_05_02.md`, `evaporchain_reaudit_round_10_2026_05_02.md`.


**Adversary capability added:** an adversary with file-read access to a validator's `data_dir` can recover the BLS signing key from `bls_key.bin` (plaintext, mode 0600 only).

**Why it's not in the base doc:** the base doc says "compromise honest validator private keys" is out-of-scope for the external adversary. That holds for *network* adversaries, but the local-host attack surface (lateral movement, container escape, backup leak, supply-chain) is real. The C-10 fix from the prior audit landed on wallet-user keys (`auth.rs`), not validator keys.

**Defense status:** OS file permissions only.

**Fix required:** passphrase or KMS-backed encryption of `bls_key.bin` before mainnet. Document key-rotation procedure.

### 2.7 Persistence panic on write failure (HIGH, **CLOSED** — verified 2026-05-03)

> **Closure note (2026-05-03):** re-audit of `rocksdb_backend.rs` at commit
> `fff3e6f` confirmed that EVERY persistence write site uses the
> `if let Err(e) = ... { fatal_persistence_error(op, e); }` pattern. The
> helper at lines 62–71 emits a structured `tracing::error!` with the
> failed operation name + I/O error, sleeps 100 ms for the subscriber
> to flush, then `std::process::exit(2)`s. The two remaining `.expect()`
> calls in the file (line 1014 just-inserted-HashMap-lookup, line 1209
> startup-time CF handle) are programmer-invariant checks, not I/O paths.
> The original audit-flagged `.expect("write object to RocksDB")` was
> closed in an earlier sprint and the supplement was simply not refreshed.

**Adversary capability added (historic):** an adversary who fills the disk or revokes write permissions on a validator host triggered a panic in `rocksdb_backend.rs:338, 388` (`.expect("write object to RocksDB")`), crashing the node.

**Why it's not in the base doc:** the base doc treats persistence as "RocksDB with crash recovery" without enumerating the failure-mode surface. Crashes on local resource exhaustion are slashable downtime.

**Defense status:** none — direct panic.

**Fix required:** propagate `Result` from persistence into the block-apply path; on failure, halt the chain gracefully (don't panic mid-block).

---

## 3. Adversary-model tightening

The base doc's external-adversary section says "Cannot: compromise honest validator private keys." Refine this to:

> The external adversary cannot compromise honest validator private keys *via the network*, assuming the validator runs a hardened deployment with:
> - Encrypted at-rest validator keys (Argon2id + XChaCha20-Poly1305 EVPL format) — **IMPLEMENTED (2026-05-02)**
> - File system isolation (validator data_dir not readable by other users)
> - No backup of `data_dir` to untrusted storage
> - Operational hygiene around log redaction (no key bytes in logs)

The local-host adversary (lateral movement, supply chain, backup compromise) is treated as out-of-scope only because it's an *operator* problem, not a *protocol* problem. Mainnet operator runbook must enumerate these.

---

## 4. Cross-references

- Cross-verification of contested findings: `audit/cross_verification_2026_04_27.md`
- End-to-end audit including all six domain reviews: `audit/end_to_end_audit_2026_04_27.md`
- RFP for external auditor engagement: `audit/external_audit_rfp_2026_04_27.md`
- Audit-readiness pack (in/out scope, invariants, known issues): `audit/audit_readiness_pack_2026_04_27.md`
- Operational parameters (every constant cited here): `docs/PARAMETERS.md`
- Genesis ceremony procedure (where the centralization concern surfaces): `docs/GENESIS_CEREMONY.md`

---

## 5. Recommended next revision

When the open items in §2 are closed in source, this supplement should be folded into the base `THREAT_MODEL.md`:

1. Move §1 entries into the base doc's section 2 (Trust Assumptions).
2. Add new sections under base doc's section 4 (Attack Surface Analysis) for each §2 item that became defended (mitigation notes, citation).
3. Move §3 refinement into base doc's section 3.1 (External Adversary).
4. Delete this supplement.

Until that fold-in happens, treat both documents as joint canon.
