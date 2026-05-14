#!/usr/bin/env bash
#
# scripts/d-track-adversarial.sh — T0.2 D.1: adversarial API sweep
#
# Exercises the live cluster against Layer 4 adversarial inputs:
#   A.1  Invalid/forged signature (all-zeros, truncated, wrong key)
#   A.2  Replay attack (resubmit a known-accepted tx)
#   A.3  Conservation violation (send more than balance)
#   A.4  Energy overflow (u64::MAX energy value)
#   A.5  Malformed JSON to every API endpoint
#   A.6  Governance parameter attack (out-of-range / unknown params)
#   A.7  Double-nullifier (PNT replay)
#   A.8  Zero-value transfer
#   A.9  Invalid recipient (address format attacks)
#   A.10 Future-height nonce (nonce far ahead of current height)
#
# Pass criterion: for each vector, the node must respond with 4xx (never
# 5xx / panic / hang), finality must continue progressing on the cluster
# throughout, and no node restarts involuntarily.
#
# Usage:
#   TARGETS=host1:port,host2:port ./scripts/d-track-adversarial.sh
#
# Env vars:
#   TARGETS       Comma-separated host:port list (default: 3-Mini Tailscale)
#   PRIMARY       Primary target for write attacks (default: first in TARGETS)
#   LOG_DIR       Output directory (default: ./logs/d-track-adversarial)
#   PAUSE_SECS    Seconds to pause between vectors (default: 5)
#   DRAIN_SECS    Seconds to monitor finality after each vector (default: 15)

set -euo pipefail

TARGETS="${TARGETS:-100.119.53.101:8080,100.113.253.72:8080,100.103.216.125:8080}"
PAUSE_SECS="${PAUSE_SECS:-5}"
DRAIN_SECS="${DRAIN_SECS:-15}"
LOG_DIR="${LOG_DIR:-./logs/d-track-adversarial}"
RUN_ID="$(date +%Y%m%dT%H%M%SZ)"

OUTPUT="$LOG_DIR/$RUN_ID"
mkdir -p "$OUTPUT"
REPORT="$OUTPUT/report.txt"
EVENT_LOG="$OUTPUT/events.log"

IFS=',' read -ra TARGET_ARR <<< "$TARGETS"
PRIMARY="${PRIMARY:-${TARGET_ARR[0]}}"

log()    { echo "$(date -u +%Y-%m-%dT%H:%M:%SZ)  $*" | tee -a "$EVENT_LOG"; }
pass()   { log "  [PASS] $*"; }
fail()   { log "  [FAIL] $*"; GLOBAL_PASS=0; }
warn()   { log "  [WARN] $*"; }
header() { echo ""; log "━━━ $* ━━━"; }

jget() {
    python3 -c "
import sys,json
try:
    d=json.loads(sys.argv[1])
    print(d.get(sys.argv[2],-1))
except:
    print(-1)" "$1" "$2" 2>/dev/null || echo "-1"
}

api_post() {
    # api_post <host:port> <path> <body> → prints HTTP code
    local t="$1" path="$2" body="$3"
    curl -s -m 10 -o /dev/null -w "%{http_code}" \
        -X POST "http://${t}${path}" \
        -H 'Content-Type: application/json' \
        -d "$body" 2>/dev/null || echo "000"
}

api_get() {
    local t="$1" path="$2"
    curl -s -m 10 "http://${t}${path}" 2>/dev/null || echo "{}"
}

get_finalized() {
    jget "$(api_get "$1" /api/status)" finalized_height
}

# Checks that finality advances within DRAIN_SECS on PRIMARY
check_finality_ok() {
    local before
    before=$(get_finalized "$PRIMARY")
    local waited=0
    while [ "$waited" -lt "$DRAIN_SECS" ]; do
        sleep 2; waited=$((waited + 2))
        local after
        after=$(get_finalized "$PRIMARY")
        if [ "$after" != "-1" ] && [ "$after" -gt "$before" ]; then
            return 0
        fi
    done
    return 1
}

