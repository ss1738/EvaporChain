# BLS Key Rotation Runbook

**Lane T1.17** — BLS key rotation under live cluster conditions. Rotate a validator's on-chain BLS keypair via the `Transaction::RotateValidatorKey` mechanism without taking the cluster offline.

Pairs with: `MAINNET_READINESS.md` T1.17, `docs/runbooks/validator-passphrase-migration.md` (which migrates the *passphrase format*, not the key itself), `docs/runbooks/governance-rehearsal.md` (the staged-flip pattern this runbook extends to keys).

---

## When to rotate

- **Suspected compromise** — passphrase leaked via `/proc/<pid>/environ`, key file copied off-host, etc.
- **Scheduled rotation** — annual key-hygiene policy. Doctrine: BLS keys aren't fragile, but periodic rotation reduces tail-risk from undetected past compromise.
- **Format upgrade** — when adopting a new BLS suite version (currently `min_pk` / blst — no near-term change planned).

If only the passphrase needs rotation but the BLS keypair stays the same, use `validator-passphrase-migration.md` instead. That's a strictly local operation; no on-chain tx.

---

## What `RotateValidatorKey` does

The chain processes `Transaction::RotateValidatorKey { validator_id, new_bls_public_key, bls_pop_new, effective_epoch, nonce }` and, at `effective_epoch`, swaps the validator's recorded public key. The execution path enforces:

- `effective_epoch` is strictly in the future (`execution/lib.rs:3148-3154`)
- `new_bls_public_key.len() == 48` bytes (compressed G1)
- `validator_id` has an existing stake record
- Signer's address matches the validator's recorded address
- `nonce` matches the validator's expected next nonce
- `bls_pop_new` verifies under the new public key (proof-of-possession — prevents rogue-key attacks)

Once the tx commits at height H_commit, the validator continues attesting with the OLD key until `effective_epoch`. At `effective_epoch`, the chain's validator-set lookup returns the NEW public key. The operator must have the new private key in the on-disk slot by then.

---

## Pre-flight

1. **Cluster healthy.** Same checklist as `governance-rehearsal.md` step 1-3: lockstep heights, no finality stall, conservation audits non-null.

2. **Operator buddy in chat.** Key rotation is consensus-affecting — if the timing slips, the validator misses blocks and gets jailed.

3. **Choose `effective_epoch`.** Recommended: current_epoch + 200 (gives ~3 minutes of slack at 1s blocks; adjust to your block cadence). Too short → cutover stress; too long → if the rotation is needed for compromise response, the old key keeps signing for longer than needed.

4. **BLS-key passphrase format.** Decide before generating: encrypted (EVK1) or magic-plaintext (EVPL). Production should be EVK1 per `bls_key_store` doctrine. If migrating from legacy raw bytes, follow `evpl-plaintext-migration.md` *first*, then rotate.

5. **Backup the current key.** Copy `~/.evaporchain-tailscale-data/bls_key.bin` to a holding location BEFORE generating the new key. Without this, a failed rotation is unrecoverable.

   ```bash
   ssh <node> 'cp ~/.evaporchain-tailscale-data/bls_key.bin ~/.evaporchain-tailscale-data/bls_key.bin.preroll-$(date +%s)'
   ```

---

## Execution

### Step 1 — generate the new BLS keypair on a staging machine

Use the CLI's keygen path (`evaporchain-cli keygen` or whatever your bring-up runbook uses). Critically: **do not generate the new key on the validator itself** if you suspect host compromise — generate on a clean staging host, then copy.

The output is a 48-byte compressed G1 public key + 32-byte secret + a PoP signature over the new public key.

Capture:
- `NEW_BLS_PUBLIC_KEY` — 96-char hex
- `NEW_BLS_POP` — 192-char hex (G2 signature)
- Place the secret in EVK1 format at the staging machine, ready to push.

### Step 2 — build the `RotateValidatorKey` transaction

The tx body matches the schema in `crates/evaporchain-types/src/lib.rs::RotateValidatorKey`. Build it offline:

```jsonc
{
  "type": "rotate_validator_key",
  "validator_id": <your_validator_id>,
  "new_bls_public_key": "<NEW_BLS_PUBLIC_KEY>",
  "bls_pop_new": "<NEW_BLS_POP>",
  "effective_epoch": <current_epoch + 200>,
  "nonce": <next_nonce_for_your_address>
}
```

Sign with your validator's authority key (ML-DSA / Ed25519 / whatever your validator-onboarding runbook installed) — NOT the BLS key itself. The signer's address must match the validator's recorded address.

### Step 3 — submit + wait for inclusion

POST to any healthy node's `/api/tx` endpoint. The chain accepts the tx if execution invariants pass (pre-flight section above).

Watch for inclusion in `/api/chain/tx/:hash`:

```bash
curl -fsS "http://100.119.53.101:8081/api/chain/tx/$TX_HASH"
```

Expect `status: "committed"` with a block_number ≪ effective_epoch. If the tx is rejected at execution time, the error message (from `execution/lib.rs:3140-3197`) tells you which gate fired.

### Step 4 — stage the new key on the validator (before cutover)

