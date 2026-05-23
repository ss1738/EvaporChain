#!/usr/bin/env bash
#
# deploy-singh-heartbeat.sh — end-to-end doctrine proof for SinghHeartbeat
# (contracts/evaporscript/singh_heartbeat.es).
#
# §A5.4 EvaporWallet-Pulse. "Your crypto wallet has a pulse."
#
# Vitals model: aggregate_health_bp = sum(cur_e) * 100 / sum(anchor_e).
# bpm = 60 + 60 * (100 − health_bp) / 100. Color: Green/Amber/Red.
# Arrhythmia = health_bp − worst_item_hp (gap between aggregate and worst).
#
# Two modes:
#
#   --mode healthy (default):
#     deploy → register two large healthy items (huge energy, huge hl) →
#     pulse (caller 0) → verify: color=0 (Green), bpm ≈ 60,
#     arrhythmia=0, health_bp ≈ 100 → require_healthy PASSED.
#     Proves: healthy wallet → Green pulse, slow BPM.
#
#   --mode arrhythmia:
#     deploy → register one large healthy item + one tiny dying item →
#     pulse (caller 0) → verify: color=0 (Green, giant dominates),
#     arrhythmia > 0 (dying item creates irregular rhythm) →
#     require_arrhythmic PASSED.
#     Proves: "what makes the wallet feel wrong before the user sees
#     the inbox" — aggregate is fine but rhythm is irregular.
#
# TX HASH DEDUP NOTE:
#   pulse(), require_healthy(), require_arrhythmic() take no args.
#   pulse = caller[0]; require_healthy/require_arrhythmic = caller[1].
#   INITIAL_ENERGY randomised per run to produce a unique deploy hash.
#
# Usage:
#   ./scripts/deploy-singh-heartbeat.sh --dry-run
#   ./scripts/deploy-singh-heartbeat.sh --node http://89.167.52.40:8099 --mode healthy
#   ./scripts/deploy-singh-heartbeat.sh --node http://89.167.52.40:8099 --mode arrhythmia
#
# Exit: 0 ok · 2 precondition · 3 deploy · 4 register/call · 5 gate · 6 verify
#
# Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CONTRACT_PATH="$ROOT_DIR/contracts/evaporscript/singh_heartbeat.es"

NODE_URL="${NODE_URL:-http://89.167.52.40:8099}"
TOKEN="${EVAPORCHAIN_TX_TOKEN:-}"
DEPLOYER_U8="${DEPLOYER_U8:-0}"       # owner — must be funded
CALLER2_U8="${CALLER2_U8:-1}"         # second caller (avoids dedup on no-arg calls)
MODE="${MODE:-healthy}"               # healthy | arrhythmia

# Contract energy — randomised per run to avoid deploy-tx dedup
INITIAL_ENERGY="${INITIAL_ENERGY:-$(( 20000000 + RANDOM % 32768 ))}"
CONTRACT_HALF_LIFE="${CONTRACT_HALF_LIFE:-500000}"

# healthy-mode item params (large energy, large hl → stays near 100% health)
HEALTHY_ENERGY="${HEALTHY_ENERGY:-1000000}"
HEALTHY_HL="${HEALTHY_HL:-1000000}"

# arrhythmia-mode items:
#   item0 = large healthy giant (dominates aggregate)
#   item1 = tiny dying item (energy=4, hl=1; dies in a few epochs)
GIANT_ENERGY="${GIANT_ENERGY:-1000000}"
GIANT_HL="${GIANT_HL:-1000000}"
DYING_ENERGY="${DYING_ENERGY:-4}"
DYING_HL="${DYING_HL:-1}"

POLL_TIMEOUT_SEC="${POLL_TIMEOUT_SEC:-300}"
DRY_RUN=false
VERBOSE=false

usage() { cat <<'EOF'
deploy-singh-heartbeat.sh [options]
  --dry-run                validate + print intended calls; no network
  --node URL               node base URL (default http://89.167.52.40:8099)
  --token TOKEN            auth bearer ($EVAPORCHAIN_TX_TOKEN)
  --deployer U8            owner index (default 0)
  --caller2 U8             second caller index (default 1)
  --mode healthy|arrhythmia   prove mode (default healthy)
  --energy N               contract initial energy (default randomised ~20M)
  --hl N                   contract half-life (default 500000)
  --timeout SEC            poll timeout (default 300)
  --verbose                echo node responses
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
    --mode)              MODE="$2"; shift 2 ;;
    --energy)            INITIAL_ENERGY="$2"; shift 2 ;;
    --hl)                CONTRACT_HALF_LIFE="$2"; shift 2 ;;
    --timeout)           POLL_TIMEOUT_SEC="$2"; shift 2 ;;
    --verbose)           VERBOSE=true; shift ;;
    -h|--help)           usage; exit 0 ;;
    *)                   echo "Unknown flag: $1" >&2; usage; exit 2 ;;
  esac
done

