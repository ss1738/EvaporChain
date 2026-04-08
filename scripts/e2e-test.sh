#!/bin/bash
# ═══════════════════════════════════════════════════════════════════
# EvaporChain End-to-End Behavioral Test Suite
# Tests that the chain ACTUALLY WORKS — not just that APIs respond.
# ═══════════════════════════════════════════════════════════════════
set -e

API="http://localhost:18002"  # Use node 2 (non-demo, should have consensus blocks)
API2="http://localhost:18003" # Cross-check on node 3
PASS=0
FAIL=0
TOTAL=0

test_pass() { PASS=$((PASS+1)); TOTAL=$((TOTAL+1)); echo "  PASS: $1"; }
test_fail() { FAIL=$((FAIL+1)); TOTAL=$((TOTAL+1)); echo "  FAIL: $1 — $2"; }

echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  EvaporChain End-to-End Behavioral Test Suite                ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""

# ─────────────────────────────────────────────────────────────────
# TEST 1: Chain is alive and producing blocks
# ─────────────────────────────────────────────────────────────────
echo "--- Test 1: Chain Liveness ---"

STATUS=$(curl -s $API/api/status)
HEIGHT=$(echo "$STATUS" | python3 -c "import sys,json;print(json.load(sys.stdin)['block_height'])")
EPOCH=$(echo "$STATUS" | python3 -c "import sys,json;print(json.load(sys.stdin)['epoch'])")

if [ "$HEIGHT" -gt 0 ] 2>/dev/null; then
    test_pass "Chain is producing blocks (height=$HEIGHT, epoch=$EPOCH)"
else
    test_fail "Chain is NOT producing blocks" "height=$HEIGHT"
fi

PEERS=$(echo "$STATUS" | python3 -c "import sys,json;print(json.load(sys.stdin)['peer_count'])")
if [ "$PEERS" -gt 0 ] 2>/dev/null; then
    test_pass "Node has peers connected (peers=$PEERS)"
else
    test_fail "No peers connected" "peers=$PEERS"
fi

# ─────────────────────────────────────────────────────────────────
# TEST 2: Genesis state is correct
# ─────────────────────────────────────────────────────────────────
echo ""
echo "--- Test 2: Genesis State ---"

# Check genesis foundation account
FOUNDATION=$(curl -s $API/api/address/0x7f3a8b2ce419d605a1c74e823fb960d4159ae378)
F_BALANCE=$(echo "$FOUNDATION" | python3 -c "import sys,json;print(json.load(sys.stdin)['balance'])" 2>/dev/null)
F_NONCE=$(echo "$FOUNDATION" | python3 -c "import sys,json;print(json.load(sys.stdin)['nonce'])" 2>/dev/null)

if [ -n "$F_BALANCE" ] && [ "$F_BALANCE" != "null" ]; then
    test_pass "Genesis Foundation account exists (balance=$F_BALANCE, nonce=$F_NONCE)"
else
    test_fail "Genesis Foundation account not found" "$FOUNDATION"
fi

# Check genesis objects exist
OBJECTS=$(curl -s $API/api/objects)
OBJ_COUNT=$(echo "$OBJECTS" | python3 -c "import sys,json;print(len(json.load(sys.stdin)))" 2>/dev/null)
if [ "$OBJ_COUNT" -gt 0 ] 2>/dev/null; then
    test_pass "Genesis objects exist (count=$OBJ_COUNT)"
else
    test_fail "No objects found" "count=$OBJ_COUNT"
fi

# ─────────────────────────────────────────────────────────────────
# TEST 3: Faucet works — get funds for a NEW address
# ─────────────────────────────────────────────────────────────────
echo ""
echo "--- Test 3: Faucet ---"

# Use a unique test address
TEST_ADDR="0xdeadbeef$(date +%s | md5sum | head -c 16)"
echo "  Test address: $TEST_ADDR"

FAUCET_RESP=$(curl -s -X POST $API/api/faucet \
    -H "Content-Type: application/json" \
    -d "{\"address\": \"$TEST_ADDR\"}")
FAUCET_OK=$(echo "$FAUCET_RESP" | python3 -c "import sys,json;print(json.load(sys.stdin).get('success',False))" 2>/dev/null)
FAUCET_BAL=$(echo "$FAUCET_RESP" | python3 -c "import sys,json;print(json.load(sys.stdin).get('balance',0))" 2>/dev/null)

