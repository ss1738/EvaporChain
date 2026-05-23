#!/usr/bin/env bash
#
# deploy-sbav-vm.sh — end-to-end doctrine proof for SinghBennettVM
# (contracts/evaporscript/sbav_vm.es).
#
# §A5.1 Reversible-VM Paradigm. "Laws of thermodynamics dictate gas cost."
#
# Landauer's principle: only irreversible ops export entropy; only ops
# that export entropy pay gas. Classical (reversible) ops cost 0.
# Decay is the unique irreversible primitive.
#
# Two modes:
#
#   --mode reversible (default):
#     deploy → op_add(reg=0, k=1000) → witness_vm(0) [snap1: reg0=1000, entropy=0]
#     → op_sub(reg=0, k=1000) [inverse of add — round-trip] →
#     witness_vm(0) [snap2: reg0=0, entropy=0] → require_zero_entropy PASSED.
#     Proves: reversible ops changed register state then recovered it; entropy
#     never moved — the thermodynamic arrow did not advance.
#
#   --mode decay:
#     deploy → op_swap(0,1) → op_add(reg=0, k=9999) →
#     op_decay(500) → witness_vm(0) [snap1: entropy=500] →
#     require_nonzero_entropy PASSED.
#     Proves: only Decay exports entropy; classical ops run first with zero cost.
#
# TX HASH DEDUP NOTE:
#   op_add / op_sub / op_swap / op_decay all take args → unique per run.
#   witness_vm(reg=0) is called twice with DIFFERENT CALLERS to avoid dedup.
#   require_zero_entropy / require_nonzero_entropy: caller = CALLER3_U8.
#   INITIAL_ENERGY randomised per run.
#
# Usage:
#   ./scripts/deploy-sbav-vm.sh --dry-run
#   ./scripts/deploy-sbav-vm.sh --node http://89.167.52.40:8099 --mode reversible
#   ./scripts/deploy-sbav-vm.sh --node http://89.167.52.40:8099 --mode decay
#
# Exit: 0 ok · 2 precondition · 3 deploy · 4 op/call · 5 gate · 6 verify
#
# Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CONTRACT_PATH="$ROOT_DIR/contracts/evaporscript/sbav_vm.es"

NODE_URL="${NODE_URL:-http://89.167.52.40:8099}"
TOKEN="${EVAPORCHAIN_TX_TOKEN:-}"
DEPLOYER_U8="${DEPLOYER_U8:-0}"
CALLER2_U8="${CALLER2_U8:-1}"    # second witness call (avoids dedup)
CALLER3_U8="${CALLER3_U8:-2}"    # proof gate caller
MODE="${MODE:-reversible}"       # reversible | decay

INITIAL_ENERGY="${INITIAL_ENERGY:-$(( 20000000 + RANDOM % 32768 ))}"
CONTRACT_HALF_LIFE="${CONTRACT_HALF_LIFE:-500000}"

# reversible-mode params
REG_ADD_K="${REG_ADD_K:-1000}"    # op_add(0, k) then op_sub(0, k) for round-trip

# decay-mode params
DECAY_AMOUNT="${DECAY_AMOUNT:-500}"

POLL_TIMEOUT_SEC="${POLL_TIMEOUT_SEC:-300}"
DRY_RUN=false
VERBOSE=false

usage() { cat <<'EOF'
deploy-sbav-vm.sh [options]
  --dry-run                print intended calls; no network
  --node URL               node base URL (default http://89.167.52.40:8099)
  --token TOKEN            auth bearer ($EVAPORCHAIN_TX_TOKEN)
  --deployer U8            owner index (default 0)
  --caller2 U8             second witness caller (default 1)
  --caller3 U8             proof gate caller (default 2)
  --mode reversible|decay  prove mode (default reversible)
  --add-k N                ADD/SUB operand for reversible mode (default 1000)
  --decay N                Decay amount for decay mode (default 500)
  --energy N               contract initial energy (~20M randomised)
  --hl N                   contract half-life (default 500000)
  --timeout SEC            poll timeout (default 300)
  --verbose
  -h|--help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)           DRY_RUN=true; shift ;;
    --node)              NODE_URL="$2"; shift 2 ;;
    --token)             TOKEN="$2"; shift 2 ;;
    --deployer)          DEPLOYER_U8="$2"; shift 2 ;;
    --caller2)           CALLER2_U8="$2"; shift 2 ;;
    --caller3)           CALLER3_U8="$2"; shift 2 ;;
    --mode)              MODE="$2"; shift 2 ;;
    --add-k)             REG_ADD_K="$2"; shift 2 ;;
    --decay)             DECAY_AMOUNT="$2"; shift 2 ;;
    --energy)            INITIAL_ENERGY="$2"; shift 2 ;;
    --hl)                CONTRACT_HALF_LIFE="$2"; shift 2 ;;
    --timeout)           POLL_TIMEOUT_SEC="$2"; shift 2 ;;
    --verbose)           VERBOSE=true; shift ;;
    -h|--help)           usage; exit 0 ;;
    *)                   echo "Unknown flag: $1" >&2; usage; exit 2 ;;
  esac
