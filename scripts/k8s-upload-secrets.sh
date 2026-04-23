#!/usr/bin/env bash
# Upload validator TLS certificates to Kubernetes secrets.
#
# Prerequisites:
#   1. Run scripts/generate-tls-certs.sh first
#   2. kubectl configured with cluster access
#   3. Namespace 'evaporchain' exists (kubectl apply -f deploy/k8s/namespace.yaml)
#
# Usage:
#   ./scripts/k8s-upload-secrets.sh [NUM_VALIDATORS] [CERTS_DIR]

set -euo pipefail

NUM_VALIDATORS="${1:-4}"
CERTS_DIR="${2:-./tls-certs}"
NAMESPACE="evaporchain"

echo "=== Uploading EvaporChain secrets to Kubernetes ==="
echo "Namespace: $NAMESPACE"
echo "Validators: $NUM_VALIDATORS"
echo "Certs dir: $CERTS_DIR"
echo

# Upload CA certificate
if [ -f "$CERTS_DIR/ca-cert.pem" ]; then
    echo "--- Uploading CA certificate ---"
    kubectl create secret generic evaporchain-ca \
        --from-file=ca-cert.pem="$CERTS_DIR/ca-cert.pem" \
        -n "$NAMESPACE" \
        --dry-run=client -o yaml | kubectl apply -f -
    echo "  Done."
else
    echo "WARNING: $CERTS_DIR/ca-cert.pem not found. Run scripts/generate-tls-certs.sh first."
    exit 1
fi

# Upload per-validator keys
for i in $(seq 0 $((NUM_VALIDATORS - 1))); do
    name="validator${i}"
    echo "--- Uploading keys for $name ---"

    ARGS="--from-file=validator-cert.pem=$CERTS_DIR/${name}-cert.pem"
    ARGS="$ARGS --from-file=validator-key.pem=$CERTS_DIR/${name}-key.pem"

    kubectl create secret generic "validator-${i}-keys" \
        $ARGS \
        -n "$NAMESPACE" \
        --dry-run=client -o yaml | kubectl apply -f -
    echo "  Done."
done

echo
echo "=== All secrets uploaded. Verify with: ==="
echo "  kubectl get secrets -n $NAMESPACE"
