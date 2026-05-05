# Validator Passphrase Migration Runbook

**Audience:** validator operators running EvaporChain in production.
**Severity:** non-blocking (legacy env-var still works), but recommended.
**Time:** ~5 minutes per node.

## Why migrate

The legacy `EVAPORCHAIN_VALIDATOR_KEY_PASS=<value>` env var is visible to any process owned by the same user via `/proc/<pid>/environ` on Linux. Any sibling service compromised on the same host can read the BLS-key passphrase by walking `/proc`.

The new `EVAPORCHAIN_VALIDATOR_KEY_PASS_FILE=<path>` reads the passphrase from a mode-0600 file on disk. The file is still readable by the same user, but it's not exposed via `/proc/.../environ` and not echoed by `ps -E`.

Both forms are accepted by the node binary; the file form takes precedence when both are set.

## Migration

```sh
# 1. Create the passphrase file with mode 0600.
PASS_DIR=/etc/evaporchain
sudo mkdir -p "$PASS_DIR"
sudo chmod 0700 "$PASS_DIR"

# Write the passphrase WITHOUT a trailing newline. printf is preferred
# over echo because echo on some shells appends a literal '\n'.
printf '%s' "$EVAPORCHAIN_VALIDATOR_KEY_PASS" | sudo tee "$PASS_DIR/validator_pass" > /dev/null
sudo chmod 0600 "$PASS_DIR/validator_pass"
sudo chown root:root "$PASS_DIR/validator_pass"   # adjust to your service-user

# 2. Verify the file content matches the env var.
diff <(printf '%s' "$EVAPORCHAIN_VALIDATOR_KEY_PASS") <(sudo cat "$PASS_DIR/validator_pass")
# (no output = identical)

# 3. Update the systemd unit (or whatever supervises the node).
#    Replace:
#       Environment=EVAPORCHAIN_VALIDATOR_KEY_PASS=...
#    With:
#       Environment=EVAPORCHAIN_VALIDATOR_KEY_PASS_FILE=/etc/evaporchain/validator_pass
sudo systemctl edit evaporchain-validator   # or vi /etc/systemd/system/evaporchain-validator.service
sudo systemctl daemon-reload

# 4. Restart the node.
sudo systemctl restart evaporchain-validator
sudo journalctl -u evaporchain-validator -n 200 --no-pager

# 5. Confirm the node decrypted the BLS key successfully.
#    Look for one of these lines in the log:
#      BLS validator key encrypted at rest (Argon2id+XChaCha20-Poly1305, path-bound AAD)
#    or
#      BLS12-381 keypair loaded from disk (pk=48B)
```

## Rollback

If the file form fails to load, the node falls back to the legacy env var (when both are set). To roll back fully:

```sh
sudo systemctl stop evaporchain-validator
sudo rm /etc/evaporchain/validator_pass
# Restore the legacy env var in your unit file.
sudo systemctl restart evaporchain-validator
```

## Verification

The new code path emits a distinct startup log line when the file form is used:

```
… BLS validator key encrypted at rest (Argon2id+XChaCha20-Poly1305, path-bound AAD)
```

If you instead see plain `BLS12-381 keypair loaded from disk`, the file wasn't applied and the node is still using the legacy env var (or the key is plaintext).

## Notes

- The passphrase file is read by `evaporchain_crypto::bls_key_store::passphrase_from_env()`. A trailing `\n` or `\r\n` is stripped automatically — `echo "secret" > pass.txt` works as expected.
- The file path itself is also AAD-bound to the encrypted BLS key (audit H5). Moving an EVK1 ciphertext to a different file path will fail decryption.
- Argon2id parameters are pinned to OWASP 2024 baseline via the named constants `ARGON2_M_COST_KIB`, `ARGON2_T_COST`, `ARGON2_P_COST`, `ARGON2_OUT_LEN` in `bls_key_store.rs` so a future crate-default downgrade won't silently regress security.