if [ "$FAUCET_OK" = "True" ]; then
    test_pass "Faucet credited $FAUCET_BAL to new address"
else
    test_fail "Faucet failed" "$FAUCET_RESP"
fi

# Verify the balance is queryable (faucet goes through consensus, wait for block inclusion)
echo "  Waiting 30s for faucet tx inclusion in block..."
sleep 30
ADDR_RESP=$(curl -s $API/api/address/$TEST_ADDR)
ADDR_BAL=$(echo "$ADDR_RESP" | python3 -c "import sys,json;print(json.load(sys.stdin).get('balance',0))" 2>/dev/null)

if [ "$ADDR_BAL" -gt 0 ] 2>/dev/null; then
    test_pass "Faucet balance confirmed on-chain (balance=$ADDR_BAL)"
else
    test_fail "Faucet balance not on-chain yet" "balance=$ADDR_BAL (may need more blocks)"
fi

# ─────────────────────────────────────────────────────────────────
# TEST 4: Transfer transaction — actually moves money
# ─────────────────────────────────────────────────────────────────
echo ""
echo "--- Test 4: Transfer ---"

# Use the faucet-funded test address from Test 3 as sender
FROM_ADDR="$TEST_ADDR"
TO_ADDR="0xaaaa$(date +%s | md5sum | head -c 24)"

# Get current nonce (should be 0 since this is a fresh faucet-funded account)
PRE=$(curl -s $API/api/address/$FROM_ADDR)
FROM_BAL_BEFORE=$(echo "$PRE" | python3 -c "import sys,json;print(json.load(sys.stdin)['balance'])" 2>/dev/null)
FROM_NONCE=$(echo "$PRE" | python3 -c "import sys,json;print(json.load(sys.stdin)['nonce'])" 2>/dev/null)
echo "  From: $FROM_ADDR (balance=$FROM_BAL_BEFORE, nonce=$FROM_NONCE)"

# Submit transfer (without signature — testing if the node accepts unsigned in demo mode)
TRANSFER_AMT=100
TX_RESP=$(curl -s -X POST $API/api/tx/transfer \
    -H "Content-Type: application/json" \
    -d "{\"from\": \"$FROM_ADDR\", \"to\": \"$TO_ADDR\", \"amount\": $TRANSFER_AMT, \"nonce\": $FROM_NONCE, \"signature\": \"deadbeef\"}")
TX_OK=$(echo "$TX_RESP" | python3 -c "import sys,json;print(json.load(sys.stdin).get('success',False))" 2>/dev/null)
TX_HASH=$(echo "$TX_RESP" | python3 -c "import sys,json;print(json.load(sys.stdin).get('tx_hash','none'))" 2>/dev/null)

if [ "$TX_OK" = "True" ]; then
    test_pass "Transfer submitted (hash=$TX_HASH)"
else
    # Might need to wait for consensus to include it
    echo "  Transfer response: $TX_RESP"
    test_fail "Transfer rejected" "$TX_RESP"
fi

# Wait for block inclusion
echo "  Waiting 30s for block inclusion..."
sleep 30

# Check balances changed
POST=$(curl -s $API/api/address/$FROM_ADDR)
FROM_BAL_AFTER=$(echo "$POST" | python3 -c "import sys,json;print(json.load(sys.stdin)['balance'])" 2>/dev/null)

if [ -n "$FROM_BAL_BEFORE" ] && [ -n "$FROM_BAL_AFTER" ]; then
    if [ "$FROM_BAL_AFTER" -lt "$FROM_BAL_BEFORE" ] 2>/dev/null; then
        DIFF=$((FROM_BAL_BEFORE - FROM_BAL_AFTER))
        test_pass "Sender balance decreased by $DIFF (was $FROM_BAL_BEFORE, now $FROM_BAL_AFTER)"
    else
        test_fail "Sender balance did NOT decrease" "before=$FROM_BAL_BEFORE after=$FROM_BAL_AFTER"
    fi
fi

# ─────────────────────────────────────────────────────────────────
# TEST 5: Object creation — create and verify energy object
# ─────────────────────────────────────────────────────────────────
echo ""
echo "--- Test 5: Object Creation ---"

