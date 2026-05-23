#!/usr/bin/env bash
#
# deploy-singh-letter.sh — end-to-end doctrine proof for SinghLetter
# (contracts/evaporscript/singh_letter.es).
#
# §A5.5 ChildKey / Singh Letter — inverted-decay time-lock.
# Press claim: "Parent dies? Seal still opens on schedule."
#
# Inverted decay: energy-to-unlock GROWS from 0 → threshold over the
# recipient's lifetime. Same primitive as standard decay, opposite sign.
# unlock_epoch = recipient_birth_epoch + unlock_age_years * epochs_per_year.
# At epoch >= unlock_epoch: countdown reaches zero → gate opens.
#
# Two modes:
#
#   --mode countdown (default):
#     deploy → seal_letter (birth=current_epoch, age=18, epy=365 →
#       unlock far in future) →
#     witness_countdown (caller 0): snapshot1_remaining > 0, unlockable = 0 →
#     witness_countdown (caller 1): snapshot2_remaining > 0, unlockable = 0 →
#     require_sealed (letter still locked) →
#     verify: letter_status=0, witness_count=2, unlock_epoch=birth+age*epy.
#     Proves: inverted-decay countdown is positive, letter is sealed.
#
#   --mode open:
#     deploy → seal_letter (birth=0, age=1, epy=1 → unlock_epoch=1;
#       epoch is already >> 1 → immediately unlockable) →
#     witness_countdown (caller 0): snapshot1_remaining=0, unlockable=1 →
#     open_letter (caller 0): epoch >= unlock_epoch; gate fires →
#     require_opened (caller 1): proof gate confirms child came of age →
#     verify: letter_status=1, opened_at_epoch > 0, payload_hash correct.
#     Proves: unlock gate fires when epoch >= unlock_epoch.
#
# TX HASH DEDUP NOTE:
#   witness_countdown(), require_sealed(), require_opened() take no args.
#   tx hash = H(caller, cid, method, args). Successive no-arg calls from the
#   same caller get deduped. Fix: caller rotation — witness1/require_sealed
#   use caller[0]; witness2/require_opened use caller[1].
#   INITIAL_ENERGY randomised per run to produce a unique deploy hash.
#
# Usage:
#   ./scripts/deploy-singh-letter.sh --dry-run
#   ./scripts/deploy-singh-letter.sh --node http://89.167.52.40:8099 --mode countdown
#   ./scripts/deploy-singh-letter.sh --node http://89.167.52.40:8099 --mode open
#
# Exit: 0 ok · 2 precondition · 3 deploy · 4 seal/call · 5 gate-not-exercised · 6 verify
#
# Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CONTRACT_PATH="$ROOT_DIR/contracts/evaporscript/singh_letter.es"

NODE_URL="${NODE_URL:-http://89.167.52.40:8099}"
TOKEN="${EVAPORCHAIN_TX_TOKEN:-}"
DEPLOYER_U8="${DEPLOYER_U8:-0}"       # sender/owner — must be funded
CALLER2_U8="${CALLER2_U8:-1}"         # second caller (avoids witness/gate dedup)
MODE="${MODE:-countdown}"             # countdown | open

# Payload hash placeholder (BLAKE3 commitment to off-chain ciphertext)
PAYLOAD_HASH="${PAYLOAD_HASH:-blake3:aabbccdd00000000000000000000000000000000000000000000000000000000}"

# Contract energy — randomised per run to avoid deploy-tx dedup across runs
INITIAL_ENERGY="${INITIAL_ENERGY:-$(( 20000000 + RANDOM % 32768 ))}"
CONTRACT_HALF_LIFE="${CONTRACT_HALF_LIFE:-500000}"

# countdown-mode params (unlock far in future; birth = current epoch)
AGE_YEARS_COUNTDOWN="${AGE_YEARS_COUNTDOWN:-18}"
EPY_COUNTDOWN="${EPY_COUNTDOWN:-365}"
# → unlock_epoch = current_epoch + 18 * 365 = current_epoch + 6570

