# evaporchain-wallet

Post-quantum CLI wallet for EvaporChain. ML-DSA-65 (FIPS 204 / Dilithium3) signatures, AES-256-GCM keystore, end-to-end tested against the live 5-node WAN cluster.

**Last updated:** 2026-05-08, after the session that landed `account import [--address-override]` / `account export` / persistent active account / list with live balance refresh.

---

## Build

```bash
cd ~/EvaporChain
cargo build --release -p evaporchain-wallet
```

Binary lands at `target/release/evaporchain-wallet`. Single static-linked Rust binary.

## Quick start (genesis-allocated account)

If you have a `validator-N-keys.json` bundle from the genesis ceremony and want the wallet to control that account:

```bash
# Import V1's key, pinning the wallet to the genesis-allocated address
# (which is hand-picked, NOT derived from the pubkey).
wallet --node http://your-node:8081 \
    account import validator-1 ~/validator-1-keys.json \
    --address-override 0x0100000000000000000000000000000000000000000000000000000000000000

# Verify on-chain balance
wallet --node http://your-node:8081 account list
#   *validator-1    0x0100...0000     20198 EVAP    nonce=9

# Send EVAP
wallet --node http://your-node:8081 \
    send 0x0200000000000000000000000000000000000000000000000000000000000000 25 --wait
#   Tx Hash: d2979c0a...
#   CONFIRMED Confirmed in block #17300
```

## Quick start (fresh account)

```bash
wallet --node http://your-node:8081 account create alice
# Address: 0x655278d7...

wallet --node http://your-node:8081 account list
#   *alice          0x655278d7...               ?       nonce=?

# Once funded (via faucet or transfer-in), balance shows live.
```

---

## Account management

| Command | Purpose |
|---|---|
| `account create <NAME>` | Generate fresh ML-DSA-65 keypair. |
| `account import <NAME> <FILE> [--address-override 0x…]` | Import existing keypair from JSON. Use `--address-override` for genesis-allocated accounts whose addresses ≠ `hash(pubkey)`. |
| `account export <NAME> <FILE>` | Round-trip-able plaintext export. Refuses overwrite; mode 0600 on Unix. |
| `account list` | Show all accounts with live on-chain balance + nonce. Falls back to cached values if node unreachable. |
| `account switch <NAME>` | Change active account. Persisted across wallet invocations. |
| `account balance [NAME]` | Show balance + nonce for one account. |
| `account detail [NAME]` | Show balance + nonce + objects + NFTs + tokens. |

### About `--address-override`