done

log()  { printf '\033[1;36m[sbav]\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m[sbav ERROR]\033[0m %s\n' "$*" >&2; exit "${2:-1}"; }
ok()   { printf '\033[1;32m[sbav OK]\033[0m %s\n' "$*"; }

curl_json() {
  local method="$1" path="$2" body="${3:-}"
  if $DRY_RUN; then echo "  [DRY-RUN] $method $NODE_URL$path ${body:+(body omitted)}" >&2; echo '{}'; return 0; fi
  local args=(-sS -m 30 -X "$method" -H 'Content-Type: application/json')
  [[ -n "$TOKEN" ]] && args+=(-H "Authorization: Bearer $TOKEN")
  [[ -n "$body" ]] && args+=(-d "$body")
  local resp; resp=$(curl "${args[@]}" "$NODE_URL$path") || die "curl $method $path failed" 2
  $VERBOSE && echo "  <- $resp" >&2
  printf '%s' "$resp"
}

submit_tx() {
  local resp; resp=$(curl_json POST "$1" "$2")
  $DRY_RUN && { echo "DRYHASH"; return 0; }
  local hash; hash=$(printf '%s' "$resp" | jq -r '.tx_hash // empty')
  [[ -n "$hash" ]] || die "$3 failed: $(printf '%s' "$resp" | jq -r '.message // .error // "(no msg)"')" "$4"
  printf '%s' "$hash"
}

poll_tx_state() {
  $DRY_RUN && { echo "finalised"; return 0; }
  local deadline=$(( $(date +%s) + POLL_TIMEOUT_SEC )) resp st
  while (( $(date +%s) < deadline )); do
    resp=$(curl_json GET "/api/tx/$1") || true
    st=$(printf '%s' "$resp" | jq -r '.state // "unknown"')
    case "$st" in
      included|finalised|rejected) printf '%s' "$st"; return 0 ;;
    esac
    sleep 2
  done
  printf 'timeout'
}

require_tx() {
  local h; h=$(submit_tx "$1" "$2" "$3" "$4")
  $DRY_RUN && return 0
  local s; s=$(poll_tx_state "$h")
  [[ "$s" == "finalised" || "$s" == "included" ]] || die "$3 tx not accepted (state=$s)" "$4"
  printf '%s' "$h"
}

get_epoch() { $DRY_RUN && { echo 0; return 0; }; curl_json GET "/api/status" | jq -r '.epoch // 0'; }
untag()     { jq -r ".state.$1 | if type==\"object\" then (.Bool // .U64 // .Str // .Address // .) else . end"; }

acquire_token() {
  $DRY_RUN && return 0
  [[ -n "$TOKEN" ]] && return 0
  local ts; ts=$(date +%s%N 2>/dev/null || date +%s)
  local email="deploy-sbav-${ts}@example.com"
  local pass="EvaporSBAV${ts}!"
  local reg_body; reg_body=$(jq -n --arg e "$email" --arg p "$pass" \
    '{email:$e, password:$p, display_name:"sbav-deploy"}')
  local reg_resp; reg_resp=$(curl -sS -m 15 -X POST \
    -H 'Content-Type: application/json' -d "$reg_body" \
    "$NODE_URL/api/auth/register") || die "auth register curl failed" 2
  local ok_r; ok_r=$(printf '%s' "$reg_resp" | jq -r '.success // false')
  [[ "$ok_r" == "true" ]] || die "auth register failed: $(printf '%s' "$reg_resp" | jq -r '.message')" 2
  local login_body; login_body=$(jq -n --arg e "$email" --arg p "$pass" '{email:$e, password:$p}')
  local login_resp; login_resp=$(curl -sS -m 15 -X POST \
    -H 'Content-Type: application/json' -d "$login_body" \
    "$NODE_URL/api/auth/login") || die "auth login curl failed" 2
  TOKEN=$(printf '%s' "$login_resp" | jq -r '.token // empty')
  [[ -n "$TOKEN" ]] || die "auth login returned no token: $(printf '%s' "$login_resp" | jq -r '.message')" 2
  log "auth: registered + logged in (email=$email)"
}