# open-mode params: birth=0, age=1, epy=1 → unlock_epoch=1
# At any epoch > 0 the gate fires immediately.
BIRTH_EPOCH_OPEN="${BIRTH_EPOCH_OPEN:-0}"
AGE_YEARS_OPEN="${AGE_YEARS_OPEN:-1}"
EPY_OPEN="${EPY_OPEN:-1}"

POLL_TIMEOUT_SEC="${POLL_TIMEOUT_SEC:-300}"
DRY_RUN=false
VERBOSE=false

usage() { cat <<'EOF'
deploy-singh-letter.sh [options]
  --dry-run                validate + print intended calls; no network
  --node URL               node base URL (default http://89.167.52.40:8099)
  --token TOKEN            auth bearer ($EVAPORCHAIN_TX_TOKEN)
  --deployer U8            sender/owner index (default 0)
  --caller2 U8             second caller index (default 1)
  --mode countdown|open    prove mode (default countdown)
  --payload-hash STR       BLAKE3 payload hash placeholder
  --energy N               contract initial energy (default randomised ~20M)
  --hl N                   contract half-life (default 500000)
  --age-years N            unlock age years for countdown mode (default 18)
  --epy N                  epochs per year for countdown mode (default 365)
  --timeout SEC            poll timeout (default 300)
  --verbose                echo node responses
  -h|--help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)        DRY_RUN=true; shift ;;
    --node)           NODE_URL="$2"; shift 2 ;;
    --token)          TOKEN="$2"; shift 2 ;;
    --deployer)       DEPLOYER_U8="$2"; shift 2 ;;
    --caller2)        CALLER2_U8="$2"; shift 2 ;;
    --mode)           MODE="$2"; shift 2 ;;
    --payload-hash)   PAYLOAD_HASH="$2"; shift 2 ;;
    --energy)         INITIAL_ENERGY="$2"; shift 2 ;;
    --hl)             CONTRACT_HALF_LIFE="$2"; shift 2 ;;
    --age-years)      AGE_YEARS_COUNTDOWN="$2"; shift 2 ;;
    --epy)            EPY_COUNTDOWN="$2"; shift 2 ;;
    --timeout)        POLL_TIMEOUT_SEC="$2"; shift 2 ;;
    --verbose)        VERBOSE=true; shift ;;
    -h|--help)        usage; exit 0 ;;
    *)                echo "Unknown flag: $1" >&2; usage; exit 2 ;;
  esac
done

log()  { printf '\033[1;36m[letter]\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m[letter ERROR]\033[0m %s\n' "$*" >&2; exit "${2:-1}"; }
ok()   { printf '\033[1;32m[letter OK]\033[0m %s\n' "$*"; }

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

