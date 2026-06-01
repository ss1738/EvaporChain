# Validator Onboarding Runbook

How a new EvaporChain validator joins a multi-node cluster (mainnet or
production testnet) using the coordinator-signed genesis-config flow that
closes audit findings K-07/K-08.

The model: **one** coordinator produces **one** signed
`genesis-config.json`. Every validator passes that exact file to its node
via `--genesis-config <path>`. The node verifies the signature on startup
and refuses to bootstrap against a tampered or unsigned config.

**See also**:
- [`docs/MAINNET_LAUNCH.md`](MAINNET_LAUNCH.md) — the `--mainnet` strict-mode
  boot path; the 11 pre-flight checks the binary refuses to skip and how
  to satisfy each one.
- [`docs/GENESIS_CEREMONY.md`](GENESIS_CEREMONY.md) — the once-per-launch
  ceremony that produces the signed `genesis-config.json` this runbook
  consumes. Post-genesis validator joining (this runbook) is a different
  shape than launch-time validator participation (covered there).

## Roles

- **Coordinator** — runs the genesis ceremony. Holds the ML-DSA-65 secret
  key. Distributes the signed `genesis-config.json` and the matching
  `coordinator-pk.hex` out-of-band (signed email, encrypted chat,
  PGP-signed Git tag, etc.).
- **Validator operator** — submits their BLS pubkey to the coordinator,
  receives the signed genesis-config, runs the node.

## 1. Validator generates a key bundle

Each operator runs this **once** on the machine that will operate the
validator:

```bash
evaporchain keygen --output validator-keys.json
```

This produces a JSON bundle with BLS, ML-DSA, and VRF keypairs. The file
is written `0600`. **Never share `validator-keys.json` itself.**

Extract the BLS public key for the coordinator:

```bash
jq -r '.bls.public_key' validator-keys.json > my-validator-bls-pk.hex
```

Send the operator’s `id` (assigned by the coordinator), moniker, BLS
pubkey hex, requested stake, and (optional) libp2p multiaddress to the
coordinator over a channel you both trust.

## 2. Coordinator generates their key

```bash
mkdir -p coordinator/
evaporchain onboarding generate-coordinator --out-dir coordinator/
```

This writes:

- `coordinator/coordinator-pk.hex` — public key. Distribute with the
  signed genesis-config.
- `coordinator/coordinator-sk.hex` — secret key, `0600`. Keep offline,
  used only to run `build-genesis`.

## 3. Coordinator collects validator pubkeys

The coordinator assembles every validator’s entry into a JSON manifest:

```jsonc
{
  "validators": [
    {
      "id": 1,
      "name": "alpha",
      "bls_public_key": "<48-byte hex>",
      "stake": 250000,
      "balance": 50000000,
      "p2p_address": "/ip4/203.0.113.10/tcp/26656"
    },
    {
      "id": 2,
      "name": "beta",
      "bls_public_key": "<48-byte hex>",
      "stake": 250000,
      "balance": 50000000
    }
  ],
  "allocations": [
    {
      "address": "a000000000000000000000000000000000000000000000000000000000000000",
      "balance": 350000000,
      "label": "Foundation Treasury"
    }
  ]
}
```

Save it as `validators-manifest.json`.

## 4. Coordinator builds the signed genesis-config

```bash
evaporchain onboarding build-genesis \
  --validators validators-manifest.json \
  --coordinator-sk coordinator/coordinator-sk.hex \
  --chain-id evaporchain-mainnet-1 \
  --output genesis-config.json \
  --block-interval-ms 2000 \
  --total-supply 1000000000 \
  --min-stake 100000
```

The `--chain-id` argument's canonical values live at
`evaporchain_types::chain_ids` — `MAINNET = "evaporchain-mainnet-1"`,
`TESTNET = "evaporchain-testnet-1"`, `DEVNET = "evaporchain-devnet-1"`.
Chain-id is bound into the BLS signing message, the VRF leader-selection
input, the paymaster sponsorship payload, and the gossipsub topic
namespace — a one-character typo silently creates a partition, so prefer
the constants over typing the literal string.

The coordinator-sk is read from disk; its paired `coordinator-pk.hex`
must sit in the same directory. The command:

1. Validates the manifest (BLS hex length, unique ids, stake floor).
2. Builds the full `GenesisConfig` (chain params, tokenomics, validator
   set, allocations).
3. Runs `GenesisConfig::validate()` and refuses to sign on any error.
4. Signs the canonical bytes with ML-DSA-65 and embeds the hex signature
   plus the coordinator’s public key in the JSON.

Distribute `genesis-config.json` and `coordinator/coordinator-pk.hex` to
every operator over a channel that proves authenticity (PGP-signed
release, signed Git tag, SSH-protected file drop). The signature inside
the file does not authenticate the *coordinator* itself — only that the
file matches what the coordinator produced.

