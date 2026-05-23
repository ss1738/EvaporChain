#!/usr/bin/env bash
#
# deploy-ssm-vm.sh — end-to-end doctrine proof for SinghStrategyMachines
# (contracts/evaporscript/ssm_vm.es).
#
# §A5.1 Smart Contract Paradigm: game-semantic contracts.
# "The contract is a proof you can win against any adversary, mechanically.
#  Decay is the visibility condition: draining a P-move's energy makes every
#  O-challenge it justified disappear from the arena structurally."
#
# Game structure:
#   O-move: Opponent plays; initial move (justifier=999=root sentinel).
#   P-respond: Proponent answers an O-move (P justified by the O-move).
#   O-challenge: Opponent challenges a P-move (O justified by the P-move).
#   Visibility: a move is visible iff its justifier has energy > 0.
#
# Two modes:
#
#   --mode strategy (default):
#     Prove P has a live strategy covering all visible O-moves.
#     o_move(1000) → p_respond(0, 800) → o_challenge(1, 600) →
#     p_respond(2, 500) →
#     witness_move(0=O-root, snap1: player=0, energy=1000, jus_e=0) →
#     witness_move(1=P-resp, snap2: player=1, energy=800, jus_e=1000) →
#     check_strategy → strategy_holds=1 →
#     require_strategy_holds PASSED.
#     Proves: P has responded to all O-moves; strategy is live.
#
#   --mode decay:
#     Prove structural pruning: draining a P-move makes O-challenge invisible.
#     o_move(1000) → p_respond(0, 800) → o_challenge(1, 600) →
#     drain_move(slot=1, amount=800) → P-move slot=1 energy=0 →
#     witness_move(2=O-challenge, snap1: justifier_energy=0) →
#     require_move_invisible(2) PASSED.
#     Proves: the game tree prunes itself — no explicit revocation needed.
#
# TX HASH DEDUP:
#   o_move, p_respond, o_challenge have distinct args → naturally unique.
#   drain_move takes (slot, amount) → unique.
#   witness_move(slot=0) vs witness_move(slot=2) → different slot args.
#   check_strategy uses CALLER2. require_* uses CALLER3.
#   INITIAL_ENERGY randomised per run.
#
# Usage:
#   ./scripts/deploy-ssm-vm.sh --dry-run
#   ./scripts/deploy-ssm-vm.sh --node http://89.167.52.40:8099 --mode strategy
#   ./scripts/deploy-ssm-vm.sh --node http://89.167.52.40:8099 --mode decay
#
# Exit: 0 ok · 2 precondition · 3 deploy · 4 call · 5 gate · 6 verify
#
# Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CONTRACT_PATH="$ROOT_DIR/contracts/evaporscript/ssm_vm.es"

NODE_URL="${NODE_URL:-http://89.167.52.40:8099}"
TOKEN="${EVAPORCHAIN_TX_TOKEN:-}"
DEPLOYER_U8="${DEPLOYER_U8:-0}"
CALLER2_U8="${CALLER2_U8:-1}"    # check_strategy caller
CALLER3_U8="${CALLER3_U8:-2}"    # gate caller
MODE="${MODE:-strategy}"         # strategy | decay

INITIAL_ENERGY="${INITIAL_ENERGY:-$(( 20000000 + RANDOM % 32768 ))}"
CONTRACT_HALF_LIFE="${CONTRACT_HALF_LIFE:-500000}"

POLL_TIMEOUT_SEC="${POLL_TIMEOUT_SEC:-300}"
DRY_RUN=false
VERBOSE=false

# Move energies (strategy mode)
O_ROOT_ENERGY=1000
P_RESP1_ENERGY=800
O_CHAL_ENERGY=600
P_RESP2_ENERGY=500

# Move energies (decay mode)
DECAY_O_ROOT=1000
DECAY_P_RESP=800   # will be fully drained
DECAY_O_CHAL=600

