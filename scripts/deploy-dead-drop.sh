#!/usr/bin/env bash
#
# deploy-dead-drop.sh — live-e2e runbook for the Dead Drop flagship demo.
#
# Dead Drop IS the mortal-messages dApp (contracts/evaporscript/
# mortal_message.es) positioned as the "prove the chain" forgetting
# demo — see research/DEAD_DROP_ARCHITECTURE.md. A message contract is
# deployed with a SMALL energy budget + half-life (the TTL); the
# chain's evaporation engine drives it to a terminal evaporated
# forgetting end-to-end on a live node and is the inverse of
# deploy-sfsv.sh: there, a 404 after payout was a BUG (wrong endpoint);
# here, disappearance from /api/script/:id IS the success — but ONLY
# when preceded by a confirmed successful read (non-vacuity guard), so
# a green run cannot be a vacuous "deploy failed / never sealed".
#
# Node contract is the SOURCE-VERIFIED one (VERIFICATION_2026_05_16.md /
# the SFSV live-e2e PASS), corrected endpoint included:
#   - deployer/caller are u8 indices; addr_from_byte(0) = genesis faucet
#   - call-script args = externally-tagged Vec<Value>:
#       string  -> {"Str": "..."}    address -> {"Address":[b0..b31]}
#   - epoch is REQUIRED on call-script
#   - script state is at GET /api/script/:id  (NOT /api/contract/:id —
#     the unrelated template store; this is the b76df4a2 correction)
#
# Flow:
#   1. POST /api/tx/deploy-script  mortal_message.es (small energy/hl)
#   2. poll GET /api/tx/<hash>     -> .contract_id
#   3. call-script set_payload(body, recipient)        (seal once)
#   4. call-script read() -> finalised  AND GET /api/script/:id shows
#      the body  => CONFIRMED READABLE (sets saw_readable; non-vacuity)
#   5. poll GET /api/script/:id until evaporated==true (terminal,
#      energy-exhausted, unrefreshable). Success iff saw_readable==1
#      AND it reached terminal evaporation. NOTE (verified 2026-05-17):
#      get_script keeps returning .state.body post-evaporation — this
#      proves liveness-death, NOT byte-erasure. Not overclaimed.
#
# Exit: 0 terminal-evap-after-readable · 2 precondition · 3 deploy ·
#       4 set_payload · 5 read/non-vacuity · 6 never-forgot
#
# Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CONTRACT_PATH="$ROOT_DIR/contracts/evaporscript/mortal_message.es"

NODE_URL="${NODE_URL:-http://89.167.52.40:8099}"
TOKEN="${EVAPORCHAIN_TX_TOKEN:-}"
DEPLOYER_U8="${DEPLOYER_U8:-0}"          # 0 = genesis-funded faucet account
RECIPIENT_U8="${RECIPIENT_U8:-0}"        # read() allows caller==owner, so 0 is fine
BODY="${BODY:-dead-drop-secret-$$}"      # the payload that must vanish
INITIAL_ENERGY="${INITIAL_ENERGY:-120000}"
HALF_LIFE="${HALF_LIFE:-6}"              # small -> fast forgetting (the TTL knob)
DRY_RUN=false
VERBOSE=false
POLL_TIMEOUT_SEC=240

usage() { cat <<'EOF'
deploy-dead-drop.sh [options]
  --dry-run            validate + print intended calls; no network
  --node URL           node base URL (default http://89.167.52.40:8099)
  --token TOKEN        auth bearer ($EVAPORCHAIN_TX_TOKEN)
  --deployer U8        deployer/caller account index (default 0 = faucet)
  --recipient U8       set_payload recipient index (default 0)
  --body STR           payload string (must reach terminal evaporation)
  --energy N           initial contract energy (default 120000)
  --half-life N        decay rate; smaller = forgets sooner (default 6)
  --timeout SEC        per-phase poll timeout (default 240)
  --verbose            echo node responses
  -h|--help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN=true; shift ;;
    --node) NODE_URL="$2"; shift 2 ;;
    --token) TOKEN="$2"; shift 2 ;;
    --deployer) DEPLOYER_U8="$2"; shift 2 ;;
    --recipient) RECIPIENT_U8="$2"; shift 2 ;;
    --body) BODY="$2"; shift 2 ;;
    --energy) INITIAL_ENERGY="$2"; shift 2 ;;
    --half-life) HALF_LIFE="$2"; shift 2 ;;
    --timeout) POLL_TIMEOUT_SEC="$2"; shift 2 ;;
    --verbose) VERBOSE=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown flag: $1" >&2; usage; exit 2 ;;
  esac
