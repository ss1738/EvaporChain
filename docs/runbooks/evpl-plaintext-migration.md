# EVPL Plaintext Key Migration Runbook

**Lane T1.19** — EVPL plaintext key migration on live nodes. Migrate legacy raw-32-byte BLS keys on disk to the EVPL magic-tagged format (or directly to EVK1 encrypted, recommended).

Pairs with: `MAINNET_READINESS.md` T1.19, `docs/runbooks/validator-passphrase-migration.md` (passphrase env-var → file), `docs/runbooks/bls-key-rotation.md` (rotating the BLS keypair itself).

---

## Background — three on-disk formats

`crates/evaporchain-crypto/src/bls_key_store.rs` recognises three BLS-secret file formats:

| Format | Length | Magic | Encryption |
|---|---|---|---|
| `LegacyRaw` | 32 bytes | none | none (mode 0600 only) |
| `PlaintextMagic` (EVPL) | 36 bytes | `b"EVPL"` | none |
| `Encrypted` (EVK1) | 92 bytes | `b"EVK1"` | Argon2id KDF + XChaCha20-Poly1305 |

`detect_bls_key_format(bytes)` classifies the file. The node binary auto-detects on startup; all three load correctly. But the legacy 32-byte form is the **audit Crypto-6 footgun** — a 32-byte ciphertext fragment can be misclassified as a plaintext key if the format isn't tagged. EVPL closes that by requiring the magic prefix.

**Production should use EVK1**. EVPL is a transitional format for operators who cannot yet adopt passphrase-based encryption.

---

## Which format does your node currently use?

```bash
ssh <node> 'stat -c %s ~/.evaporchain-tailscale-data/bls_key.bin'
```

| Size | Format | Action |
|---|---|---|
| 32 | `LegacyRaw` | Migrate (this runbook) |
| 36 | `PlaintextMagic` | Already migrated; consider upgrading to EVK1 |
| 92 | `Encrypted` | No action — already at the highest format |
| anything else | malformed | **STOP** — the node won't start. Restore from backup. |

If you see 32 bytes, the legacy raw format is in use. Proceed with the migration below.

---

## Migration paths

Two destinations:
- **EVPL (plaintext+magic)** — same security as legacy raw, but Crypto-6-safe. No passphrase required. Use when you cannot yet manage a passphrase.
- **EVK1 (encrypted)** — recommended. Adds Argon2id + XChaCha20-Poly1305. Requires a passphrase (env-var or file per `validator-passphrase-migration.md`).

Pick one. The runbook covers both.

---

## Pre-flight

1. **Backup the current key file.** Mandatory.

   ```bash
   ssh <node> 'cp ~/.evaporchain-tailscale-data/bls_key.bin ~/.evaporchain-tailscale-data/bls_key.bin.preformat-$(date +%s)'
   ```

2. **Confirm size = 32 bytes** (not 36, not 92). If the file is already EVPL or EVK1, this runbook does not apply.

3. **Plan the cutover window.** Migration is a stop-restart of the node binary, ~10-30s of validator downtime. Will not cross slashing thresholds. Do it during a low-traffic window or while another validator carries the load.

4. **Cluster lockstep + no finality stall** — same pre-flight as the other key-management runbooks. Do not migrate when the cluster is degraded.

---

## Path A — migrate to EVPL (plaintext+magic)

Simplest path. The on-disk bytes change from `<32 raw>` to `b"EVPL" || <32 raw>` (36 bytes total).

### Step 1 — generate the EVPL file

```bash
ssh <node> 'python3 -c "
import sys
raw = open(\"$HOME/.evaporchain-tailscale-data/bls_key.bin\", \"rb\").read()
assert len(raw) == 32, f\"expected 32 bytes, got {len(raw)}\"
out = b\"EVPL\" + raw
open(\"$HOME/.evaporchain-tailscale-data/bls_key_NEW.bin\", \"wb\").write(out)
"'
```

### Step 2 — verify the new file

```bash
ssh <node> 'stat -c %s ~/.evaporchain-tailscale-data/bls_key_NEW.bin && head -c 4 ~/.evaporchain-tailscale-data/bls_key_NEW.bin'
```

Expect `36` (size) and `EVPL` (magic).

### Step 3 — atomic swap + restart

```bash
ssh <node> "
  systemctl stop evaporchain
  chmod 0600 ~/.evaporchain-tailscale-data/bls_key_NEW.bin
  mv ~/.evaporchain-tailscale-data/bls_key.bin     ~/.evaporchain-tailscale-data/bls_key_OLD_RAW.bin
  mv ~/.evaporchain-tailscale-data/bls_key_NEW.bin ~/.evaporchain-tailscale-data/bls_key.bin
  systemctl start evaporchain
"
```