log()  { printf '\033[1;36m[heartbeat]\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m[heartbeat ERROR]\033[0m %s\n' "$*" >&2; exit "${2:-1}"; }
ok()   { printf '\033[1;32m[heartbeat OK]\033[0m %s\n' "$*"; }

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
  local email="deploy-heartbeat-${ts}@example.com"
  local pass="EvaporHB${ts}!"
  local reg_body; reg_body=$(jq -n --arg e "$email" --arg p "$pass" \
    '{email:$e, password:$p, display_name:"heartbeat-deploy"}')
  local reg_resp; reg_resp=$(curl -sS -m 15 -X POST \
    -H 'Content-Type: application/json' -d "$reg_body" \
    "$NODE_URL/api/auth/register") || die "auth register curl failed" 2
  local ok_r; ok_r=$(printf '%s' "$reg_resp" | jq -r '.success // false')
  [[ "$ok_r" == "true" ]] || die "auth register failed: $(printf '%s' "$reg_resp" | jq -r '.message')" 2
  local login_body; login_body=$(jq -n --arg e "$email" --arg p "$pass" \
    '{email:$e, password:$p}')
  local login_resp; login_resp=$(curl -sS -m 15 -X POST \
    -H 'Content-Type: application/json' -d "$login_body" \
    "$NODE_URL/api/auth/login") || die "auth login curl failed" 2
  TOKEN=$(printf '%s' "$login_resp" | jq -r '.token // empty')
  [[ -n "$TOKEN" ]] || die "auth login returned no token: $(printf '%s' "$login_resp" | jq -r '.message')" 2
  log "auth: registered + logged in (email=$email)"
}

# ── preflight ──────────────────────────────────────────────────────────────
[[ -f "$CONTRACT_PATH" ]] || die "contract not found: $CONTRACT_PATH" 2
grep -q "^contract SinghHeartbeat"  "$CONTRACT_PATH" || die ".es missing SinghHeartbeat header" 3
grep -q "fn register_item("         "$CONTRACT_PATH" || die ".es missing register_item" 3
grep -q "fn pulse("                 "$CONTRACT_PATH" || die ".es missing pulse" 3
grep -q "fn require_healthy("       "$CONTRACT_PATH" || die ".es missing require_healthy" 3
grep -q "fn require_arrhythmic("    "$CONTRACT_PATH" || die ".es missing require_arrhythmic" 3
[[ "$MODE" == "healthy" || "$MODE" == "arrhythmia" ]] \
  || die "unknown --mode '$MODE' (healthy|arrhythmia)" 2

if ! $DRY_RUN; then
  command -v curl >/dev/null || die "curl required" 2
  command -v jq   >/dev/null || die "jq required" 2
  curl -sS -m 5 "$NODE_URL/api/status" >/dev/null 2>&1 || die "node $NODE_URL unreachable" 2
fi

acquire_token

# ── Step 1: deploy ────────────────────────────────────────────────────────
log "Step 1 - deploy-script (singh_heartbeat.es)  energy=$INITIAL_ENERGY"
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

# ── Item registration ─────────────────────────────────────────────────────
if [[ "$MODE" == "healthy" ]]; then

  cat <<EOF