done

log()  { printf '\033[1;36m[dead-drop]\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m[dead-drop ERROR]\033[0m %s\n' "$*" >&2; exit "${2:-1}"; }

curl_json() {  # <METHOD> <path> [body]
  local method="$1" path="$2" body="${3:-}"
  if $DRY_RUN; then echo "  [DRY-RUN] $method $NODE_URL$path ${body:+(body: $body)}" >&2; echo '{}'; return 0; fi
  local args=(-sS -m 30 -X "$method" -H 'Content-Type: application/json')
  [[ -n "$TOKEN" ]] && args+=(-H "Authorization: Bearer $TOKEN")
  [[ -n "$body" ]] && args+=(-d "$body")
  local resp; resp=$(curl "${args[@]}" "$NODE_URL$path") || die "curl $method $path failed" 2
  $VERBOSE && echo "  <- $resp" >&2
  printf '%s' "$resp"
}

submit_tx() {  # <path> <body> <name> <failcode> -> tx hash
  local resp; resp=$(curl_json POST "$1" "$2")
  $DRY_RUN && { echo "DRYHASH"; return 0; }
  local hash; hash=$(printf '%s' "$resp" | jq -r '.tx_hash // empty')
  [[ -n "$hash" ]] || die "$3 failed: $(printf '%s' "$resp" | jq -r '.message // .error // "(no msg)"')" "$4"
  printf '%s' "$hash"
}

poll_tx() {  # <hash> <name> <failcode> -> echoes final status JSON
  $DRY_RUN && { echo '{"state":"finalised","contract_id":0}'; return 0; }
  local deadline=$(( $(date +%s) + POLL_TIMEOUT_SEC )) resp st
  while (( $(date +%s) < deadline )); do
    resp=$(curl_json GET "/api/tx/$1") || true
    st=$(printf '%s' "$resp" | jq -r '.state // "unknown"')
    case "$st" in
      included|finalised) printf '%s' "$resp"; return 0 ;;
      rejected) die "$2 rejected: $(printf '%s' "$resp" | jq -r '.error // "?"')" "$3" ;;
    esac
    sleep 2
  done
  die "$2 not included within ${POLL_TIMEOUT_SEC}s" "$3"
}

get_epoch() { $DRY_RUN && { echo 0; return 0; }; curl_json GET "/api/status" | jq -r '.epoch // 0'; }

# ── preflight ──
[[ -f "$CONTRACT_PATH" ]] || die "contract not found: $CONTRACT_PATH" 2
grep -q "^contract MortalMessage" "$CONTRACT_PATH" || die ".es missing MortalMessage header" 3
grep -q "fn set_payload(" "$CONTRACT_PATH" || die ".es missing set_payload" 3
grep -q "fn read()" "$CONTRACT_PATH" || die ".es missing read" 3
if ! $DRY_RUN; then
  command -v curl >/dev/null || die "curl required" 2
  command -v jq   >/dev/null || die "jq required" 2
  curl -sS -m 5 "$NODE_URL/api/status" >/dev/null 2>&1 || die "node $NODE_URL unreachable" 2
fi

cat <<EOF
+==================================================================+
|  Dead Drop — forgetting e2e (mortal_message.es, corrected spec)  |
+------------------------------------------------------------------+
|  node: $NODE_URL   mode: $($DRY_RUN && echo DRY-RUN || echo LIVE)
|  deployer(u8): $DEPLOYER_U8   recipient(u8): $RECIPIENT_U8
|  energy/half-life: $INITIAL_ENERGY/$HALF_LIFE   body: "$BODY"
+==================================================================+
EOF

# ── 1. deploy ──
log "Step 1/5 - POST /api/tx/deploy-script (mortal_message.es)"
SRC=$(jq -Rs . < "$CONTRACT_PATH")
DBODY=$(jq -n --argjson d "$DEPLOYER_U8" --argjson s "$SRC" \
  --argjson e "$INITIAL_ENERGY" --argjson hl "$HALF_LIFE" \
  '{deployer:$d, source_code:$s, energy:$e, half_life:$hl}')
DH=$(submit_tx "/api/tx/deploy-script" "$DBODY" deploy 3)
log "deploy tx: $DH"
DSTAT=$(poll_tx "$DH" deploy 3)
CID=$(printf '%s' "$DSTAT" | jq -r '.contract_id // empty')
[[ -n "$CID" ]] || { $DRY_RUN && CID=0 || die "no contract_id on deploy tx status" 3; }
log "contract_id: $CID"