### Step 4 — verify the node loaded the new format

The node logs `BLS key loaded (format: PlaintextMagic, EVPL)` at startup. Check:

```bash
ssh <node> 'journalctl -u evaporchain -n 100 | grep "BLS key loaded"'
```

If the log says `LegacyRaw`, the new file isn't in place — investigate. If the log says `Encrypted (EVK1)`, you accidentally used the EVK1 path; double-check Step 1.

Confirm the validator is back in the active set within 60s:

```bash
curl -fsS -H "Authorization: Bearer $EVAPORCHAIN_ADMIN_KEY" \
     "http://<node>:8081/metrics" | grep evap_active_validators
```

---

## Path B — migrate to EVK1 (encrypted, recommended)

The on-disk bytes change from `<32 raw>` to `b"EVK1" || salt(16) || nonce(24) || ciphertext(48)` (92 bytes total). Requires a passphrase.

### Step 1 — set up the passphrase

Follow `docs/runbooks/validator-passphrase-migration.md` to install `EVAPORCHAIN_VALIDATOR_KEY_PASS_FILE` on the node. Confirm the file is mode 0600 and readable by the node's service user.

### Step 2 — generate the EVK1 file

Use the `evaporchain-cli encrypt-bls-key` subcommand (mentioned in `crates/evaporchain-crypto/src/bls_key_store.rs:17`):

```bash
ssh <node> "EVAPORCHAIN_VALIDATOR_KEY_PASS_FILE=/etc/evaporchain/validator_pass \
            evaporchain-cli encrypt-bls-key \
              --in  ~/.evaporchain-tailscale-data/bls_key.bin \
              --out ~/.evaporchain-tailscale-data/bls_key_NEW.bin"
```

If the CLI subcommand doesn't exist on your binary version, the Rust API `encrypt_bls_secret_with_aad(secret, passphrase, path_aad(file_path))` does the same thing — build a small helper binary that wraps it.

### Step 3 — verify the new file

```bash
ssh <node> 'stat -c %s ~/.evaporchain-tailscale-data/bls_key_NEW.bin && head -c 4 ~/.evaporchain-tailscale-data/bls_key_NEW.bin'
```

Expect `92` (size) and `EVK1` (magic).

### Step 4 — atomic swap + restart (same as Path A Step 3)

### Step 5 — verify the node loaded the encrypted format

```bash
ssh <node> 'journalctl -u evaporchain -n 100 | grep "BLS key loaded"'
```

Expect `BLS key loaded (format: Encrypted, EVK1)`. If the passphrase file is unreadable, the node fails to start with a clear error — fix permissions and retry.

---

## Rollback

If the validator doesn't come back online within 60s after the restart, swap back to the legacy file:

```bash
ssh <node> "
  systemctl stop evaporchain
  mv ~/.evaporchain-tailscale-data/bls_key.bin           ~/.evaporchain-tailscale-data/bls_key_FAILED_NEW.bin
  mv ~/.evaporchain-tailscale-data/bls_key_OLD_RAW.bin   ~/.evaporchain-tailscale-data/bls_key.bin
  systemctl start evaporchain
"
```

This restores the original raw 32-byte key. The validator should rejoin within 60s. Then investigate why the migration failed — typically: wrong file size, wrong magic bytes (path A), unreadable passphrase file (path B), or wrong file ownership.

---

## Post-migration cleanup

After 24h of healthy operation with the new format:

```bash
ssh <node> "shred -u ~/.evaporchain-tailscale-data/bls_key_OLD_RAW.bin && \
            shred -u ~/.evaporchain-tailscale-data/bls_key.bin.preformat-*"
```

Use `shred` — the raw key bytes may have already been swapped to disk by the kernel; a plain `rm` leaves them recoverable.

---

## Cross-references

- `MAINNET_READINESS.md` T1.19
- `crates/evaporchain-crypto/src/bls_key_store.rs:268` — `detect_bls_key_format` classifier
- `crates/evaporchain-crypto/src/bls_key_store.rs:235` — `format_plaintext_for_disk` (EVPL)
- `crates/evaporchain-crypto/src/bls_key_store.rs:134` — `encrypt_bls_secret_with_aad` (EVK1 with path-binding)
- `docs/runbooks/validator-passphrase-migration.md` — passphrase env-var → file (separate operation)
- `docs/runbooks/bls-key-rotation.md` — rotating the BLS keypair itself (separate operation)