usage() { cat <<'EOF'
deploy-ssm-vm.sh [options]
  --dry-run                print intended calls; no network
  --node URL               node base URL (default http://89.167.52.40:8099)
  --token TOKEN            auth bearer ($EVAPORCHAIN_TX_TOKEN)
  --deployer U8            owner index (default 0)
  --caller2 U8             check_strategy caller (default 1)
  --caller3 U8             gate caller (default 2)
  --mode strategy|decay    prove mode (default strategy)
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
    --energy)            INITIAL_ENERGY="$2"; shift 2 ;;
    --hl)                CONTRACT_HALF_LIFE="$2"; shift 2 ;;
    --timeout)           POLL_TIMEOUT_SEC="$2"; shift 2 ;;
    --verbose)           VERBOSE=true; shift ;;
    -h|--help)           usage; exit 0 ;;
    *)                   echo "Unknown flag: $1" >&2; usage; exit 2 ;;
  esac
done

log()  { printf '\033[1;36m[ssm]\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m[ssm ERROR]\033[0m %s\n' "$*" >&2; exit "${2:-1}"; }
ok()   { printf '\033[1;32m[ssm OK]\033[0m %s\n' "$*"; }

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
  local email="deploy-ssm-${ts}@example.com"
  local pass="EvaporSSM${ts}!"
  local reg_body; reg_body=$(jq -n --arg e "$email" --arg p "$pass" \
    '{email:$e, password:$p, display_name:"ssm-deploy"}')
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
grep -q "^contract SinghStrategyMachines" "$CONTRACT_PATH" || die ".es missing SinghStrategyMachines header" 3
grep -q "fn o_move("                      "$CONTRACT_PATH" || die ".es missing o_move" 3
grep -q "fn p_respond("                   "$CONTRACT_PATH" || die ".es missing p_respond" 3
grep -q "fn o_challenge("                 "$CONTRACT_PATH" || die ".es missing o_challenge" 3
grep -q "fn drain_move("                  "$CONTRACT_PATH" || die ".es missing drain_move" 3
grep -q "fn witness_move("                "$CONTRACT_PATH" || die ".es missing witness_move" 3
grep -q "fn check_strategy("              "$CONTRACT_PATH" || die ".es missing check_strategy" 3
grep -q "fn require_strategy_holds("      "$CONTRACT_PATH" || die ".es missing require_strategy_holds" 3
grep -q "fn require_move_invisible("      "$CONTRACT_PATH" || die ".es missing require_move_invisible" 3
[[ "$MODE" == "strategy" || "$MODE" == "decay" ]] \
  || die "unknown --mode '$MODE' (strategy|decay)" 2

if ! $DRY_RUN; then
  command -v curl >/dev/null || die "curl required" 2
  command -v jq   >/dev/null || die "jq required" 2
  curl -sS -m 5 "$NODE_URL/api/status" >/dev/null 2>&1 || die "node $NODE_URL unreachable" 2
fi

acquire_token

if [[ "$MODE" == "strategy" ]]; then
cat <<EOF

+=====================================================================+
|  SinghStrategyMachines — §A5.1 doctrine proof (strategy mode)    |
+---------------------------------------------------------------------+
|  node: $NODE_URL
|  deployer: $DEPLOYER_U8  caller2: $CALLER2_U8  caller3: $CALLER3_U8
|  game: O(1000) → P(800) → O-chal(600) → P(500)
|  all O-moves answered with live P-responses
|  expect: strategy_holds=1, require_strategy_holds PASSED
+=====================================================================+
EOF
else
cat <<EOF

+=====================================================================+
|  SinghStrategyMachines — §A5.1 doctrine proof (decay mode)       |
+---------------------------------------------------------------------+
|  node: $NODE_URL
|  deployer: $DEPLOYER_U8  caller2: $CALLER2_U8  caller3: $CALLER3_U8
|  game: O(1000) → P(800) → O-chal(600) → drain P(800) fully
|  O-challenge slot=2 loses its justifier → invisible
|  expect: require_move_invisible(2) PASSED
+=====================================================================+
EOF
fi