# ── preflight ──────────────────────────────────────────────────────────────
[[ -f "$CONTRACT_PATH" ]] || die "contract not found: $CONTRACT_PATH" 2
grep -q "^contract SinghBennettVM" "$CONTRACT_PATH" || die ".es missing SinghBennettVM header" 3
grep -q "fn op_add("              "$CONTRACT_PATH" || die ".es missing op_add" 3
grep -q "fn op_sub("              "$CONTRACT_PATH" || die ".es missing op_sub" 3
grep -q "fn op_swap("             "$CONTRACT_PATH" || die ".es missing op_swap" 3
grep -q "fn op_decay("            "$CONTRACT_PATH" || die ".es missing op_decay" 3
grep -q "fn witness_vm("          "$CONTRACT_PATH" || die ".es missing witness_vm" 3
grep -q "fn require_zero_entropy"    "$CONTRACT_PATH" || die ".es missing require_zero_entropy" 3
grep -q "fn require_nonzero_entropy" "$CONTRACT_PATH" || die ".es missing require_nonzero_entropy" 3
[[ "$MODE" == "reversible" || "$MODE" == "decay" ]] \
  || die "unknown --mode '$MODE' (reversible|decay)" 2

if ! $DRY_RUN; then
  command -v curl >/dev/null || die "curl required" 2
  command -v jq   >/dev/null || die "jq required" 2
  curl -sS -m 5 "$NODE_URL/api/status" >/dev/null 2>&1 || die "node $NODE_URL unreachable" 2
fi

acquire_token

if [[ "$MODE" == "reversible" ]]; then
cat <<EOF

+=====================================================================+
|  SinghBennettVM — §A5.1 SBAV doctrine proof (reversible mode)     |
+---------------------------------------------------------------------+
|  node: $NODE_URL
|  deployer: $DEPLOYER_U8  caller2: $CALLER2_U8  caller3: $CALLER3_U8
|  op_add(reg=0, k=$REG_ADD_K) → witness → op_sub(reg=0, k=$REG_ADD_K) → witness
|  expect: snap1_reg0=$REG_ADD_K snap1_entropy=0  snap2_reg0=0 snap2_entropy=0
+=====================================================================+
EOF
else
cat <<EOF

+=====================================================================+
|  SinghBennettVM — §A5.1 SBAV doctrine proof (decay mode)          |
+---------------------------------------------------------------------+
|  node: $NODE_URL
|  deployer: $DEPLOYER_U8  caller2: $CALLER2_U8  caller3: $CALLER3_U8
|  op_swap(0,1) → op_add(0,9999) → op_decay($DECAY_AMOUNT) → witness
|  expect: snap1_entropy=$DECAY_AMOUNT > 0
+=====================================================================+
EOF
fi

# ── Step 1: deploy ────────────────────────────────────────────────────────
log "Step 1 - deploy SinghBennettVM  energy=$INITIAL_ENERGY"
SRC=$(jq -Rs . < "$CONTRACT_PATH")
DBODY=$(jq -n \
  --argjson d "$DEPLOYER_U8" --argjson s "$SRC" \
  --argjson e "$INITIAL_ENERGY" --argjson hl "$CONTRACT_HALF_LIFE" \
  '{deployer:$d, source_code:$s, energy:$e, half_life:$hl}')
DH=$(submit_tx "/api/tx/deploy-script" "$DBODY" "deploy" 3)
if ! $DRY_RUN; then
  DS=$(poll_tx_state "$DH")
  [[ "$DS" == "finalised" || "$DS" == "included" ]] || die "deploy not accepted (state=$DS)" 3
  CID=$(curl_json GET "/api/tx/$DH" | jq -r '.contract_id // empty')
  [[ -n "$CID" ]] || die "no contract_id in deploy receipt" 3
  ok "deployed contract_id=$CID"