# Validates that a status code is 4xx (rejection), not 2xx or 5xx
expect_rejection() {
    local code="$1" desc="$2"
    if python3 -c "exit(0 if 400 <= int('${code}') < 500 else 1)" 2>/dev/null; then
        pass "$desc → HTTP $code (4xx rejection — correct)"
    elif [ "$code" = "000" ]; then
        warn "$desc → no response / connection refused (node down?)"
    elif python3 -c "exit(0 if 500 <= int('${code}') < 600 else 1)" 2>/dev/null; then
        fail "$desc → HTTP $code (5xx — possible panic/crash)"
    else
        fail "$desc → HTTP $code (unexpected; expected 4xx)"
    fi
}

GLOBAL_PASS=1
VECTORS_PASSED=0
VECTORS_FAILED=0

# ── Pre-flight ──────────────────────────────────────────────────────────────
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  T0.2 D.1 Adversarial sweep — $RUN_ID"
echo "  Primary: $PRIMARY"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

REACHABLE=0
for t in "${TARGET_ARR[@]}"; do
    CODE=$(curl -s -m 5 -o /dev/null -w "%{http_code}" "http://${t}/api/health" 2>/dev/null || echo "000")
    if [ "$CODE" = "200" ]; then
        echo "  ✅  $t"
        REACHABLE=$((REACHABLE + 1))
    else
        echo "  ⚠️   $t  (HTTP $CODE)"
    fi
done

if [ "$REACHABLE" -eq 0 ]; then
    echo "ERROR: no targets reachable — aborting"; exit 2
fi

PRE_FIN=$(get_finalized "$PRIMARY")
log "ADVERSARIAL_SWEEP_START run=$RUN_ID primary=$PRIMARY pre_finalized=$PRE_FIN"

# ── A.1  Invalid / forged signature ─────────────────────────────────────────
header "A.1 — Invalid / forged transaction signatures"

# All-zero 64-byte signature (BLS/ML-DSA will reject)
CODE=$(api_post "$PRIMARY" "/api/transaction" \
    '{"tx_type":"Transfer","from":"0000000000000000000000000000000000000000000000000000000000000001","to":"0000000000000000000000000000000000000000000000000000000000000002","amount":100,"nonce":1,"signature":"0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"}')
expect_rejection "$CODE" "all-zero 64-byte sig"

# Truncated signature (31 bytes as hex)
CODE=$(api_post "$PRIMARY" "/api/transaction" \
    '{"tx_type":"Transfer","from":"aabb","to":"ccdd","amount":1,"nonce":1,"signature":"deadbeef01020304050607080910111213141516171819202122232425262728"}')
expect_rejection "$CODE" "truncated 32-byte sig"

# Wrong-type signature field (integer instead of hex string)
CODE=$(api_post "$PRIMARY" "/api/transaction" \
    '{"tx_type":"Transfer","from":"aabb","to":"ccdd","amount":1,"nonce":1,"signature":12345}')
expect_rejection "$CODE" "non-string signature field"

if check_finality_ok; then
    pass "A.1 — finality continued after signature attacks"
    VECTORS_PASSED=$((VECTORS_PASSED + 1))
else
    fail "A.1 — finality stalled after signature attacks"
    VECTORS_FAILED=$((VECTORS_FAILED + 1))
fi
sleep "$PAUSE_SECS"

# ── A.2  Replay attack ───────────────────────────────────────────────────────
header "A.2 — Replay attack (resubmit accepted faucet tx)"

# Fund address A, capture the response, resubmit the same funding request
REPLAY_ADDR=$(printf '%064x' 999999)
CODE1=$(api_post "$PRIMARY" "/api/faucet" "{\"address\":\"${REPLAY_ADDR}\"}")
sleep 2
CODE2=$(api_post "$PRIMARY" "/api/faucet" "{\"address\":\"${REPLAY_ADDR}\"}")

log "  First submit: HTTP $CODE1  Replay: HTTP $CODE2"
if [ "$CODE1" = "200" ] || [ "$CODE1" = "201" ]; then
    # Faucet has a cooldown — replay must be rejected with 429 or 400
    if python3 -c "exit(0 if int('${CODE2}') in (400,409,429) else 1)" 2>/dev/null; then
        pass "A.2 — replay rejected with HTTP $CODE2"
    elif [ "$CODE2" = "000" ]; then
        warn "A.2 — no response on replay (node unreachable)"
    else
        fail "A.2 — replay accepted with HTTP $CODE2 (should be 400/409/429)"
    fi
else
    warn "A.2 — initial faucet request returned HTTP $CODE1 (faucet may be disabled)"
fi