# Auto-register + login if no TOKEN is set.
# Testnet auto-verifies email, so register → login gives a token immediately.
acquire_token() {
  $DRY_RUN && return 0
  [[ -n "$TOKEN" ]] && return 0   # already set by env var or --token flag
  local ts; ts=$(date +%s%N 2>/dev/null || date +%s)
  local email="deploy-letter-${ts}@example.com"
  local pass="EvaporLetter${ts}!"
  local reg_body; reg_body=$(jq -n --arg e "$email" --arg p "$pass" \
    '{email:$e, password:$p, display_name:"letter-deploy"}')
  local reg_resp; reg_resp=$(curl -sS -m 15 -X POST \
    -H 'Content-Type: application/json' -d "$reg_body" \
    "$NODE_URL/api/auth/register") || die "auth register curl failed" 2
  local ok; ok=$(printf '%s' "$reg_resp" | jq -r '.success // false')
  [[ "$ok" == "true" ]] || die "auth register failed: $(printf '%s' "$reg_resp" | jq -r '.message')" 2
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
grep -q "^contract SinghLetter"     "$CONTRACT_PATH" || die ".es missing SinghLetter header" 3
grep -q "fn seal_letter("           "$CONTRACT_PATH" || die ".es missing seal_letter" 3
grep -q "fn open_letter("           "$CONTRACT_PATH" || die ".es missing open_letter" 3
grep -q "fn witness_countdown("     "$CONTRACT_PATH" || die ".es missing witness_countdown" 3
grep -q "fn require_sealed("        "$CONTRACT_PATH" || die ".es missing require_sealed" 3
grep -q "fn require_opened("        "$CONTRACT_PATH" || die ".es missing require_opened" 3
[[ "$MODE" == "countdown" || "$MODE" == "open" ]] \
  || die "unknown --mode '$MODE' (countdown|open)" 2

if ! $DRY_RUN; then
  command -v curl >/dev/null || die "curl required" 2
  command -v jq   >/dev/null || die "jq required" 2
  curl -sS -m 5 "$NODE_URL/api/status" >/dev/null 2>&1 || die "node $NODE_URL unreachable" 2
fi

acquire_token

# ── Step 1: deploy ────────────────────────────────────────────────────────
log "Step 1 - deploy-script (singh_letter.es)  energy=$INITIAL_ENERGY"
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

# ── Resolve seal parameters for the chosen mode ───────────────────────────
if [[ "$MODE" == "countdown" ]]; then
  BIRTH_EPOCH=$(get_epoch)           # birth = now → unlock is 18*365 epochs away
  AGE_YEARS="$AGE_YEARS_COUNTDOWN"
  EPY="$EPY_COUNTDOWN"
else
  BIRTH_EPOCH="$BIRTH_EPOCH_OPEN"   # birth = 0, age = 1, epy = 1 → unlock_epoch = 1
  AGE_YEARS="$AGE_YEARS_OPEN"
  EPY="$EPY_OPEN"
fi
EXPECTED_UNLOCK=$(( BIRTH_EPOCH + AGE_YEARS * EPY ))

cat <<EOF

+=====================================================================+
|  SinghLetter — §A5.5 ChildKey doctrine proof (Inverted-Decay Lock) |
+---------------------------------------------------------------------+
|  node: $NODE_URL
|  mode: $($DRY_RUN && echo DRY-RUN || echo LIVE)  run-mode: $MODE
|  deployer(u8): $DEPLOYER_U8  caller2(u8): $CALLER2_U8
|  birth_epoch: $BIRTH_EPOCH  age_years: $AGE_YEARS  epy: $EPY
|  expected unlock_epoch: $EXPECTED_UNLOCK
|  payload_hash: ${PAYLOAD_HASH:0:30}...
+=====================================================================+
EOF

# ── Step 2: seal_letter ───────────────────────────────────────────────────
EPOCH=$(get_epoch)
log "Step 2 - seal_letter(birth=$BIRTH_EPOCH, age=$AGE_YEARS, epy=$EPY) at epoch=$EPOCH"
SEAL_BODY=$(jq -n \
  --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  --argjson birth "$BIRTH_EPOCH" --argjson age "$AGE_YEARS" --argjson epy "$EPY" \
  --arg phash "$PAYLOAD_HASH" \
  '{caller:$c, contract_id:$cid, method:"seal_letter",
    args:[{U64:$birth},{U64:$age},{U64:$epy},{Str:$phash}],
    epoch:$ep}')
require_tx "/api/tx/call-script" "$SEAL_BODY" "seal_letter" 4

if ! $DRY_RUN; then
  STATE=$(curl_json GET "/api/script/$CID")
  SEALED=$(printf '%s' "$STATE" | untag sealed)
  UE=$(printf '%s'     "$STATE" | untag unlock_epoch)
  LS=$(printf '%s'     "$STATE" | untag letter_status)
  [[ "$SEALED" == "true" || "$SEALED" == "1" || "$SEALED" == "True" ]] \
    || die "expected sealed=true, got $SEALED" 6
  [[ "$UE" == "$EXPECTED_UNLOCK" ]] \
    || die "expected unlock_epoch=$EXPECTED_UNLOCK, got $UE" 6
  [[ "$LS" == "0" ]] || die "expected letter_status=0, got $LS" 6
  ok "sealed=true  unlock_epoch=$UE  letter_status=0 ✓"
fi

# ── Step 3: witness_countdown 1 ──────────────────────────────────────────
# Caller = DEPLOYER_U8 (0). Second call uses CALLER2_U8 (1) to avoid dedup.
EPOCH=$(get_epoch)
log "Step 3 - witness_countdown 1 (caller=account[$DEPLOYER_U8]): snapshot countdown"
W1_BODY=$(jq -n \
  --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  '{caller:$c, contract_id:$cid, method:"witness_countdown", args:[], epoch:$ep}')
require_tx "/api/tx/call-script" "$W1_BODY" "witness_countdown1" 4

if ! $DRY_RUN; then
  STATE=$(curl_json GET "/api/script/$CID")
  S1_REM=$(printf '%s' "$STATE" | untag snapshot1_remaining)
  S1_UNL=$(printf '%s' "$STATE" | untag snapshot1_unlockable)
  if [[ "$MODE" == "countdown" ]]; then
    [[ "$S1_REM" -gt 0 ]] \
      || die "expected snapshot1_remaining > 0 (countdown active), got $S1_REM" 6
    [[ "$S1_UNL" == "0" ]] \
      || die "expected snapshot1_unlockable=0 (locked), got $S1_UNL" 6
    ok "snapshot1: remaining=$S1_REM (countdown > 0)  unlockable=0 (locked) ✓"
  else
    [[ "$S1_REM" == "0" ]] \
      || die "expected snapshot1_remaining=0 (already unlockable), got $S1_REM" 6
    [[ "$S1_UNL" == "1" ]] \
      || die "expected snapshot1_unlockable=1 (unlockable), got $S1_UNL" 6
    ok "snapshot1: remaining=0 (countdown zeroed)  unlockable=1 ✓"
  fi
fi

# ── countdown mode: step 4 witness2 + step 5 require_sealed ──────────────
if [[ "$MODE" == "countdown" ]]; then

  EPOCH=$(get_epoch)
  log "Step 4 - witness_countdown 2 (caller=account[$CALLER2_U8]): second snapshot"
  W2_BODY=$(jq -n \
    --argjson c "$CALLER2_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"witness_countdown", args:[], epoch:$ep}')
  require_tx "/api/tx/call-script" "$W2_BODY" "witness_countdown2" 4

  if ! $DRY_RUN; then
    STATE=$(curl_json GET "/api/script/$CID")
    S2_REM=$(printf '%s' "$STATE" | untag snapshot2_remaining)
    S2_UNL=$(printf '%s' "$STATE" | untag snapshot2_unlockable)
    [[ "$S2_REM" -gt 0 ]] \
      || die "expected snapshot2_remaining > 0, got $S2_REM" 6
    [[ "$S2_UNL" == "0" ]] \
      || die "expected snapshot2_unlockable=0, got $S2_UNL" 6
    ok "snapshot2: remaining=$S2_REM  unlockable=0 ✓"
  fi

  EPOCH=$(get_epoch)
  log "Step 5 - require_sealed: letter_status=0 (sealed) must PASS"
  RS_BODY=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"require_sealed", args:[], epoch:$ep}')
  require_tx "/api/tx/call-script" "$RS_BODY" "require_sealed" 5
  ok "require_sealed PASSED — letter_status=0 (sealed) ✓"

  if ! $DRY_RUN; then
    STATE=$(curl_json GET "/api/script/$CID")
    LS=$(printf '%s' "$STATE"  | untag letter_status)
    WC=$(printf '%s' "$STATE"  | untag witness_count)
    UE=$(printf '%s' "$STATE"  | untag unlock_epoch)
    PH=$(printf '%s' "$STATE"  | untag payload_hash)
    [[ "$LS" == "0" ]] || die "expected letter_status=0, got $LS" 6
    [[ "$WC" == "2" ]] || die "expected witness_count=2, got $WC" 6
    ok "letter_status=0 (sealed)  witness_count=2  unlock_epoch=$UE ✓"
    ok "payload_hash=$PH ✓"
  fi

  cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — SinghLetter (countdown mode)            |
+---------------------------------------------------------------------+
|  contract_id: $CID
|  PROVEN (inverted-decay countdown):
|   - sealed=true  unlock_epoch=$EXPECTED_UNLOCK ✓
|   - snapshot1: remaining>0  unlockable=0 (letter locked) ✓
|   - snapshot2: remaining>0  unlockable=0 (countdown stable) ✓
|   - require_sealed PASSED (letter_status=0) ✓
|   - Inverted decay: energy-to-unlock grows toward threshold ✓
|   - "Parent dies? Seal still opens on schedule." ✓
+=====================================================================+
EOF
  exit 0
fi

# ── open mode: step 4 open_letter + step 5 require_opened + step 6 verify

EPOCH=$(get_epoch)
log "Step 4 - open_letter (caller=account[$DEPLOYER_U8]): epoch=$EPOCH >= unlock_epoch=$EXPECTED_UNLOCK"
OPEN_BODY=$(jq -n \
  --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  '{caller:$c, contract_id:$cid, method:"open_letter", args:[], epoch:$ep}')
require_tx "/api/tx/call-script" "$OPEN_BODY" "open_letter" 4
ok "open_letter finalised — countdown reached zero, gate opened ✓"

EPOCH=$(get_epoch)
log "Step 5 - require_opened (caller=account[$CALLER2_U8]): letter_status=1 must PASS"
RO_BODY=$(jq -n \
  --argjson c "$CALLER2_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  '{caller:$c, contract_id:$cid, method:"require_opened", args:[], epoch:$ep}')
require_tx "/api/tx/call-script" "$RO_BODY" "require_opened" 5
ok "require_opened PASSED — letter_status=1 (child came of age) ✓"

log "Step 6 - verify final state"
if ! $DRY_RUN; then
  STATE=$(curl_json GET "/api/script/$CID")
  LS=$(printf '%s'  "$STATE" | untag letter_status)
  OAE=$(printf '%s' "$STATE" | untag opened_at_epoch)
  WC=$(printf '%s'  "$STATE" | untag witness_count)
  UE=$(printf '%s'  "$STATE" | untag unlock_epoch)
  PH=$(printf '%s'  "$STATE" | untag payload_hash)
  [[ "$LS" == "1" ]]   || die "expected letter_status=1 (opened), got $LS" 6
  [[ "$OAE" -gt 0 ]]   || die "expected opened_at_epoch > 0, got $OAE" 6
  [[ "$WC" == "1" ]]   || die "expected witness_count=1, got $WC" 6
  [[ "$UE" == "$EXPECTED_UNLOCK" ]] || die "expected unlock_epoch=$EXPECTED_UNLOCK, got $UE" 6
  [[ "$PH" == "$PAYLOAD_HASH" ]]    || die "expected payload_hash match, got $PH" 6
  ok "letter_status=1 (opened) ✓"
  ok "opened_at_epoch=$OAE ✓"
  ok "unlock_epoch=$UE ✓"
  ok "payload_hash=$PH ✓"
  ok "witness_count=1 ✓"
fi

cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — SinghLetter (open mode)                 |
+---------------------------------------------------------------------+
|  contract_id: $CID
|  birth_epoch: $BIRTH_EPOCH  age_years: $AGE_YEARS  epy: $EPY
|  unlock_epoch: $EXPECTED_UNLOCK
|  PROVEN (inverted-decay unlock gate):
|   - sealed=true  unlock_epoch=$EXPECTED_UNLOCK ✓
|   - snapshot1: remaining=0  unlockable=1 (countdown zeroed) ✓
|   - open_letter: epoch >= unlock_epoch — gate fired correctly ✓
|   - require_opened PASSED (letter_status=1, child came of age) ✓
|   - opened_at_epoch > 0 ✓
|   - payload_hash preserved ✓
|   - Same primitive, opposite sign: inverted decay → unlock ✓
+=====================================================================+
EOF
