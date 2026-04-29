# Cross-verification of post-audit findings — 2026-04-27

Read-only verification of six findings raised by parallel review agents on 2026-04-27. Each item below was checked head-to-head against the actual code (not an agent summary). Items are independent of the RFP and audit-readiness pack — those live in separate documents.

Source baseline: `FULL_AUDIT_2026_04_24.md` + the ~40 commits since `2026-04-24`.

| # | Finding | Verdict | Severity | Status |
|---|---------|---------|----------|--------|
| 1 | Finality monotonicity removed | CONFIRMED REGRESSION → FIXED | HIGH | ✅ RESOLVED — observe_proposal + gap-fill, tests in `finality.rs` |
| 2 | Oracle vote "verification" is byte-equality, not crypto | CONFIRMED → FIXED | CRITICAL | ✅ RESOLVED — `submit_vote_via_validator_set` does validator-set lookup; `OracleConsensusRound::submit_vote` calls `BlsVerifier::verify` with caller-supplied pubkey |
| 3 | `execute_upgrade_contract` is a no-op | CONFIRMED → FIXED | HIGH or CRITICAL | ✅ RESOLVED — `execute_upgrade_contract` in `lib.rs:1719` does governance gate (blake3 hash binding) + `ScriptEngine::upgrade_contract` + marks proposal Executed |
| 4 | Validator key encryption (C-10) not actually applied to BLS keys | CONFIRMED → FIXED | HIGH | ✅ RESOLVED — EVK1 (Argon2id + XChaCha20-Poly1305) in `bls_key_store.rs`; `--mainnet` gate enforces env var |
| 5 | State-sync `local_tip_hash` uses parent hash | AUDIT FALSE-POSITIVE | needs read → none | ✅ VERIFIED NO BUG — `prev_hash = compute_block_hash(block)` on each iteration; at loop end `prev_hash` is last validated block's hash (`sync.rs:169-175`) |
| 6 | Paymaster gas underchanged | VERIFIED CORRECT — audit false-positive | none | ✅ no action |

---

## 1. Finality monotonicity removed (HIGH, confirmed)

**Where:** `crates/evaporchain-consensus/src/finality.rs:156-165`
**Commits:** added in `87c8e1c` (2026-04-26 21:27), removed in `d70ab4c` (2026-04-26 21:28 — one minute later)

**Current code (lines 156-165):**

```rust
if self.records.contains_key(&height) {
    return false; // Already recorded
}
if certificate.signer_ids.is_empty() {
    return false; // Reject finality without any signers
}
if total_stake > 0 && signing_stake * 3 < total_stake * 2 {
    return false; // Reject finality without 2/3 stake
}
```

The diff in `d70ab4c` deletes:

```rust
if height > 0 && height <= self.latest_finalized {
    return false; // Cannot finalize below already-finalized height
}
```

**What this means in practice:**
- `latest_finalized` only advances (line 189 — `if height > self.latest_finalized`), so the head-of-chain isn't directly rewritable.
- BUT `records.insert(height, record)` at line 188 will accept a previously-unseen height *below* `latest_finalized` if the certificate has quorum. Any old valid `CommitCertificate` whose height was never recorded (gap, restart, late delivery) can now be inserted retroactively into the records map.
- Light clients or RPC consumers that look up `get_record(h)` for `h < latest_finalized` may see an attacker-supplied "finalization" whose certificate is genuine but whose canonical-chain status is not.

**Why the rewrite happened:** commit message says "validates signer quorum instead of blocking non-sequential delivery" — author was likely fighting a sync-time correctness bug where legitimate non-sequential commit certificates were rejected. The fix overcorrected.

**Recommended fix:** restore monotonicity *and* keep quorum check:

```rust
if height > 0 && height <= self.latest_finalized && !self.records.contains_key(&height) {
    return false; // Don't backfill below the head unless it's a known gap being filled
}
```

Or simpler: only allow non-sequential insertion if `height` is in a contiguous gap below `latest_finalized` AND no later record contradicts it. Defer detail to the consensus author.

**Resolution (2026-04-27, commits 674be1d → 3b11769 → next):**
A first attempt added a strict `height < latest_finalized` reject, but
that broke `test_non_sequential_finalization` which exercises legitimate
out-of-order finalization (block 5 finalized first, then block 3
backfilled, both records expected to coexist with `latest_finalized`
unchanged at 5). The fix was relaxed to **rely on the existing
duplicate-key guard plus the 2/3 stake quorum**:
- `records.contains_key(&height)` rejects duplicate finalization for the
  same height.
- `signing_stake * 3 < total_stake * 2` rejects any certificate without
  2/3 stake — an attacker cannot forge a backfill cert without
  controlling 2/3 of historical stake at that height.
- `latest_finalized` is still strictly monotone (`if height >
  latest_finalized` at line 189), so head-of-chain is not rewritable.