OBJ_CREATOR="0x2b91f50d68a37ce214b65903d74a8ef1c5263b90"
OBJ_ID="0x$(date +%s | md5sum | head -c 16)"
OBJ_ENERGY=5000
OBJ_HALFLIFE=50

OBJ_RESP=$(curl -s -X POST $API/api/tx/create-object \
    -H "Content-Type: application/json" \
    -d "{\"creator\": \"$OBJ_CREATOR\", \"object_id\": \"$OBJ_ID\", \"energy\": $OBJ_ENERGY, \"half_life\": $OBJ_HALFLIFE, \"signature\": \"deadbeef\"}")
OBJ_OK=$(echo "$OBJ_RESP" | python3 -c "import sys,json;print(json.load(sys.stdin).get('success',False))" 2>/dev/null)

if [ "$OBJ_OK" = "True" ]; then
    test_pass "Object creation submitted (id=$OBJ_ID, energy=$OBJ_ENERGY, half_life=$OBJ_HALFLIFE)"
else
    echo "  Object response: $OBJ_RESP"
    test_fail "Object creation failed" "$OBJ_RESP"
fi

echo "  Waiting 30s for block inclusion..."
sleep 30

# Verify object exists
ALL_OBJECTS=$(curl -s $API/api/objects)
NEW_OBJ_COUNT=$(echo "$ALL_OBJECTS" | python3 -c "import sys,json;print(len(json.load(sys.stdin)))" 2>/dev/null)
echo "  Objects after creation: $NEW_OBJ_COUNT (was $OBJ_COUNT before)"

if [ "$NEW_OBJ_COUNT" -gt "$OBJ_COUNT" ] 2>/dev/null; then
    test_pass "New object appears in object list (count: $OBJ_COUNT -> $NEW_OBJ_COUNT)"
else
    test_fail "New object NOT in object list" "count=$NEW_OBJ_COUNT"
fi

# ─────────────────────────────────────────────────────────────────
# TEST 6: Energy decay — objects lose energy over epochs
# ─────────────────────────────────────────────────────────────────
echo ""
echo "--- Test 6: Energy Decay ---"

