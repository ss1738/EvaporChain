#!/bin/bash
# Launch the 3-Mini Tailscale colo cluster (T3.1 zero-cost interim path).
# Operates on M1 (validator-1) / M2 (validator-2) / M3 (validator-3) via SSH.
#
# Pre-conditions:
#   - Release binary at ~/EvaporChain/target/release/evaporchain-node on each Mini
#   - Genesis file at ~/EvaporChain/genesis-tailscale-3node.json on each Mini
#   - Data dir at ~/.evaporchain-tailscale-3node-data/ with bls_key.bin containing
#     the validator-N secret key matching genesis's validator-N public key
#
# Acceptance:
#   - All 3 nodes report identical light_cone_block_count + last_conservation_audit_ok=true
#   - BLS CommitCertificate per block: 3 signers, stake=750000/750000 (100%)
#
# To stop: pkill -f 'evaporchain-node.*tailscale-3node' on each Mini.

set -euo pipefail

for spec in "satyawansingh@100.119.53.101:M1:1" \
            "satyawan-mini-1@100.113.253.72:M2:2" \
            "satyawan-mini-2@100.103.216.125:M3:3"; do
  host="${spec%%:*}"
  rest="${spec#*:}"
  label="${rest%%:*}"
  vid="${rest##*:}"
  ssh -o IdentitiesOnly=yes -i ~/.ssh/id_ed25519 "$host" "
    cd ~/EvaporChain
    nohup ./target/release/evaporchain-node \
      --network --api --api-port 8081 --mock-prove \
      --genesis-config ./genesis-tailscale-3node.json \
      --validator-id $vid --validators 3 --node-id node-$vid \
      --port 9000 --data-dir ~/.evaporchain-tailscale-3node-data \
      --interval 8000 \
      --bootstrap /ip4/100.119.53.101/tcp/9000 \
      --bootstrap /ip4/100.113.253.72/tcp/9000 \
      --bootstrap /ip4/100.103.216.125/tcp/9000 \
      > /tmp/evaporchain-node.log 2>&1 &
    echo '$label launched: PID '\$!
  " &
done
wait