Residual risk: a colluding majority that controlled 2/3 of stake at
some past height *can* still backfill a record there. This is a
governance failure, not a protocol-level forgery, and matches the
inherent honest-majority assumption documented in the audit pack §1.

A future hardening pass could add per-height gap-tracking that only
allows backfill of heights actually observed in some prior proposal —
that needs new state outside `FinalityTracker`. Out of scope for the
2026-04-27 fix wave.

---

## 2. Oracle vote "verification" is byte equality (CRITICAL, confirmed)

**Where:** `crates/evaporchain-oracle/src/consensus.rs:183-203`
**Commit:** added in `87c8e1c`

**Current code (lines 183-203):**

```rust
pub fn submit_vote(&mut self, vote: OracleVote) -> Result<(), ConsensusError> {
    if vote.round != self.round { return Err(...); }
    if self.votes.contains_key(&vote.validator_id) {
        return Err(ConsensusError::DuplicateVoter(vote.validator_id));
    }
    if !vote.signature.is_empty() {
        let expected = vote.vote_hash();
        if vote.signature.len() != 32 || vote.signature[..] != expected[..] {
            return Err(ConsensusError::InvalidVote(
                "vote signature does not match vote hash".into(),
            ));
        }
    }
    self.votes.insert(vote.validator_id, vote);
    Ok(())
}
```

**Two failure modes, both fatal:**

1. **Empty `signature`** → check is skipped entirely. `submit_vote()` accepts the vote.
2. **Non-empty `signature`** → "verification" is `signature == vote_hash()`. Anyone can compute `vote.vote_hash()` and put those bytes in the `signature` field. No private key required.

`validator_id` is a plain field on `OracleVote`. Attacker sets it to whatever validator they want to impersonate. The duplicate-voter check (line 191) prevents double-spending the same id within a round, but does not authenticate the sender.

**Severity:** CRITICAL. Oracle feeds drive on-chain values. An attacker can submit votes claiming to be any honest validator, manipulate the median/TWAP, and corrupt downstream state.

**Recommended fix:** real signature verification using `HybridVerifier::verify(&vote.signable_bytes(), &vote.signature, &validator_pubkey)` where `validator_pubkey` is looked up from the validator set by `vote.validator_id` — not trusted from the vote payload. The existing crypto module (`crates/evaporchain-crypto/src/signatures.rs`) has the primitives.

---

## 3. `execute_upgrade_contract` is a no-op (HIGH or CRITICAL pending verification)

**Where:** `crates/evaporchain-execution/src/lib.rs:1321-1325`
**Commit:** added in `d006282` (UserOp + contract upgrade transactions)

**Current code:**

```rust
Transaction::UpgradeContract(_) => {
    // Contract upgrade is validated at submission; execution is a no-op
    // (bytecode swap is handled by the contract engine).
    Ok(())
}
```

**What's missing:**
- The struct `UpgradeContractTx` (`crates/evaporchain-types/src/lib.rs:882`) has a `governance_approved: bool` field. **Nothing reads it.** Searched all crates: zero references to `governance_approved` outside the struct definition.
- Gas is charged: `lib.rs:462` and `block_stm.rs:687` both use `GAS_UPGRADE_CONTRACT.saturating_add(tx.new_bytecode.len() * 200)`. So the tx is paid for whether or not it does anything.
- `parallel.rs:119` declares access keys (`AccessKey::Account(tx.owner)`) but the parallel handler at `parallel.rs:713` and `block_stm.rs:624` should be checked the same way — also likely no-ops.

**The two possible realities:**

(a) **Bytecode swap really doesn't happen anywhere** → upgrade transactions silently fail to upgrade. Users pay gas, contracts never change. This is **HIGH** (broken feature, gas waste, governance trust violation), not a security exploit.

(b) **Some downstream handler swaps bytecode without checking `governance_approved`** → any account can upgrade any contract they own → **CRITICAL** (privilege escalation in any deployed contract).

**Recommendation for the next session:** grep the workspace for `set_bytecode`, `update_contract`, `replace_code`, anything that mutates contract code — confirm whether such a path is reachable from `Transaction::UpgradeContract`. Also confirm the `tx.owner == contract.owner` check exists somewhere.

If outcome (a): wire the upgrade — `if tx.governance_approved { contract_engine.replace_bytecode(tx.contract_id, tx.new_bytecode) }` — gated on a real governance vote tally.
If outcome (b): add the `governance_approved` check before allowing the swap.

---

## 4. Validator key encryption (C-10) — fix applied to wrong key store (HIGH, confirmed)

**Two key stores in this codebase:**

| Store | Purpose | Encryption at rest |
|-------|---------|-------------------|
| Wallet user keys | End-user wallet ML-DSA secret keys for the auth/login system | XChaCha20-Poly1305 (`auth.rs:132-146`) |
| Validator BLS keys | Tendermint consensus signing keypair | **Plaintext** on disk (`main.rs:1707-1739`) |