Now copy the new EVK1-encrypted BLS key to the validator, **alongside** the old one — do not overwrite yet:

```bash
scp bls_key_new.bin <node>:~/.evaporchain-tailscale-data/bls_key_NEW.bin
```

Verify on the node:

```bash
ssh <node> 'ls -la ~/.evaporchain-tailscale-data/bls_key*'
# Expect both bls_key.bin (current) and bls_key_NEW.bin (staged) at mode 0600.
```

### Step 5 — observe the chain approach `effective_epoch`

Watch the chain's current_epoch via `/api/chain`:

```bash
watch -n 5 'curl -fsS http://100.119.53.101:8081/api/chain | python3 -c "import sys,json; d=json.load(sys.stdin); print(f\"h={d[\\\"block_height\\\"]} e={d[\\\"epoch\\\"]}\")"'
```

When `current_epoch` is `effective_epoch - 5` (i.e., about 5 epochs before activation), proceed to Step 6.

### Step 6 — cutover (the critical moment)

The chain's validator-set lookup returns the new public key at `effective_epoch`. The node binary reads the BLS key from `bls_key.bin` on every signing operation. You need `bls_key.bin` to point at the NEW key **by the time `effective_epoch` lands**.

Two cutover strategies:

**(A) Atomic rename** — fastest, no downtime. Run shortly before `effective_epoch`:

```bash
ssh <node> "mv ~/.evaporchain-tailscale-data/bls_key.bin     ~/.evaporchain-tailscale-data/bls_key_OLD.bin && \
            mv ~/.evaporchain-tailscale-data/bls_key_NEW.bin ~/.evaporchain-tailscale-data/bls_key.bin"
```

The node picks up the new key on its next signing operation (sub-second). Brief race window where the validator could attempt to sign with the OLD key while the chain expects NEW — this is harmless as long as the chain has not yet crossed `effective_epoch`.

**(B) Restart-mediated cutover** — slower (10-30s of validator downtime), eliminates the race. Run AT `effective_epoch`:

```bash
ssh <node> "systemctl stop evaporchain && \
            mv ~/.evaporchain-tailscale-data/bls_key.bin     ~/.evaporchain-tailscale-data/bls_key_OLD.bin && \
            mv ~/.evaporchain-tailscale-data/bls_key_NEW.bin ~/.evaporchain-tailscale-data/bls_key.bin && \
            systemctl start evaporchain"
```

Validator misses 10-30 blocks of attestations; doesn't cross slashing threshold (current `consensus.tendermint_signer_jail_threshold = 50` per default config).

Pick (A) for routine rotation, (B) when key compromise is acute.

### Step 7 — verify the rotation took

Confirm the validator is signing with the new key by checking attestation membership at `effective_epoch + 5`:

```bash
curl -fsS "http://100.119.53.101:8081/api/validators/active" | \
  python3 -c "import sys,json; d=json.load(sys.stdin); v=[x for x in d if x['validator_id']==<id>]; print(v[0]['bls_pubkey'])"
```

Expect the NEW public key. If you see the OLD key, the on-chain rotation didn't activate — check the tx status again.

Also confirm the validator is in `evap_active_validators`:

```bash
curl -fsS -H "Authorization: Bearer $EVAPORCHAIN_ADMIN_KEY" \
     "http://100.119.53.101:8081/metrics" | grep evap_active_validators
```

The metric should not have dropped during the cutover window.

---

## Rollback

Before `effective_epoch`: the rotation hasn't activated yet, so the operator can:
- Keep the old `bls_key.bin` in place (do not rename)
- Submit a follow-up tx that supersedes the pending rotation (if the chain supports — currently it does not; pending rotations are committed at the chain level)
- Accept the rotation and execute Step 4-6 as planned

After `effective_epoch`: rollback requires another `RotateValidatorKey` tx swapping back to the previous public key. Possible but slow; the chain must commit a second rotation and reach its new `effective_epoch`. During the rollback window the validator continues attesting with the (now-active) NEW key. The validator's PoP check requires the operator to KEEP the new private key while waiting — do not destroy `bls_key_NEW.bin` until the rollback completes.

---

## Post-rotation cleanup

After `effective_epoch + 100` (confirmed in active set with the new key):

```bash
ssh <node> "shred -u ~/.evaporchain-tailscale-data/bls_key_OLD.bin && \
            shred -u ~/.evaporchain-tailscale-data/bls_key.bin.preroll-*"
```

Use `shred` (or equivalent on macOS) — never `rm` — because the old key file may have already been swapped to disk and a plain `rm` leaves the bytes recoverable.

---

## Cross-references

- `MAINNET_READINESS.md` T1.17
- `crates/evaporchain-execution/src/lib.rs:3140-3197` — `RotateValidatorKey` execution semantics
- `crates/evaporchain-consensus/src/tendermint.rs:3748` — `apply_validator_key_rotations` consensus hook
- `crates/evaporchain-crypto/src/bls_key_store.rs` — EVK1 / EVPL on-disk formats
- `docs/runbooks/validator-passphrase-migration.md` — passphrase format migration (different operation)
- `docs/runbooks/governance-rehearsal.md` — staged-flip pattern this runbook extends
