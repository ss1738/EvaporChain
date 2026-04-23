#!/usr/bin/env bash
# Generate TLS certificates for an EvaporChain validator cluster.
#
# Usage:
#   ./scripts/generate-tls-certs.sh [NUM_VALIDATORS] [OUTPUT_DIR]
#
# Defaults: 4 validators, output to ./tls-certs/

set -euo pipefail

NUM_VALIDATORS="${1:-4}"
OUTPUT_DIR="${2:-./tls-certs}"

mkdir -p "$OUTPUT_DIR"

echo "=== EvaporChain TLS Certificate Generator ==="
echo "Validators: $NUM_VALIDATORS"
echo "Output dir: $OUTPUT_DIR"
echo

# Generate CA
echo "--- Generating CA certificate ---"
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 \
    -keyout "$OUTPUT_DIR/ca-key.pem" -out "$OUTPUT_DIR/ca-cert.pem" \
    -days 3650 -nodes \
    -subj "/CN=EvaporChain Validator CA/O=EvaporChain" 2>/dev/null
echo "  CA cert: $OUTPUT_DIR/ca-cert.pem"
echo "  CA key:  $OUTPUT_DIR/ca-key.pem"
echo

# Generate validator certs
for i in $(seq 0 $((NUM_VALIDATORS - 1))); do
    name="validator$i"
    echo "--- Generating certificate for $name ---"

    # Generate key and CSR
    openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 \
        -keyout "$OUTPUT_DIR/${name}-key.pem" -out "$OUTPUT_DIR/${name}.csr" \
        -nodes \
        -subj "/CN=${name}.evaporchain.local/O=EvaporChain" 2>/dev/null

    # Sign with CA
    openssl x509 -req -in "$OUTPUT_DIR/${name}.csr" \
        -CA "$OUTPUT_DIR/ca-cert.pem" -CAkey "$OUTPUT_DIR/ca-key.pem" \
        -CAcreateserial -out "$OUTPUT_DIR/${name}-cert.pem" \
        -days 365 2>/dev/null

    rm -f "$OUTPUT_DIR/${name}.csr"
    echo "  Cert: $OUTPUT_DIR/${name}-cert.pem"
    echo "  Key:  $OUTPUT_DIR/${name}-key.pem"
done

rm -f "$OUTPUT_DIR/ca-cert.srl"

echo
echo "=== Done. $NUM_VALIDATORS validator certificates generated. ==="
echo
echo "Usage with EvaporChain node:"
echo "  evaporchain-node --network --tls --tls-cert tls-certs/validator0-cert.pem --tls-key tls-certs/validator0-key.pem --tls-ca tls-certs/ca-cert.pem"