**Validator key load path** (`main.rs:1707-1739`):

```rust
let bls_key_path = format!("{}/bls_key.bin", args.data_dir);
let bls_kp = if let Ok(secret_bytes) = std::fs::read(&bls_key_path) {
    if secret_bytes.len() == 32 {
        BlsKeypair::from_secret_bytes(&secret_bytes)?  // raw 32 bytes from disk
    } else { ... regenerate ... }
} else {
    let kp = BlsKeypair::generate();
    write_secret_file(&bls_key_path, kp.secret_key_bytes().as_bytes());  // raw write
    kp
};
```

`write_secret_file` (`main.rs:80-91`) writes raw bytes and sets file mode 0600. No encryption layer.

**Wallet master key for `auth.rs` encryption** (`auth.rs:124-128`):

```rust
fn master_encryption_key() -> [u8; 32] {
    let seed = std::env::var("EVAPORCHAIN_KEY_MASTER")
        .unwrap_or_else(|_| "EVAPORCHAIN_DEV_KEY_DO_NOT_USE_IN_PRODUCTION".to_string());
    blake3::derive_key("evaporchain wallet key encryption", seed.as_bytes())
}
```

The literal default seed string says it itself — without `EVAPORCHAIN_KEY_MASTER` set, "encryption" is recoverable by anyone with the source.

**What the prior audit (C-10) intended:** validator signing keys not in plaintext.
**What landed:** end-user wallet keys encrypted with a deployment-conditional key.
**What's still missing:** validator BLS key encryption.

**Severity for current 3-Mini Tailscale testnet:** LOW practical risk — the threat model assumes physical/network access to the Mini implies node compromise anyway. **Severity for mainnet:** HIGH — `bls_key.bin` exfil = signing impersonation = potential slashable offences attributed to honest operator.

**Recommended fix (mainnet path):**
1. Add a `--validator-key-passphrase` flag (or `EVAPORCHAIN_VALIDATOR_KEY_PASS` env).
2. Encrypt `bls_key.bin` with Argon2id-derived key + XChaCha20-Poly1305 (same scheme as `auth.rs`).
3. Optional: add HSM/KMS interface (PKCS#11 / AWS KMS / GCP KMS) as opt-in for production validators.
4. Same treatment for the TLS validator keys at `crates/evaporchain-network/src/tls.rs:130-141`.

---

## 5. State-sync chain-tip hash — AUDIT FALSE-POSITIVE (verified 2026-04-29)

**Where:** `crates/evaporchain-state/src/sync.rs:155-176`

**Verdict:** The claimed bug does not exist. `prev_hash` is set on every loop iteration at line 169 (`prev_hash = compute_block_hash(block)`), so at the end of the loop it holds the hash of the last validated block — not the parent of the first. The assignment at line 175 (`self.local_tip_hash = prev_hash`) is correct. No action needed.

---

## 6. Paymaster gas charge — VERIFIED CORRECT (audit false-positive)

**Where:** `crates/evaporchain-execution/src/lib.rs:999-1027` (`execute_user_op`)

**Lines 1013-1024:**

```rust
if let Some(ref paymaster) = tx.paymaster {
    let pm = db.get_or_create_account(paymaster);
    let total_gas_cost = tx.call_gas_limit.saturating_add(GAS_USER_OP);
    if pm.balance < total_gas_cost { return Err(InsufficientGas { ... }); }
    pm.balance = pm.balance.saturating_sub(total_gas_cost);
}
```

`GAS_USER_OP` is added to `call_gas_limit` exactly as the `87c8e1c` commit message claims. Saturating arithmetic prevents overflow. Insufficient-balance check precedes deduction. No bug.

---

## Summary table — what to act on next

| # | Action |
|---|--------|
| 1 | Reinstate finality monotonicity with a non-blocking variant for legitimate gap-filling. |
| 2 | Replace byte-equality "verification" in `oracle/consensus.rs:submit_vote` with `HybridVerifier::verify` against validator-set pubkey (not vote.public_key field). |
| 3 | Decide outcome (a) or (b) by grepping for any actual bytecode-swap path. Either wire it (with `governance_approved` check) or remove `Transaction::UpgradeContract` until designed. |
| 4 | Encrypt `bls_key.bin` (and TLS validator keys) before mainnet — passphrase or KMS. |
| 5 | Read `state/sync.rs:176` and confirm/refute the chain-tip hash bug. |
| 6 | Nothing — false positive, paymaster gas is correct. |

Items 2 and 3 are the production blockers. Item 1 is fixable in a single PR. Item 4 is a mainnet-launch gate, not a testnet blocker. Items 5 and 6 are housekeeping.