## 5. Every operator verifies before launch

Always before running the node:

```bash
evaporchain onboarding verify \
  --genesis genesis-config.json \
  --coordinator-pk coordinator-pk.hex
```

Exit code is `0` on a valid signature, `1` on any failure (missing
signature, mismatched pk, tampered field). Re-fetch from the coordinator
if it fails — never patch a broken file by hand.

## 6. Operator launches the node

```bash
# --mainnet strict mode requires both env vars set, non-empty,
# non-dev-default. The binary refuses to boot otherwise.
export EVAPORCHAIN_KEY_MASTER="<32+ hex chars from /dev/urandom>"
export EVAPORCHAIN_BLS_PASSPHRASE="<this validator's own EVPL passphrase>"

evaporchain-node \
  --mainnet \
  --network --tendermint --tls \
  --port 26656 \
  --api --api-port 8080 \
  --node-id "validator-alpha" \
  --validator-id 1 \
  --validators 4 \
  --data-dir /var/lib/evaporchain \
  --genesis-config /etc/evaporchain/genesis-config.json \
  --bootstrap /ip4/<peer-ip>/tcp/26656
```

The node re-runs the same coordinator-signature check on startup. In
`--mainnet` strict mode it additionally requires `coordinator_pk` in
the genesis to match the binary's baked-in `MAINNET_COORDINATOR_PK_BYTES`
constant (at `crates/evaporchain-node/src/main.rs:1418`), so a forked
binary cannot accept a different coordinator.

The full set of pre-flight checks the binary refuses to skip in
`--mainnet` mode is documented in [`MAINNET_LAUNCH.md`](MAINNET_LAUNCH.md)
§3 — 11 checks aggregated into a single boot-time error message if any
violate.

For a non-mainnet (testnet/staging) cluster, drop the `--mainnet` flag.
The node still verifies the signature, but accepts whatever
`coordinator_pk` the genesis itself supplies — useful for ephemeral
local clusters.

## 7. Operator confirms the node is healthy

```bash
curl http://localhost:8080/readyz
curl http://localhost:8080/api/status | jq .
```

## What goes wrong

- **`signature did not verify`** at `onboarding verify` or node startup
  → the file was edited after signing. Re-fetch from the coordinator.
- **`coordinator_pk in genesis does not match baked-in MAINNET_COORDINATOR_PK_BYTES`**
  → either you're running a binary that hasn't been updated for this
  network, or the genesis was signed by the wrong coordinator. Stop and
  contact the coordinator before doing anything else.
- **`EVAPORCHAIN_KEY_MASTER must be set in --mainnet mode`** or
  **`EVAPORCHAIN_KEY_MASTER is set to the dev default`** or
  **`EVAPORCHAIN_KEY_MASTER must be at least 16 chars`**
  → the master key env var failed `--mainnet` strict-mode pre-flight.
  Generate a fresh random value: `head -c 32 /dev/urandom | xxd -p -c 0`,
  then `export EVAPORCHAIN_KEY_MASTER=<value>` and retry. Never re-use
  across operators (each validator's master key is independent).
- **`EVAPORCHAIN_VALIDATOR_KEY_PASS must be set (non-empty) so the
  validator BLS key can be encrypted at rest`**
  → the EVPL key-encryption passphrase isn't set. Generate via a
  similar random source and `export EVAPORCHAIN_BLS_PASSPHRASE=<value>`.
  The full set of `--mainnet` strict-mode pre-flight errors is
  documented in [`MAINNET_LAUNCH.md`](MAINNET_LAUNCH.md) §3.
- **`validator-id N not in genesis validator set`** → coordinator forgot
  to include your entry, or you're using the wrong `--validator-id`.
  Check `jq '.validators[].id' genesis-config.json`.
- **`chain_id` mismatch on the wire** → another operator launched with
  a stale genesis-config. Confirm everyone has the same file by hashing
  it: `sha256sum genesis-config.json`.
- **Lost validator key** → there is no recovery. Generate a new bundle
  and ask the coordinator for a key-rotation patch genesis (next
  upgrade only, not mid-run).
- **Lost coordinator-sk** → no future genesis-config can be produced
  under that key. The cluster keeps running on the existing genesis;
  for the next network start the coordinator generates a new keypair
  and every operator re-fetches the new `coordinator-pk.hex`.

## Verifying out-of-band integrity

Operators should compare the SHA-256 of `genesis-config.json` over a
side-channel (Signal, signed mailing-list post) before launch. The
ML-DSA signature catches tampering by anyone without the coordinator
secret; the side-channel hash catches social-engineering attacks where
a wrong-but-signed file is substituted before distribution.

```bash
sha256sum genesis-config.json
```