else
  CID=99
fi

# ── Mode-specific steps ───────────────────────────────────────────────────
if [[ "$MODE" == "reversible" ]]; then

  # Step 2: op_add(reg=0, k)
  EPOCH=$(get_epoch)
  log "Step 2 - op_add(reg=0, k=$REG_ADD_K)  epoch=$EPOCH"
  ADD_BODY=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson reg 0 --argjson k "$REG_ADD_K" \
    '{caller:$c, contract_id:$cid, method:"op_add",
      args:[{U64:$reg},{U64:$k}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$ADD_BODY" "op_add" 4

  # Step 3: witness_vm(reg=0) → snapshot1
  EPOCH=$(get_epoch)
  log "Step 3 - witness_vm(reg=0, caller=$DEPLOYER_U8) → snapshot1  epoch=$EPOCH"
  W1_BODY=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson reg 0 \
    '{caller:$c, contract_id:$cid, method:"witness_vm",
      args:[{U64:$reg}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$W1_BODY" "witness1" 4

  # Step 4: op_sub(reg=0, k) — inverse of add
  EPOCH=$(get_epoch)
  log "Step 4 - op_sub(reg=0, k=$REG_ADD_K) [inverse of add]  epoch=$EPOCH"
  SUB_BODY=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson reg 0 --argjson k "$REG_ADD_K" \
    '{caller:$c, contract_id:$cid, method:"op_sub",
      args:[{U64:$reg},{U64:$k}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$SUB_BODY" "op_sub" 4

  # Step 5: witness_vm(reg=0) → snapshot2 (different caller to avoid dedup)
  EPOCH=$(get_epoch)
  log "Step 5 - witness_vm(reg=0, caller=$CALLER2_U8) → snapshot2  epoch=$EPOCH"
  W2_BODY=$(jq -n \
    --argjson c "$CALLER2_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson reg 0 \
    '{caller:$c, contract_id:$cid, method:"witness_vm",
      args:[{U64:$reg}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$W2_BODY" "witness2" 4

  # Read and verify state
  if ! $DRY_RUN; then
    STATE=$(curl_json GET "/api/script/$CID")
    S1V=$(printf '%s' "$STATE" | untag snapshot1_reg_val)
    S1E=$(printf '%s' "$STATE" | untag snapshot1_entropy)
    S2V=$(printf '%s' "$STATE" | untag snapshot2_reg_val)
    S2E=$(printf '%s' "$STATE" | untag snapshot2_entropy)
    OC=$(printf '%s'  "$STATE" | untag op_count)
    WC=$(printf '%s'  "$STATE" | untag witness_count)
    ok "op_count=$OC  witness_count=$WC"
    ok "snapshot1: reg0=$S1V  entropy=$S1E"
    ok "snapshot2: reg0=$S2V  entropy=$S2E"
    [[ "$S1V" -eq "$REG_ADD_K" ]] || die "expected snap1_reg0=$REG_ADD_K (after add), got $S1V" 6
    [[ "$S1E" -eq 0 ]]            || die "expected snap1_entropy=0 (no Decay), got $S1E" 6
    [[ "$S2V" -eq 0 ]]            || die "expected snap2_reg0=0 (after sub inverse), got $S2V" 6
    [[ "$S2E" -eq 0 ]]            || die "expected snap2_entropy=0 (still no Decay), got $S2E" 6
    ok "round-trip: reg0=$REG_ADD_K → 0 (add then sub); entropy stayed 0 throughout ✓"
  fi

  # Step 6: require_zero_entropy gate
  EPOCH=$(get_epoch)
  log "Step 6 - require_zero_entropy (caller=$CALLER3_U8)"
  GZ_BODY=$(jq -n \
    --argjson c "$CALLER3_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"require_zero_entropy", args:[], epoch:$ep}')
  require_tx "/api/tx/call-script" "$GZ_BODY" "require_zero_entropy" 5
  ok "require_zero_entropy PASSED — purely reversible program ✓"

else  # decay mode

  # Step 2: op_swap(0, 1) — reversible, zero gas
  EPOCH=$(get_epoch)
  log "Step 2 - op_swap(a=0, b=1)  epoch=$EPOCH"
  SW_BODY=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"op_swap",
      args:[{U64:0},{U64:1}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$SW_BODY" "op_swap" 4

  # Step 3: op_add(0, 9999) — reversible, zero gas
  EPOCH=$(get_epoch)
  log "Step 3 - op_add(reg=0, k=9999)  epoch=$EPOCH"
  ADD2_BODY=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"op_add",
      args:[{U64:0},{U64:9999}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$ADD2_BODY" "op_add2" 4

  # Step 4: op_decay(amount) — irreversible; entropy_exported += amount
  EPOCH=$(get_epoch)
  log "Step 4 - op_decay(amount=$DECAY_AMOUNT) — IRREVERSIBLE  epoch=$EPOCH"
  DEC_BODY=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson amt "$DECAY_AMOUNT" \
    '{caller:$c, contract_id:$cid, method:"op_decay",
      args:[{U64:$amt}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$DEC_BODY" "op_decay" 4

  # Step 5: witness_vm(reg=0) → snapshot1
  EPOCH=$(get_epoch)
  log "Step 5 - witness_vm(reg=0, caller=$DEPLOYER_U8) → snapshot1  epoch=$EPOCH"
  WD_BODY=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"witness_vm",
      args:[{U64:0}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$WD_BODY" "witness_decay" 4

  # Read and verify state
  if ! $DRY_RUN; then
    STATE=$(curl_json GET "/api/script/$CID")
    S1E=$(printf '%s' "$STATE" | untag snapshot1_entropy)
    S1V=$(printf '%s' "$STATE" | untag snapshot1_reg_val)
    OC=$(printf '%s'  "$STATE" | untag op_count)
    ENT=$(printf '%s' "$STATE" | untag entropy_exported)
    ok "op_count=$OC  entropy_exported=$ENT"
    ok "snapshot1: reg0=$S1V  entropy=$S1E"
    [[ "$S1E" -eq "$DECAY_AMOUNT" ]] || die "expected snap1_entropy=$DECAY_AMOUNT, got $S1E" 6
    [[ "$S1E" -gt 0 ]]               || die "entropy should be > 0 after Decay, got $S1E" 6
    ok "entropy_exported=$S1E > 0 ✓ (Decay is the sole irreversible op)"
  fi

  # Step 6: require_nonzero_entropy gate
  EPOCH=$(get_epoch)
  log "Step 6 - require_nonzero_entropy (caller=$CALLER3_U8)"
  GNZ_BODY=$(jq -n \
    --argjson c "$CALLER3_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"require_nonzero_entropy", args:[], epoch:$ep}')
  require_tx "/api/tx/call-script" "$GNZ_BODY" "require_nonzero_entropy" 5
  ok "require_nonzero_entropy PASSED — Decay applied, thermodynamic arrow advanced ✓"

fi

# ── Final summary ──────────────────────────────────────────────────────────
if [[ "$MODE" == "reversible" ]]; then
  cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — SinghBennettVM (reversible mode)       |
+---------------------------------------------------------------------+
|  contract_id: $CID
|  PROVEN (reversible ops cost zero entropy):
|   - op_add(reg=0, k=$REG_ADD_K): reg0 0 → $REG_ADD_K ✓
|   - witness snap1: reg0=$REG_ADD_K  entropy=0 ✓
|   - op_sub(reg=0, k=$REG_ADD_K) [inverse]: reg0 $REG_ADD_K → 0 ✓
|   - witness snap2: reg0=0  entropy=0 ✓ (arrow didn't move)
|   - require_zero_entropy PASSED ✓
|   - "Laws of thermodynamics dictate gas cost." — classical = free ✓
+=====================================================================+
EOF
else
  cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — SinghBennettVM (decay mode)            |
+---------------------------------------------------------------------+
|  contract_id: $CID
|  PROVEN (only Decay exports entropy):
|   - op_swap + op_add: classical, zero entropy cost ✓
|   - op_decay($DECAY_AMOUNT): entropy_exported=$DECAY_AMOUNT > 0 ✓
|   - require_nonzero_entropy PASSED ✓
|   - "The unique irreversible op." — Landauer-literal ✓
+=====================================================================+
EOF
fi