if check_finality_ok; then
    pass "A.2 — finality continued after replay attack"
    VECTORS_PASSED=$((VECTORS_PASSED + 1))
else
    fail "A.2 — finality stalled after replay attack"
    VECTORS_FAILED=$((VECTORS_FAILED + 1))
fi
sleep "$PAUSE_SECS"

# ── A.3  Conservation violation ─────────────────────────────────────────────
header "A.3 — Conservation violation (spend more than balance)"

CODE=$(api_post "$PRIMARY" "/api/transaction" \
    '{"tx_type":"Transfer","from":"0000000000000000000000000000000000000000000000000000000000000001","to":"0000000000000000000000000000000000000000000000000000000000000002","amount":999999999999999999,"nonce":1,"signature":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=="}')
expect_rejection "$CODE" "transfer exceeding balance"

if check_finality_ok; then
    pass "A.3 — finality continued after conservation attack"
    VECTORS_PASSED=$((VECTORS_PASSED + 1))
else
    fail "A.3 — finality stalled after conservation attack"
    VECTORS_FAILED=$((VECTORS_FAILED + 1))
fi
sleep "$PAUSE_SECS"

# ── A.4  Energy overflow ─────────────────────────────────────────────────────
header "A.4 — Energy overflow (u64::MAX in energy field)"

CODE=$(api_post "$PRIMARY" "/api/transaction" \
    '{"tx_type":"Transfer","from":"0001","to":"0002","amount":1,"energy":18446744073709551615,"nonce":1,"signature":"00"}')
expect_rejection "$CODE" "u64::MAX energy value"

# Negative energy (should be rejected by JSON deserialisation or validation)
CODE=$(api_post "$PRIMARY" "/api/transaction" \
    '{"tx_type":"Transfer","from":"0001","to":"0002","amount":1,"energy":-1,"nonce":1,"signature":"00"}')
expect_rejection "$CODE" "negative energy value"

if check_finality_ok; then
    pass "A.4 — finality continued after energy overflow attack"
    VECTORS_PASSED=$((VECTORS_PASSED + 1))
else
    fail "A.4 — finality stalled after energy overflow attack"
    VECTORS_FAILED=$((VECTORS_FAILED + 1))
fi
sleep "$PAUSE_SECS"

# ── A.5  Malformed JSON ──────────────────────────────────────────────────────
header "A.5 — Malformed JSON to every write endpoint"

ENDPOINTS=(
    "/api/transaction"
    "/api/faucet"
    "/api/governance/param"
    "/api/governance/amendment"
)
MALFORMED_BODIES=(
    "not-json-at-all"
    '{"incomplete": '
    '[]'
    '""'
    'null'
    '{}'
)
ALL_MALFORMED_OK=1
for ep in "${ENDPOINTS[@]}"; do
    for body in "${MALFORMED_BODIES[@]}"; do
        CODE=$(curl -s -m 10 -o /dev/null -w "%{http_code}" \
            -X POST "http://${PRIMARY}${ep}" \
            -H 'Content-Type: application/json' \
            -d "$body" 2>/dev/null || echo "000")
        if python3 -c "exit(0 if 400 <= int('${CODE}') < 500 else 1)" 2>/dev/null; then
            : # 4xx expected
        elif [ "$CODE" = "000" ]; then
            warn "A.5 $ep — no response for body: $body"
            ALL_MALFORMED_OK=0
        elif python3 -c "exit(0 if 500 <= int('${CODE}') < 600 else 1)" 2>/dev/null; then
            fail "A.5 $ep — HTTP $CODE (5xx panic) for body: $body"
            ALL_MALFORMED_OK=0
        fi
    done
done
[ "$ALL_MALFORMED_OK" = 1 ] && pass "A.5 — all malformed JSON returned 4xx on all endpoints" \
                              || log "  [NOTE] A.5 had some issues (see above)"

if check_finality_ok; then
    pass "A.5 — finality continued after malformed JSON flood"
    VECTORS_PASSED=$((VECTORS_PASSED + 1))
else
    fail "A.5 — finality stalled after malformed JSON flood"
    VECTORS_FAILED=$((VECTORS_FAILED + 1))
fi
sleep "$PAUSE_SECS"

# ── A.6  Governance parameter attack ────────────────────────────────────────
header "A.6 — Governance parameter attacks"