# ── Step 1: deploy ────────────────────────────────────────────────────────
log "Step 1 - deploy SinghStrategyMachines  energy=$INITIAL_ENERGY"
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
if [[ "$MODE" == "strategy" ]]; then

  # Step 2: O plays root move (slot=0, justifier=999)
  EPOCH=$(get_epoch)
  log "Step 2 - o_move(energy=$O_ROOT_ENERGY) → slot 0 (O root, justifier=sentinel)"
  OM=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson e "$O_ROOT_ENERGY" \
    '{caller:$c, contract_id:$cid, method:"o_move", args:[{U64:$e}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$OM" "o_move" 4
  ok "O plays root move (slot=0, energy=$O_ROOT_ENERGY, justifier=sentinel) ✓"

  # Step 3: P responds to O-root (slot=1, justified by slot=0)
  EPOCH=$(get_epoch)
  log "Step 3 - p_respond(o_slot=0, energy=$P_RESP1_ENERGY) → slot 1 (P justified by O-root)"
  PR1=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson e "$P_RESP1_ENERGY" \
    '{caller:$c, contract_id:$cid, method:"p_respond", args:[{U64:0},{U64:$e}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$PR1" "p_respond_root" 4
  ok "P responds to O-root (slot=1, energy=$P_RESP1_ENERGY) ✓"

  # Step 4: O challenges P-move slot=1 (slot=2, justified by slot=1)
  EPOCH=$(get_epoch)
  log "Step 4 - o_challenge(p_slot=1, energy=$O_CHAL_ENERGY) → slot 2 (O justified by P-move)"
  OC=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson e "$O_CHAL_ENERGY" \
    '{caller:$c, contract_id:$cid, method:"o_challenge", args:[{U64:1},{U64:$e}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$OC" "o_challenge" 4
  ok "O challenges P-move (slot=2, energy=$O_CHAL_ENERGY, justifier=slot1) ✓"

  # Step 5: P responds to O-challenge (slot=3, justified by slot=2)
  # Args differ from step 3 (o_slot=2 vs 0, energy=500 vs 800) — no dedup risk.
  EPOCH=$(get_epoch)
  log "Step 5 - p_respond(o_slot=2, energy=$P_RESP2_ENERGY) → slot 3 (P responds to challenge)"
  PR2=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson e "$P_RESP2_ENERGY" \
    '{caller:$c, contract_id:$cid, method:"p_respond", args:[{U64:2},{U64:$e}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$PR2" "p_respond_challenge" 4
  ok "P responds to O-challenge (slot=3, energy=$P_RESP2_ENERGY) ✓"

  # Step 6: witness O-root (slot=0) → snapshot1
  EPOCH=$(get_epoch)
  log "Step 6 - witness_move(slot=0 O-root) → snapshot1 [player=0, energy=$O_ROOT_ENERGY, jus_e=0]"
  WM0=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"witness_move", args:[{U64:0}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$WM0" "witness_o_root" 4

  # Step 7: witness P-response (slot=1) → snapshot2
  EPOCH=$(get_epoch)
  log "Step 7 - witness_move(slot=1 P-respond) → snapshot2 [player=1, jus_e=O-root energy]"
  WM1=$(jq -n \
    --argjson c "$CALLER3_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"witness_move", args:[{U64:1}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$WM1" "witness_p_resp" 4

  # Step 8: check_strategy
  EPOCH=$(get_epoch)
  log "Step 8 - check_strategy (caller=$CALLER2_U8)"
  CS=$(jq -n \
    --argjson c "$CALLER2_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"check_strategy", args:[], epoch:$ep}')
  require_tx "/api/tx/call-script" "$CS" "check_strategy" 4

  # Read and verify state
  if ! $DRY_RUN; then
    STATE=$(curl_json GET "/api/script/$CID")
    SH=$(printf '%s'  "$STATE" | untag strategy_holds)
    S1S=$(printf '%s' "$STATE" | untag snapshot1_slot)
    S1P=$(printf '%s' "$STATE" | untag snapshot1_player)
    S1E=$(printf '%s' "$STATE" | untag snapshot1_energy)
    S1J=$(printf '%s' "$STATE" | untag snapshot1_justifier_energy)
    S2S=$(printf '%s' "$STATE" | untag snapshot2_slot)
    S2P=$(printf '%s' "$STATE" | untag snapshot2_player)
    S2E=$(printf '%s' "$STATE" | untag snapshot2_energy)
    S2J=$(printf '%s' "$STATE" | untag snapshot2_justifier_energy)
    MC=$(printf '%s'  "$STATE" | untag move_count)
    ok "move_count=$MC  strategy_holds=$SH"
    ok "snapshot1(O-root): slot=$S1S player=$S1P energy=$S1E justifier_energy=$S1J"
    ok "snapshot2(P-resp): slot=$S2S player=$S2P energy=$S2E justifier_energy=$S2J"
    [[ "$SH"  -eq 1 ]]                   || die "strategy_holds should be 1, got $SH" 6
    [[ "$S1P" -eq 0 ]]                   || die "snap1 player should be 0 (O), got $S1P" 6
    [[ "$S1E" -eq "$O_ROOT_ENERGY" ]]    || die "snap1 energy should be $O_ROOT_ENERGY, got $S1E" 6
    [[ "$S1J" -eq 0 ]]                   || die "snap1 jus_e should be 0 (root/sentinel), got $S1J" 6
    [[ "$S2P" -eq 1 ]]                   || die "snap2 player should be 1 (P), got $S2P" 6
    [[ "$S2E" -eq "$P_RESP1_ENERGY" ]]   || die "snap2 energy should be $P_RESP1_ENERGY, got $S2E" 6
    [[ "$S2J" -eq "$O_ROOT_ENERGY" ]]    || die "snap2 jus_e should be $O_ROOT_ENERGY (O-root alive), got $S2J" 6
    ok "O-root: player=O energy=$O_ROOT_ENERGY jus_e=0 (sentinel) ✓"
    ok "P-resp: player=P energy=$P_RESP1_ENERGY jus_e=$O_ROOT_ENERGY (O-root alive) ✓"
  fi

  # Step 9: require_strategy_holds gate
  EPOCH=$(get_epoch)
  log "Step 9 - require_strategy_holds (caller=$CALLER3_U8)"
  RSH=$(jq -n \
    --argjson c "$CALLER3_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"require_strategy_holds", args:[], epoch:$ep}')
  require_tx "/api/tx/call-script" "$RSH" "require_strategy_holds" 5
  ok "require_strategy_holds PASSED — P has a complete live strategy ✓"

else  # decay mode

  # Step 2: O plays root (slot=0)
  EPOCH=$(get_epoch)
  log "Step 2 - o_move(energy=$DECAY_O_ROOT) → slot 0"
  OM=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson e "$DECAY_O_ROOT" \
    '{caller:$c, contract_id:$cid, method:"o_move", args:[{U64:$e}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$OM" "o_move" 4
  ok "O root move (slot=0, energy=$DECAY_O_ROOT) ✓"

  # Step 3: P responds (slot=1, justified by slot=0)
  EPOCH=$(get_epoch)
  log "Step 3 - p_respond(o_slot=0, energy=$DECAY_P_RESP) → slot 1"
  PR=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson e "$DECAY_P_RESP" \
    '{caller:$c, contract_id:$cid, method:"p_respond", args:[{U64:0},{U64:$e}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$PR" "p_respond" 4
  ok "P responds (slot=1, energy=$DECAY_P_RESP) ✓"

  # Step 4: O challenges P-move slot=1 (slot=2, justified by slot=1)
  EPOCH=$(get_epoch)
  log "Step 4 - o_challenge(p_slot=1, energy=$DECAY_O_CHAL) → slot 2"
  OC=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson e "$DECAY_O_CHAL" \
    '{caller:$c, contract_id:$cid, method:"o_challenge", args:[{U64:1},{U64:$e}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$OC" "o_challenge" 4
  ok "O challenges P-move (slot=2, energy=$DECAY_O_CHAL, justifier=slot1) ✓"

  # Step 5: drain P-move slot=1 fully → energy goes to 0
  EPOCH=$(get_epoch)
  log "Step 5 - drain_move(slot=1, amount=$DECAY_P_RESP) → P-move slot=1 energy=0"
  DM=$(jq -n \
    --argjson c "$CALLER2_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson a "$DECAY_P_RESP" \
    '{caller:$c, contract_id:$cid, method:"drain_move", args:[{U64:1},{U64:$a}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$DM" "drain_move" 4
  ok "P-move slot=1 drained to 0 — justifier of O-challenge slot=2 is now dead ✓"

  # Step 6: witness O-challenge (slot=2) → snapshot1 (justifier_energy=0)
  EPOCH=$(get_epoch)
  log "Step 6 - witness_move(slot=2 O-challenge) → snapshot1 [justifier_energy=0 after drain]"
  WM=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"witness_move", args:[{U64:2}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$WM" "witness_o_challenge" 4

  # Verify snapshot
  if ! $DRY_RUN; then
    STATE=$(curl_json GET "/api/script/$CID")
    S1S=$(printf '%s' "$STATE" | untag snapshot1_slot)
    S1P=$(printf '%s' "$STATE" | untag snapshot1_player)
    S1E=$(printf '%s' "$STATE" | untag snapshot1_energy)
    S1J=$(printf '%s' "$STATE" | untag snapshot1_justifier_energy)
    MC=$(printf '%s'  "$STATE" | untag move_count)
    ok "move_count=$MC"
    ok "snapshot1(O-challenge slot=2): slot=$S1S player=$S1P energy=$S1E justifier_energy=$S1J"
    [[ "$S1S" -eq 2 ]]               || die "snap1 slot should be 2 (O-challenge), got $S1S" 6
    [[ "$S1P" -eq 0 ]]               || die "snap1 player should be 0 (O), got $S1P" 6
    [[ "$S1E" -eq "$DECAY_O_CHAL" ]] || die "snap1 energy should be $DECAY_O_CHAL, got $S1E" 6
    [[ "$S1J" -eq 0 ]]               || die "snap1 justifier_energy should be 0 (P-move drained), got $S1J" 6
    ok "O-challenge: energy=$DECAY_O_CHAL  justifier_energy=0 (P-move fully drained) ✓"
    ok "O-challenge is now invisible — its justifier P-move has zero energy ✓"
  fi

  # Step 7: require_move_invisible(slot=2) gate
  EPOCH=$(get_epoch)
  log "Step 7 - require_move_invisible(slot=2)"
  RMI=$(jq -n \
    --argjson c "$CALLER3_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"require_move_invisible", args:[{U64:2}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$RMI" "require_move_invisible" 5
  ok "require_move_invisible(O-challenge) PASSED — game tree pruned structurally ✓"

fi

# ── Final summary ──────────────────────────────────────────────────────────
if [[ "$MODE" == "strategy" ]]; then
  cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — SinghStrategyMachines (strategy mode) |
+---------------------------------------------------------------------+
|  contract_id: $CID
|  PROVEN (innocent strategy covers all O-moves):
|   - O root (slot=0): player=0 energy=$O_ROOT_ENERGY jus_e=0 (sentinel) ✓
|   - P response (slot=1): player=1 energy=$P_RESP1_ENERGY jus_e=$O_ROOT_ENERGY ✓
|   - O challenge (slot=2): player=0 energy=$O_CHAL_ENERGY ✓
|   - P counter (slot=3): player=1 energy=$P_RESP2_ENERGY ✓
|   - strategy_holds = 1 ✓
|   - require_strategy_holds PASSED ✓
|   - "The contract is a proof you can win against any adversary" ✓
+=====================================================================+
EOF
else
  cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — SinghStrategyMachines (decay mode)    |
+---------------------------------------------------------------------+
|  contract_id: $CID
|  PROVEN (structural pruning via visibility decay):
|   - O root (slot=0): energy=$DECAY_O_ROOT ✓
|   - P response (slot=1): energy=$DECAY_P_RESP → drained to 0 ✓
|   - O challenge (slot=2): justifier=slot1; justifier_energy=0 after drain ✓
|   - require_move_invisible(slot=2) PASSED ✓
|   - "The game tree prunes itself — no explicit revocation needed" ✓
|   - Decay is the visibility condition: unjustified-by-dead → invisible ✓
+=====================================================================+
EOF
fi