+=====================================================================+
|  SinghHeartbeat — §A5.4 wallet pulse doctrine proof (healthy mode)  |
+---------------------------------------------------------------------+
|  node: $NODE_URL  mode: $($DRY_RUN && echo DRY-RUN || echo LIVE)
|  deployer(u8): $DEPLOYER_U8  caller2(u8): $CALLER2_U8
|  item0: energy=$HEALTHY_ENERGY  hl=$HEALTHY_HL  (large healthy)
|  item1: energy=$HEALTHY_ENERGY  hl=$HEALTHY_HL  (large healthy)
+=====================================================================+
EOF

  EPOCH=$(get_epoch)
  log "Step 2 - register_item 0 (energy=$HEALTHY_ENERGY, hl=$HEALTHY_HL)"
  R0=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson e "$HEALTHY_ENERGY" --argjson hl "$HEALTHY_HL" \
    '{caller:$c, contract_id:$cid, method:"register_item",
      args:[{U64:$e},{U64:$hl}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$R0" "register_item0" 4

  EPOCH=$(get_epoch)
  log "Step 3 - register_item 1 (energy=$HEALTHY_ENERGY, hl=$HEALTHY_HL)"
  R1=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson e "$HEALTHY_ENERGY" --argjson hl "$HEALTHY_HL" \
    '{caller:$c, contract_id:$cid, method:"register_item",
      args:[{U64:$e},{U64:$hl}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$R1" "register_item1" 4
  ok "2 healthy items registered ✓"

else

  cat <<EOF

+=====================================================================+
|  SinghHeartbeat — §A5.4 wallet pulse doctrine proof (arrhythmia)   |
+---------------------------------------------------------------------+
|  node: $NODE_URL  mode: $($DRY_RUN && echo DRY-RUN || echo LIVE)
|  deployer(u8): $DEPLOYER_U8  caller2(u8): $CALLER2_U8
|  item0: energy=$GIANT_ENERGY  hl=$GIANT_HL  (healthy giant)
|  item1: energy=$DYING_ENERGY  hl=$DYING_HL  (tiny dying item)
+=====================================================================+
EOF

  EPOCH=$(get_epoch)
  log "Step 2 - register_item 0 (giant: energy=$GIANT_ENERGY, hl=$GIANT_HL)"
  RG=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson e "$GIANT_ENERGY" --argjson hl "$GIANT_HL" \
    '{caller:$c, contract_id:$cid, method:"register_item",
      args:[{U64:$e},{U64:$hl}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$RG" "register_item_giant" 4

  EPOCH=$(get_epoch)
  log "Step 3 - register_item 1 (dying: energy=$DYING_ENERGY, hl=$DYING_HL)"
  RD=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson e "$DYING_ENERGY" --argjson hl "$DYING_HL" \
    '{caller:$c, contract_id:$cid, method:"register_item",
      args:[{U64:$e},{U64:$hl}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$RD" "register_item_dying" 4
  ok "1 giant + 1 dying item registered ✓"

fi

# ── Step 4: pulse ─────────────────────────────────────────────────────────
EPOCH=$(get_epoch)
log "Step 4 - pulse (caller=account[$DEPLOYER_U8]) at epoch=$EPOCH"
P_BODY=$(jq -n \
  --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  '{caller:$c, contract_id:$cid, method:"pulse", args:[], epoch:$ep}')
require_tx "/api/tx/call-script" "$P_BODY" "pulse" 4

if ! $DRY_RUN; then
  STATE=$(curl_json GET "/api/script/$CID")
  BPM=$(printf '%s'  "$STATE" | untag last_bpm)
  COL=$(printf '%s'  "$STATE" | untag last_color)
  ARR=$(printf '%s'  "$STATE" | untag last_arrhythmia)
  HBP=$(printf '%s'  "$STATE" | untag last_health_bp)
  WHP=$(printf '%s'  "$STATE" | untag last_worst_hp)
  PC=$(printf '%s'   "$STATE" | untag pulse_count)
  ok "pulse_count=$PC  bpm=$BPM  color=$COL  arrhythmia=$ARR  health_bp=$HBP  worst_hp=$WHP"

  if [[ "$MODE" == "healthy" ]]; then
    [[ "$COL" == "0" ]] || die "expected color=0 (Green), got $COL" 6
    [[ "$BPM" -le 65 ]] || die "expected bpm ≤ 65 (near resting), got $BPM" 6
    [[ "$ARR" == "0" ]] || die "expected arrhythmia=0 (regular), got $ARR" 6
    [[ "$HBP" -ge 90 ]] || die "expected health_bp ≥ 90 (healthy), got $HBP" 6
    ok "color=Green ✓  bpm=$BPM (≤65) ✓  arrhythmia=0 ✓  health_bp=$HBP (≥90) ✓"
  else
    [[ "$COL" == "0" ]] || die "expected color=0 (Green, giant dominates), got $COL" 6
    [[ "$ARR" -gt 0 ]] || die "expected arrhythmia > 0 (dying item), got $ARR" 6
    ok "color=Green ✓ (giant dominates aggregate)  arrhythmia=$ARR > 0 ✓ (dying item signals)"
  fi
fi

# ── Step 5: proof gate ───────────────────────────────────────────────────
EPOCH=$(get_epoch)
if [[ "$MODE" == "healthy" ]]; then
  log "Step 5 - require_healthy: color=0 (Green) must PASS"
  G_BODY=$(jq -n \
    --argjson c "$CALLER2_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"require_healthy", args:[], epoch:$ep}')
  require_tx "/api/tx/call-script" "$G_BODY" "require_healthy" 5
  ok "require_healthy PASSED — wallet pulse is Green ✓"
else
  log "Step 5 - require_arrhythmic: arrhythmia > 0 must PASS"
  G_BODY=$(jq -n \
    --argjson c "$CALLER2_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"require_arrhythmic", args:[], epoch:$ep}')
  require_tx "/api/tx/call-script" "$G_BODY" "require_arrhythmic" 5
  ok "require_arrhythmic PASSED — irregular rhythm detected ✓"
fi

if [[ "$MODE" == "healthy" ]]; then
  cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — SinghHeartbeat (healthy mode)           |
+---------------------------------------------------------------------+
|  contract_id: $CID
|  PROVEN (healthy wallet → Green pulse):
|   - 2 large healthy items registered ✓
|   - pulse: color=Green  bpm≤65 (resting)  arrhythmia=0 ✓
|   - require_healthy PASSED ✓
|   - "Your crypto wallet has a pulse." (slow 60bpm green) ✓
+=====================================================================+
EOF
else
  cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — SinghHeartbeat (arrhythmia mode)        |
+---------------------------------------------------------------------+
|  contract_id: $CID
|  PROVEN (arrhythmic wallet):
|   - 1 healthy giant + 1 tiny dying item registered ✓
|   - pulse: color=Green (aggregate) + arrhythmia > 0 (dying signal) ✓
|   - require_arrhythmic PASSED ✓
|   - "What makes the wallet feel wrong before the user sees the inbox." ✓
+=====================================================================+
EOF
fi