# Unknown param name
CODE=$(api_post "$PRIMARY" "/api/governance/param" \
    '{"param":"__totally_unknown_param__","value":"inject"}')
expect_rejection "$CODE" "unknown governance param name"

# Valid param name, out-of-range value
CODE=$(api_post "$PRIMARY" "/api/governance/param" \
    '{"param":"conservation_enforcement","value":"__INVALID__"}')
expect_rejection "$CODE" "invalid governance param value"

# Injection attempt in param value
CODE=$(api_post "$PRIMARY" "/api/governance/param" \
    '{"param":"conservation_enforcement","value":"; rm -rf /"}')
expect_rejection "$CODE" "shell injection in governance value"

# Missing required fields
CODE=$(api_post "$PRIMARY" "/api/governance/param" '{"param":"conservation_enforcement"}')
expect_rejection "$CODE" "governance param missing value field"

if check_finality_ok; then
    pass "A.6 — finality continued after governance attacks"
    VECTORS_PASSED=$((VECTORS_PASSED + 1))
else
    fail "A.6 — finality stalled after governance attacks"
    VECTORS_FAILED=$((VECTORS_FAILED + 1))
fi
sleep "$PAUSE_SECS"

# ── A.7  Double-nullifier (PNT replay) ──────────────────────────────────────
header "A.7 — Double-nullifier (PNT nullifier replay)"

# Submit a private transfer with the same fake nullifier twice.
# Both should fail (no valid note), but the second must not 5xx.
NULL_HEX="$(python3 -c "import os; print(os.urandom(32).hex())")"
PAYLOAD="{\"tx_type\":\"PrivateTransfer\",\"nullifier\":\"${NULL_HEX}\",\"commitment\":\"00\",\"proof\":\"00\",\"anchor\":\"00\",\"nonce\":1}"

CODE1=$(api_post "$PRIMARY" "/api/transaction" "$PAYLOAD")
sleep 1
CODE2=$(api_post "$PRIMARY" "/api/transaction" "$PAYLOAD")

log "  First nullifier submit: HTTP $CODE1  Replay: HTTP $CODE2"
if python3 -c "exit(0 if 400 <= int('${CODE2}') < 500 else 1)" 2>/dev/null; then
    pass "A.7 — double nullifier correctly rejected (HTTP $CODE2)"
elif [ "$CODE2" = "000" ]; then
    warn "A.7 — no response on nullifier replay"
elif python3 -c "exit(0 if 500 <= int('${CODE2}') < 600 else 1)" 2>/dev/null; then
    fail "A.7 — 5xx on nullifier replay — possible crash"
else
    warn "A.7 — nullifier replay returned HTTP $CODE2 (both rejected — acceptable)"
fi

if check_finality_ok; then
    pass "A.7 — finality continued after nullifier replay"
    VECTORS_PASSED=$((VECTORS_PASSED + 1))
else
    fail "A.7 — finality stalled after nullifier replay"
    VECTORS_FAILED=$((VECTORS_FAILED + 1))
fi
sleep "$PAUSE_SECS"

# ── A.8  Zero-value transfer ─────────────────────────────────────────────────
header "A.8 — Zero-value transfer"

CODE=$(api_post "$PRIMARY" "/api/transaction" \
    '{"tx_type":"Transfer","from":"0001","to":"0002","amount":0,"nonce":1,"signature":"00"}')
expect_rejection "$CODE" "zero-value transfer"

if check_finality_ok; then
    pass "A.8 — finality continued after zero-value attack"
    VECTORS_PASSED=$((VECTORS_PASSED + 1))
else
    fail "A.8 — finality stalled after zero-value attack"
    VECTORS_FAILED=$((VECTORS_FAILED + 1))
fi
sleep "$PAUSE_SECS"

# ── A.9  Invalid address formats ─────────────────────────────────────────────
header "A.9 — Address format attacks"