The wallet derives addresses as `blake3(public_key)` by default. But `genesis-mainnet.json` allocations hand-pick addresses (e.g. `[1, 0, 0, ...]` for validator-1's operator account) that don't correspond to any pubkey hash. The chain accepts signed transactions where the `from` field decouples from `hash(public_key)` — but the wallet needs to know about that decoupling to route transactions correctly.

Use `--address-override` when:
- You have a genesis-ceremony `validator-N-keys.json` bundle.
- The on-chain address you want to control was hand-picked (not derived).

Don't use `--address-override` for:
- Fresh accounts you create.
- Backups of accounts you originally created via `account create`.

## Transactions

| Command | Purpose |
|---|---|
| `send <TO> <AMOUNT> [--wait]` | Sign + submit a TransferTx from the active account. `--wait` polls until confirmed (~30s timeout). |
| `faucet` | Request testnet tokens. **Currently unavailable on the running cluster** — admin endpoint requires `EVAPORCHAIN_ADMIN_KEY` env var which the launch script doesn't set. |
| `refresh <OBJECT_ID> <ENERGY>` | Top up an object's energy. |
| `objects` | List all state objects. |
| `object <ID>` | Show one object's detail. |

## Validators + delegation

| Command | Purpose |
|---|---|
| `stake validate <ID> <AMOUNT>` | Bond stake to become / update a validator. |
| `stake delegate <ID> <AMOUNT>` | Delegate stake to a validator. |
| `stake undelegate <ID> <AMOUNT>` | Begin unbonding. Funds locked until unbonding period elapses. |
| `stake claim <ID>` | Claim previously-undelegated funds. |

## Tokens + NFTs

| Command | Purpose |
|---|---|
| `token list` / `token show <ID>` / `token deploy ...` | Token operations. |
| `nft list` / `nft show <ID>` / `nft mint ...` / `nft transfer <ID> <TO>` | NFT operations. |

## DAO + governance

| Command | Purpose |
|---|---|
| `dao` (subcommands) | Governance proposals + voting. |

## Backup + recovery

| Command | Purpose |
|---|---|
| `backup` (subcommands) | Encrypted keystore export / restore. |
| `seed generate` | Generate 24-word mnemonic. |
| `seed backup <NAME> <FILE>` | Backup a keypair under a seed. |
| `seed recover <FILE> <NAME>` | Recover from seed phrase + backup file. |
| `account export <NAME> <FILE>` | Plaintext keypair backup (alternative to `seed`). |

## Operational + utility

| Command | Purpose |
|---|---|
| `history` | Transaction history (local cache). |
| `contacts` (subcommands) | Address book — assign names to addresses for `send`. |
| `gas` (subcommands) | Gas estimation + fee analysis. |
| `config` (subcommands) | Wallet configuration. |
| `dashboard` | Multi-account portfolio overview. |
| `watch <ADDRESS>` | Read-only address watching (no keys needed). |
| `interactive` | Guided mode for new users. |
| `version` | Build info. |
| `doctor` | Self-diagnostic checks. |

## Advanced

| Command | Purpose |
|---|---|
| `offline sign / sign-refresh ...` | Cold-wallet / air-gapped signing. |
| `offline broadcast <FILE>` | Broadcast a pre-signed transaction. |
| `batch <FILE> [--dry-run]` | Execute a batch of transactions from JSON. |

## Global options

| Flag | Default | Purpose |
|---|---|---|
| `--node URL` | `http://localhost:3000` | RPC endpoint. |
| `--keystore PATH` | `~/.evaporchain/keystore.json` | Keystore file. |
| `--json` | (off) | Output as JSON for scripts / bots. |

## Environment

| Variable | Purpose |
|---|---|
| `EVAPORCHAIN_PASSWORD` | Skip password prompts in CI / scripts. **Plaintext password in env — use only in trusted contexts.** |

## Keystore format

```json
{
  "version": 1,
  "active": "validator-1",
  "entries": [
    {
      "name": "validator-1",
      "address": "0x0100000000000000000000000000000000000000000000000000000000000000",
      "public_key": "<3904-char hex = 1952-byte ML-DSA-65 pk>",
      "encrypted_secret_key": "<hex AES-256-GCM ciphertext>",
      "nonce": "<24-char hex = 12-byte AES nonce>",
      "salt": "<64-char hex = 32-byte Argon2id salt>",
      "created_at": "2026-05-08T..."
    }
  ]
}
```

- `active` is persisted (set by `account create` / `account import` / `account switch`; consumed by `send`, `stake`, etc).
- `address` is the on-chain address; `--address-override` at import time pins a value that decouples from `hash(public_key)`.
- Secret keys never leave the file in plaintext except via `account export` (creates a separate `.json` file with mode 0600 and a loud WARN).

## Cryptography

- **Signatures:** ML-DSA-65 (FIPS 204 / Dilithium3) — 1952-byte public keys, 4000-byte secret keys, 3293-byte signatures. Post-quantum hardness against Shor + Grover.
- **Keystore encryption:** AES-256-GCM with a key derived from the user's password via Argon2id (64 MiB memory, 3 iterations, 1 thread — OWASP-recommended).
- **Address derivation (default):** `blake3(public_key)`, truncated to 32 bytes.
- **Hybrid mode:** `WalletSigner::from_hybrid` available for ECDSA+ML-DSA hybrid signatures. ML-DSA-only is the default for legacy compatibility.

## Cross-references

- `crates/evaporchain-crypto/src/signatures.rs` — `MlDsaKeypair` + `HybridKeypair` impls.
- `crates/evaporchain-types/src/lib.rs` — `Account`, `Transaction`, `TransferTx`.
- `crates/evaporchain-node/src/api.rs` — chain HTTP endpoints the wallet talks to (`/api/tx/transfer`, `/api/tx/signable`, `/api/account/:addr`, `/api/tx/nonce/:addr`).
- `genesis-mainnet.json` — reference allocation with hand-picked addresses; `--address-override` is the wallet's path to use those.
- `TOKENOMICS.md` — economic surface (vesting locks, fee burn, staker share). Vesting affects `transferable_balance` which `send` honors.
- `/tmp/evp-transfer-tool/` (built ad-hoc 2026-05-07) — minimal standalone signer alternative; superseded by `wallet account import --address-override` + `wallet send`.

## Status

Production-shaped for basic flows (create / import / export / send / list / switch) as of 2026-05-08. End-to-end validated against the running 5-node WAN testnet — wallet-signed V1→V2 transfer of 25 EVP confirmed at block 17300, TX hash `d2979c0af9e294b3f0a97dd26eb79152ccf45bbf6581c0209030d0d3da050acd`.

Areas not yet exercised end-to-end against live cluster: stake / delegate / undelegate, token / NFT mint, contract deploy / call, offline sign / broadcast, batch, dashboard.
