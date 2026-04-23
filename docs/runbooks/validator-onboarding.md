# Validator Onboarding Runbook

## Prerequisites
- Linux amd64/arm64 or macOS (arm64)
- 2 CPU cores, 2GB RAM, 20GB disk minimum
- Open ports: 26656 (P2P), 8080 (API)

## Steps

### 1. Download binary
```bash
# From GitHub releases
wget https://github.com/ss1738/EvaporChain/releases/latest/download/evaporchain-linux-amd64.tar.gz
tar xzf evaporchain-linux-amd64.tar.gz
chmod +x evaporchain-linux-amd64
sudo mv evaporchain-linux-amd64 /usr/local/bin/evaporchain-node
```

### 2. Generate TLS certificates
```bash
./scripts/generate-tls-certs.sh 1 ./certs
```

### 3. Get genesis config
```bash
# Use the network-appropriate config
cp configs/staging.json genesis.json
# Or download from existing validator
curl http://<existing-validator>:8080/api/genesis > genesis.json
```

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