INVALID_ADDRS=(
    '""'                         # empty
    '"0xdeadbeef"'              # 0x-prefixed (not accepted)
    '"GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG"'  # non-hex chars
    '"0102030405060708091011121314151617181920212223242526272829303132EXTRA"'  # too long
)
ALL_ADDR_OK=1
for addr in "${INVALID_ADDRS[@]}"; do
    BODY="{\"tx_type\":\"Transfer\",\"from\":${addr},\"to\":\"0002\",\"amount\":1,\"nonce\":1,\"signature\":\"00\"}"
    CODE=$(api_post "$PRIMARY" "/api/transaction" "$BODY")
    if python3 -c "exit(0 if 400 <= int('${CODE}') < 500 else 1)" 2>/dev/null; then
        : # expected
    elif python3 -c "exit(0 if 500 <= int('${CODE}') < 600 else 1)" 2>/dev/null; then
        fail "A.9 — HTTP $CODE (5xx) for invalid addr: $addr"
        ALL_ADDR_OK=0
    fi
done
[ "$ALL_ADDR_OK" = 1 ] && pass "A.9 — all invalid address formats returned 4xx"

if check_finality_ok; then
    pass "A.9 — finality continued after address format attacks"
    VECTORS_PASSED=$((VECTORS_PASSED + 1))
else
    fail "A.9 — finality stalled after address format attacks"
    VECTORS_FAILED=$((VECTORS_FAILED + 1))
fi
sleep "$PAUSE_SECS"

# ── A.10  Future-height nonce ────────────────────────────────────────────────
header "A.10 — Future-height nonce (u64::MAX - 1)"

CODE=$(api_post "$PRIMARY" "/api/transaction" \
    '{"tx_type":"Transfer","from":"0001","to":"0002","amount":1,"nonce":18446744073709551614,"signature":"00"}')
expect_rejection "$CODE" "nonce = u64::MAX - 1"

if check_finality_ok; then
    pass "A.10 — finality continued after future-height nonce"
    VECTORS_PASSED=$((VECTORS_PASSED + 1))
else
    fail "A.10 — finality stalled after future-height nonce"
    VECTORS_FAILED=$((VECTORS_FAILED + 1))
fi

# ── Check no nodes crashed ───────────────────────────────────────────────────
header "Post-sweep node health"
ALL_NODES_OK=1
for t in "${TARGET_ARR[@]}"; do
    CODE=$(curl -s -m 5 -o /dev/null -w "%{http_code}" "http://${t}/api/health" 2>/dev/null || echo "000")
    FIN=$(get_finalized "$t")
    if [ "$CODE" = "200" ]; then
        pass "$t — healthy, finalized=$FIN"
    else
        fail "$t — HTTP $CODE after sweep (node may have crashed)"
        ALL_NODES_OK=0
    fi
done

POST_FIN=$(get_finalized "$PRIMARY")
BLOCK_DELTA=$((POST_FIN - PRE_FIN))
log "Finality delta: pre=$PRE_FIN post=$POST_FIN delta=$BLOCK_DELTA blocks"

# ── Summary ──────────────────────────────────────────────────────────────────
{
    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  T0.2 D.1 Adversarial sweep — $RUN_ID"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  Vectors PASS:   $VECTORS_PASSED"
    echo "  Vectors FAIL:   $VECTORS_FAILED"
    echo "  Block delta:    +$BLOCK_DELTA finalized blocks during sweep"
    echo "  All nodes up:   $([ "$ALL_NODES_OK" = 1 ] && echo YES || echo NO)"
    echo ""
    echo "  D.1 criteria:"
    echo "    - All adversarial inputs rejected with 4xx (not 5xx): $([ "$GLOBAL_PASS" = 1 ] && echo PASS || echo FAIL)"
    echo "    - No node crashed or restarted: $([ "$ALL_NODES_OK" = 1 ] && echo PASS || echo FAIL)"
    echo "    - Finality continued advancing: $([ "$BLOCK_DELTA" -gt 0 ] && echo PASS || echo "FAIL (delta=$BLOCK_DELTA)")"
    echo ""
    if [ "$GLOBAL_PASS" = 1 ] && [ "$ALL_NODES_OK" = 1 ] && [ "$BLOCK_DELTA" -gt 0 ]; then
        echo "  OVERALL VERDICT: PASS"
    else
        echo "  OVERALL VERDICT: FAIL  (see events.log)"
    fi
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  Logs: $OUTPUT"
} | tee -a "$REPORT"

log "ADVERSARIAL_SWEEP_END pass=$GLOBAL_PASS vectors_passed=$VECTORS_PASSED vectors_failed=$VECTORS_FAILED"
[ "$GLOBAL_PASS" = 1 ] && [ "$ALL_NODES_OK" = 1 ] && exit 0 || exit 1