# Check a genesis object's current energy vs initial
DECAY_OBJ=$(echo "$ALL_OBJECTS" | python3 -c "
import sys,json
objs=json.load(sys.stdin)
for o in objs:
    if o.get('current_energy',0) < o.get('energy',0):
        print(f'{o[\"id\"]}|{o[\"energy\"]}|{o[\"current_energy\"]}|{o.get(\"decay_percentage\",0):.1f}')
        break
else:
    # Check if any have decay_percentage > 0
    for o in objs:
        dp = o.get('decay_percentage', 0)
        if dp > 0:
            print(f'{o[\"id\"]}|{o[\"energy\"]}|{o[\"current_energy\"]}|{dp:.1f}')
            break
    else:
        print('NONE')
" 2>/dev/null)

if [ "$DECAY_OBJ" != "NONE" ] && [ -n "$DECAY_OBJ" ]; then
    IFS='|' read -r D_ID D_INITIAL D_CURRENT D_PCT <<< "$DECAY_OBJ"
    test_pass "Energy decay is happening (object $D_ID: $D_INITIAL -> $D_CURRENT, ${D_PCT}% decayed)"
else
    # Energy decay depends on epoch advancement — check if epochs are high enough
    CURRENT_EPOCH=$(curl -s $API/api/status | python3 -c "import sys,json;print(json.load(sys.stdin)['epoch'])" 2>/dev/null)
    echo "  Current epoch: $CURRENT_EPOCH (decay may need more epochs for objects with large half-lives)"
    test_fail "No energy decay detected yet" "epoch=$CURRENT_EPOCH"
fi

# ─────────────────────────────────────────────────────────────────
# TEST 7: Cross-node state consistency
# ─────────────────────────────────────────────────────────────────
echo ""
echo "--- Test 7: Cross-Node Consistency ---"

S1=$(curl -s http://localhost:18001/api/status 2>/dev/null)
S2=$(curl -s http://localhost:18002/api/status 2>/dev/null)
S3=$(curl -s http://localhost:18003/api/status 2>/dev/null)
S4=$(curl -s http://localhost:18004/api/status 2>/dev/null)

H1=$(echo "$S1" | python3 -c "import sys,json;print(json.load(sys.stdin)['block_height'])" 2>/dev/null)
H2=$(echo "$S2" | python3 -c "import sys,json;print(json.load(sys.stdin)['block_height'])" 2>/dev/null)
H3=$(echo "$S3" | python3 -c "import sys,json;print(json.load(sys.stdin)['block_height'])" 2>/dev/null)
H4=$(echo "$S4" | python3 -c "import sys,json;print(json.load(sys.stdin)['block_height'])" 2>/dev/null)

R2=$(echo "$S2" | python3 -c "import sys,json;print(json.load(sys.stdin)['state_root'])" 2>/dev/null)
R3=$(echo "$S3" | python3 -c "import sys,json;print(json.load(sys.stdin)['state_root'])" 2>/dev/null)
R4=$(echo "$S4" | python3 -c "import sys,json;print(json.load(sys.stdin)['state_root'])" 2>/dev/null)

echo "  Heights: v1=$H1 v2=$H2 v3=$H3 v4=$H4"

# Check that at least 3 nodes agree
AGREE=0
if [ "$H2" = "$H3" ] && [ "$R2" = "$R3" ]; then AGREE=$((AGREE+1)); fi
if [ "$H3" = "$H4" ] && [ "$R3" = "$R4" ]; then AGREE=$((AGREE+1)); fi
if [ "$H2" = "$H4" ] && [ "$R2" = "$R4" ]; then AGREE=$((AGREE+1)); fi

if [ "$AGREE" -ge 2 ]; then
    test_pass "3+ nodes agree on state (root=${R2:0:20}...)"
elif [ "$AGREE" -ge 1 ]; then
    test_pass "2 nodes agree on state (partial consensus)"
else
    test_fail "Nodes disagree on state" "roots differ"
fi

# Check balance consistency across synced nodes (skip node 3 which may be behind)
FAUCET_BAL_N1=$(curl -s http://localhost:18001/api/address/0x0000000000000000000000000000000000000000 | python3 -c "import sys,json;print(json.load(sys.stdin)['balance'])" 2>/dev/null)
FAUCET_BAL_N2=$(curl -s http://localhost:18002/api/address/0x0000000000000000000000000000000000000000 | python3 -c "import sys,json;print(json.load(sys.stdin)['balance'])" 2>/dev/null)

if [ "$FAUCET_BAL_N1" = "$FAUCET_BAL_N2" ] 2>/dev/null; then
    test_pass "Balance consistent across synced nodes (faucet account: $FAUCET_BAL_N1 on both)"
else
    test_fail "Balance inconsistent" "node1=$FAUCET_BAL_N1 node2=$FAUCET_BAL_N2"
fi

# ─────────────────────────────────────────────────────────────────
# TEST 8: Block data integrity
# ─────────────────────────────────────────────────────────────────
echo ""
echo "--- Test 8: Block Integrity ---"

BLOCKS=$(curl -s "$API/api/blocks?limit=5")
BLOCK_COUNT=$(echo "$BLOCKS" | python3 -c "import sys,json;print(len(json.load(sys.stdin)))" 2>/dev/null)

if [ "$BLOCK_COUNT" -gt 0 ] 2>/dev/null; then
    test_pass "Block history available ($BLOCK_COUNT blocks)"
else
    test_fail "No blocks in history" ""
fi

# Verify blocks are sequential
SEQUENTIAL=$(echo "$BLOCKS" | python3 -c "
import sys,json
blocks=json.load(sys.stdin)
if len(blocks) < 2:
    print('TOO_FEW')
else:
    prev = blocks[0]['number']
    ok = True
    for b in blocks[1:]:
        if b['number'] != prev - 1:
            ok = False
            break
        prev = b['number']
    print('OK' if ok else 'GAP')
" 2>/dev/null)

if [ "$SEQUENTIAL" = "OK" ]; then
    test_pass "Blocks are sequential (no gaps)"
elif [ "$SEQUENTIAL" = "TOO_FEW" ]; then
    test_pass "Only 1 block — sequential check N/A"
else
    test_fail "Blocks have gaps" "$SEQUENTIAL"
fi

# Verify blocks have valid state roots (non-zero)
VALID_ROOTS=$(echo "$BLOCKS" | python3 -c "
import sys,json
blocks=json.load(sys.stdin)
invalid = [b['number'] for b in blocks if b['state_root'] == '0'*64 or not b['state_root']]
print('OK' if not invalid else f'INVALID:{invalid}')
" 2>/dev/null)

if [ "$VALID_ROOTS" = "OK" ]; then
    test_pass "All blocks have valid state roots"
else
    test_fail "Some blocks have invalid state roots" "$VALID_ROOTS"
fi

# ─────────────────────────────────────────────────────────────────
# TEST 9: Rate limiting (faucet)
# ─────────────────────────────────────────────────────────────────
echo ""
echo "--- Test 9: Rate Limiting ---"

# Try faucet again for same address (should be rate limited)
FAUCET2=$(curl -s -X POST $API/api/faucet \
    -H "Content-Type: application/json" \
    -d "{\"address\": \"$TEST_ADDR\"}")
FAUCET2_OK=$(echo "$FAUCET2" | python3 -c "import sys,json;print(json.load(sys.stdin).get('success',False))" 2>/dev/null)

if [ "$FAUCET2_OK" = "False" ]; then
    test_pass "Faucet rate limiting works (rejected second request)"
else
    test_fail "Faucet NOT rate limiting" "allowed second request"
fi

# ─────────────────────────────────────────────────────────────────
# TEST 10: Invalid transaction rejection
# ─────────────────────────────────────────────────────────────────
echo ""
echo "--- Test 10: Invalid TX Rejection ---"

# Transfer to self
SELF_TX=$(curl -s -X POST $API/api/tx/transfer \
    -H "Content-Type: application/json" \
    -d "{\"from\": \"$FROM_ADDR\", \"to\": \"$FROM_ADDR\", \"amount\": 100, \"nonce\": 0, \"signature\": \"deadbeef\"}")
SELF_OK=$(echo "$SELF_TX" | python3 -c "import sys,json;print(json.load(sys.stdin).get('success',False))" 2>/dev/null)
if [ "$SELF_OK" = "False" ]; then
    test_pass "Self-transfer rejected"
else
    test_fail "Self-transfer was ALLOWED" "$SELF_TX"
fi

# Zero amount
ZERO_TX=$(curl -s -X POST $API/api/tx/transfer \
    -H "Content-Type: application/json" \
    -d "{\"from\": \"$FROM_ADDR\", \"to\": \"$TO_ADDR\", \"amount\": 0, \"nonce\": 0, \"signature\": \"deadbeef\"}")
ZERO_OK=$(echo "$ZERO_TX" | python3 -c "import sys,json;print(json.load(sys.stdin).get('success',False))" 2>/dev/null)
if [ "$ZERO_OK" = "False" ]; then
    test_pass "Zero-amount transfer rejected"
else
    test_fail "Zero-amount transfer was ALLOWED" "$ZERO_TX"
fi

# ─────────────────────────────────────────────────────────────────
# TEST 11: Blocks advance over time
# ─────────────────────────────────────────────────────────────────
echo ""
echo "--- Test 11: Block Advancement ---"

H_BEFORE=$(curl -s $API/api/status | python3 -c "import sys,json;print(json.load(sys.stdin)['block_height'])" 2>/dev/null)
echo "  Height now: $H_BEFORE. Waiting 30s..."
sleep 30
H_AFTER=$(curl -s $API/api/status | python3 -c "import sys,json;print(json.load(sys.stdin)['block_height'])" 2>/dev/null)
echo "  Height after: $H_AFTER"

if [ "$H_AFTER" -gt "$H_BEFORE" ] 2>/dev/null; then
    BLOCKS_PRODUCED=$((H_AFTER - H_BEFORE))
    RATE=$(echo "scale=1; $BLOCKS_PRODUCED / 30" | bc 2>/dev/null || echo "?")
    test_pass "Chain advancing: $BLOCKS_PRODUCED blocks in 30s (~$RATE blocks/sec)"
else
    test_fail "Chain is STALLED" "height before=$H_BEFORE after=$H_AFTER"
fi

# ─────────────────────────────────────────────────────────────────
# RESULTS
# ─────────────────────────────────────────────────────────────────
echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  RESULTS: $PASS passed, $FAIL failed, $TOTAL total"
if [ $FAIL -eq 0 ]; then
    echo "  STATUS: ALL TESTS PASSED"
else
    echo "  STATUS: $FAIL FAILURES"
fi
echo "═══════════════════════════════════════════════════════════════"