# ── 2. set_payload (seal once) ──
# args: [ {"Str": body}, {"Address":[recipient,0x31]} ]  (externally-tagged)
log "Step 2/5 - call-script set_payload(body, recipient)"
EP=$(get_epoch)
SBODY=$(jq -n --argjson c "$DEPLOYER_U8" --argjson cid "$CID" \
  --arg body "$BODY" --argjson r "$RECIPIENT_U8" --argjson ep "$EP" \
  '{caller:$c, contract_id:$cid, method:"set_payload",
    args:[ {Str:$body}, {Address: ([$r] + [range(0;31)|0])} ],
    epoch:$ep}')
SH=$(submit_tx "/api/tx/call-script" "$SBODY" set_payload 4)
poll_tx "$SH" set_payload 4 >/dev/null
log "payload sealed."

# ── 3+4. CONFIRM READABLE (non-vacuity) ──
# read() must finalise AND /api/script/:id must show the body, BEFORE
# any forgetting — otherwise a later disappearance proves nothing.
log "Step 3/5 - confirm readable while Active (non-vacuity guard)"
saw_readable=0
if $DRY_RUN; then
  log "[DRY-RUN] would call read() and assert .state.body == body"
else
  EP=$(get_epoch)
  RBODY=$(jq -n --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EP" \
    '{caller:$c, contract_id:$cid, method:"read", args:[], epoch:$ep}')
  RH=$(printf '%s' "$(curl_json POST /api/tx/call-script "$RBODY")" | jq -r '.tx_hash // empty')
  [[ -n "$RH" ]] && poll_tx "$RH" read 5 >/dev/null
  ST=$(curl_json GET "/api/script/$CID")
  GOTBODY=$(printf '%s' "$ST" | jq -r '.state.body.Str // .state.body // ""')
  if [[ "$GOTBODY" == "$BODY" ]]; then
    saw_readable=1
    log "CONFIRMED readable: GET /api/script/$CID .state.body == \"$BODY\""
  else
    die "payload not readable while Active (got: '$GOTBODY') — cannot prove forgetting from a never-readable drop" 5
  fi
fi

# ── 5. wait for the chain to FORGET it ──
log "Step 4/5 - poll GET /api/script/$CID until the contract evaporates"
if $DRY_RUN; then
  log "[DRY-RUN] would poll until evaporated==true (terminal liveness-death)"
  log "OK dry-run complete."; exit 0
fi
deadline=$(( $(date +%s) + POLL_TIMEOUT_SEC )); terminal=0
while (( $(date +%s) < deadline )); do
  ST=$(curl_json GET "/api/script/$CID")
  if printf '%s' "$ST" | jq -e 'has("error")' >/dev/null 2>&1; then
    terminal=1; log "contract $CID gone from script store (404 — rare; still terminal)"; break
  fi
  EVAP=$(printf '%s' "$ST" | jq -r '.evaporated // false')
  # NOTE (verified 2026-05-17, probe cid 17): get_script does NOT
  # purge .state.body and does NOT 404 post-evaporation (~300s window).
  # `evaporated==true` is the real terminal signal; "body empty / gone"
  # does not happen on this node. We assert ONLY the terminal
  # liveness-death, not byte-disappearance.
  if [[ "$EVAP" == "true" ]]; then
    terminal=1; log "note $CID reached terminal evaporated=true (energy-exhausted, unrefreshable)"; break
  fi
  sleep 4
done

# ── verdict ──
log "Step 5/5 - verdict"
(( saw_readable == 1 )) || die "non-vacuity: never confirmed a readable payload" 5
(( terminal == 1 )) || die "note never reached terminal evaporated=true within ${POLL_TIMEOUT_SEC}s — still alive past its TTL (real failure: terminal-evaporation guarantee violated)" 6
cat <<EOF

+==================================================================+
|         OK  DEAD DROP — TERMINAL EVAPORATION PROVEN              |
|  contract_id: $CID                                                |
|  proof: deploy ok  seal ok  CONFIRMED-READABLE ok  -> EVAPORATED  |
|  payload "$BODY" was readable on a live chain, then its contract  |
|  reached terminal evaporated=true (dead, unrefreshable) by        |
|  physics. HONEST SCOPE: get_script still returns the last         |
|  .state.body — this proves liveness-death, NOT byte-erasure.      |
+==================================================================+
EOF
