# Validator Onboarding Runbook

## Prerequisites
- Linux amd64/arm64 or macOS (arm64)
- 2 CPU cores, 2GB RAM, 20GB disk minimum
- Open ports: 26656 (P2P), 8080 (API)

## Steps

### 1. Build binary

There is no GitHub Release artefact today. Build from source on the
target host (or any host matching its OS/arch):

```bash
git clone https://github.com/ss1738/EvaporChain.git
cd EvaporChain
cargo build --release -p evaporchain-node --features prove
sudo install -m 0755 target/release/evaporchain-node /usr/local/bin/evaporchain-node
```

When a tagged release is published the GitHub Releases workflow at
`.github/workflows/release.yml` will produce
`evaporchain-{linux,mac}-{amd64,arm64}` archives. Until that lands,
build from source is the canonical install path.

### 2. Generate TLS certificates
```bash
./scripts/generate-tls-certs.sh 1 ./certs
```

### 3. Get genesis config

The canonical onboarding flow goes through the K-07/K-08
coordinator-signed ceremony (see `docs/VALIDATOR_ONBOARDING.md` for
the full flow):

```bash
# Coordinator builds the signed genesis once and distributes it.
# Each operator just verifies the bundle, never patches the JSON
# by hand. Re-fetch from the coordinator if your file fails to
# verify.
evaporchain onboarding verify --genesis-config /path/to/genesis.json
```

For ad-hoc / local devnets without a coordinator the
`evaporchain testnet init --validators N --per-validator-ips ...`
flow produces a self-signed genesis you can copy into place.

### 4. Start the node
```bash
evaporchain-node \
  --network --tendermint --tls \
  --port 26656 \
  --api --api-port 8080 \
  --node-id "my-validator" \
  --validator-id <ASSIGNED_ID> \
  --validators <TOTAL_COUNT> \
  --data-dir /data/evaporchain \
  --genesis-config genesis.json \
  --bootstrap /ip4/<PEER_IP>/tcp/26656
```

### 5. Verify
```bash
# Check node is running
curl http://localhost:8080/readyz

# Check peers
curl http://localhost:8080/api/status | jq '.peers'

# Check block production
watch -n 2 'curl -s http://localhost:8080/api/blocks/latest | jq .number'
```

## Troubleshooting
- **No peers**: Check firewall allows port 26656. Verify bootstrap address.
- **Consensus stuck**: All validators must be running same genesis. Check chain_id matches.
- **Disk full**: Run `scripts/backup-state.sh` then prune old data.
