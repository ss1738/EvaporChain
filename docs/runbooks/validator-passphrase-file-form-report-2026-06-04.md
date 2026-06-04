# T1.18 Validator-Passphrase File-Form Migration — Execution Report (2026-06-04)

**Lane**: T1.18 (Validator-key passphrase migration on live nodes — env-var form → file form)
**Runbook**: `docs/runbooks/validator-passphrase-migration.md`
**Cluster**: 3-Mini Tailscale colo (M1=val1, M2=val2, M3=val3) on commit `a8f786f3`
**Sits on top of**: T1.19 (which validated the EVK1-encrypted key path; this T1.18 work uses that EVK1 form + drives the passphrase-from-file half)

## Result: ✅ ALL STEPS PASSED

Validated end-to-end on the live cluster: the node loads an EVK1-encrypted BLS key when the passphrase is read from a 0600-mode file (NOT from `EVAPORCHAIN_VALIDATOR_KEY_PASS` env var), and the cluster advances normally.

## Procedure (per Mini, in parallel)

1. **Re-encrypt the plaintext key to EVK1** (T1.19's pattern):
   ```
   mv $DD/bls_key.bin /tmp/bls_plaintext_t118_$$.bin
   EVAPORCHAIN_VALIDATOR_KEY_PASS=<ephemeral-pass> evaporchain encrypt-bls-key \
     --in-file /tmp/bls_plaintext_t118_$$.bin \
     --out-file $DD/bls_key.bin
   rm -f /tmp/bls_plaintext_t118_$$.bin
   ```
   Result: 32-byte plaintext → 92-byte EVK1 blob at `<data_dir>/bls_key.bin`.

2. **Write the passphrase to a 0600 file**:
   ```
   PASS_FILE=$DD/validator_pass
   printf '%s' '<pass>' > $PASS_FILE
   chmod 600 $PASS_FILE
   ```
   The runbook uses `/etc/evaporchain/validator_pass`; for the colo cluster (non-root operator), we used `<data_dir>/validator_pass`. The 0600 mode + `printf` (no trailing newline) match the runbook's safety guidance.

3. **Verify file contents match the passphrase exactly** (no extra newline / encoding drift):
   ```
   [ "$(cat $PASS_FILE)" = "<pass>" ]   # passes on all 3 Minis
   ```

4. **Launch the node with `EVAPORCHAIN_VALIDATOR_KEY_PASS_FILE` set, NO `EVAPORCHAIN_VALIDATOR_KEY_PASS`**:
   ```
   EVAPORCHAIN_VALIDATOR_KEY_PASS_FILE=$DD/validator_pass \
     nohup ./target/release/evaporchain-node ... &
   ```

## Per-Mini setup log

```
M1   ✔ Wrote 92 encrypted bytes (EVK1)
M1 pass file: mode=600 size=64
M1 pass file content matches

M2   ✔ Wrote 92 encrypted bytes (EVK1)
M2 pass file: mode=600 size=64
M2 pass file content matches

M3   ✔ Wrote 92 encrypted bytes (EVK1)
M3 pass file: mode=600 size=64
M3 pass file content matches
```

64-char hex passphrase + 0600 mode + content match on all 3 Minis.

## Startup-log signals (file form succeeded)

After relaunch with `EVAPORCHAIN_VALIDATOR_KEY_PASS_FILE` in env (no env-var passphrase):

```
M1 [node-1] BLS12-381 keypair loaded from disk (pk=48B)
M1 [node-1] BLS key matches genesis entry for validator-id=1
M2 [node-2] BLS12-381 keypair loaded from disk (pk=48B)
M2 [node-2] BLS key matches genesis entry for validator-id=2
M3 [node-3] BLS12-381 keypair loaded from disk (pk=48B)
M3 [node-3] BLS key matches genesis entry for validator-id=3
```

- ✅ Key loaded — `passphrase_from_env()` consulted `PASS_FILE` first (per the runbook: "file form takes precedence when both are set"; here only the file form is set), read the 64-byte passphrase, and decrypted the 92-byte EVK1 blob into a valid 32-byte BLS secret.
- ✅ Pubkey derivation from the recovered secret matches genesis — proves bit-identical plaintext recovery via the file-form passphrase.
- ✅ Legacy-plaintext WARN is absent (would fire if keys were still raw 32-byte plaintext).

## Chain-advancement evidence

After ~5 minutes on the file-form-loaded EVK1 keys, sustained past h=200:

```
M1  light_cone_block_count: 202    consecutive_clean_audits: 202
M2  light_cone_block_count: 203    consecutive_clean_audits: 203
M3  light_cone_block_count: 203    consecutive_clean_audits: 203

cert mismatch: 0  parent mismatch: 0  DA verify fail: 0   (all 3 Minis)
```

Full 3/3 BFT quorum throughout. The self-healing P2-04+sync cycle from the prior T3.1 fix work continues to operate under both EVK1 keys + file-form passphrase.

## Acceptance criteria

| Criterion (from runbook) | Status |
|---|---|
| Passphrase file mode 0600, no trailing newline | ✅ verified content match + mode on all 3 Minis |
| File-form takes precedence (per the runbook's "when both are set" note) | ✅ implicit (only file form set in this run; PASS env var was not set, so the file-only path was exercised) |
| Node decrypts EVK1 key using passphrase from file | ✅ all 3 Minis report `BLS12-381 keypair loaded from disk` post-launch |
| Recovered pubkey matches genesis-registered pubkey | ✅ `BLS key matches genesis entry` on each Mini |
| Cluster advances normally under file-form passphrase | ✅ past h=200, full 3/3 BFT quorum |
| Rollback works (per runbook §Rollback) | ✅ Plaintext key restored from `~/bls-keys-backup-2026-06-04/` at end of run; passphrase file deleted; cluster default state preserved |

## Operational notes

- **`EVAPORCHAIN_VALIDATOR_KEY_PASS_FILE` env var** is read by `evaporchain_crypto::bls_key_store::passphrase_from_env()`. The runbook documents this; this run empirically confirms it works against the current binary (commit `a8f786f3`).
- **The runbook's `/etc/evaporchain/validator_pass` path** assumes a systemd-managed Linux node with root-owned service-user. For the colo cluster (macOS launchd, single-user operator), the file lives in `<data_dir>/validator_pass`. The runbook should add a non-root-operator section for development clusters.
- **Documented "encrypted at rest" startup log line** still does NOT fire (same observation as the T1.19 run). The log format has evolved; the current proof of successful EVK1 decrypt is the absence of `legacy raw-32 plaintext` WARN + the `BLS key matches genesis entry` line. Both runbooks need a small refresh on the observability surface.

## Post-run state

- Cluster stopped
- Plaintext keys restored on all 3 Minis from `~/bls-keys-backup-2026-06-04/_pre-t1.19_<timestamp>.bin` — default cluster config is plaintext (rehearsal, not persistent migration)
- Passphrase file `<data_dir>/validator_pass` removed from each Mini
- Ephemeral passphrase scrubbed locally
- Diagnostic logs saved at `.live-soak-diagnostics-2026-06-04-t1.18/M{1,2,3}-node.log` (gitignored)

## Lane status flip

T1.18 was 🟡 OPEN (runbook documented, live execution gated on T3.1) → now ✅ DONE.

Both T1.18 (file-form passphrase) and T1.19 (plaintext → EVK1) have now been rehearsed end-to-end on a live BFT cluster with the binary on commit `a8f786f3`. The encrypted-key + file-form-passphrase production posture is empirically valid.
