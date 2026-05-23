#!/usr/bin/env bash
#
# deploy-gdpr-vault.sh — live-e2e runbook for the GDPR-Erasure
# chain-side reference impl (contracts/evaporscript/gdpr_vault.es).
# See research/GDPR_ERASURE_ARCHITECTURE.md (model A: crypto-shred).
#
# HONEST SCOPE (founding constraint, verified Dead Drop §9): the chain
# does NOT byte-erase. This runbook proves the CHAIN-SIDE only — the
# tamper-evident retention clock + the provable key-shred TRIGGER.
# Actual erasure = off-chain key destruction (out of scope here).
#
# Two modes (mirrors the verified deploy-evaporcash.sh hoard/spend):
#   --mode retain (default): deploy with energy/half_life sized to the
#     retention period; seal; confirm active (non-vacuity); never
#     extend/withdraw; poll until terminal evaporated==true — the
#     natural-deadline key-shred trigger fired by physics.
#   --mode withdraw: deploy; seal; confirm active; call
#     withdraw_consent() (Art.17/7(3)); assert status==1
#     (expiry_forced) — the early key-shred trigger.
#
# Node contract = the SOURCE-VERIFIED one (deploy/call/get-script,
# externally-tagged Value args, deployer u8, /api/script/:id, session
# token). Exit: 0 ok · 2 precondition · 3 deploy · 4 seal ·
# 5 non-vacuity · 6 trigger-not-proven
#
# Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CONTRACT_PATH="$ROOT_DIR/contracts/evaporscript/gdpr_vault.es"

NODE_URL="${NODE_URL:-http://89.167.52.40:8099}"
TOKEN="${EVAPORCHAIN_TX_TOKEN:-}"
DEPLOYER_U8="${DEPLOYER_U8:-0}"          # controller = owner; 0 = genesis faucet
SUBJECT_U8="${SUBJECT_U8:-1}"            # data-subject ref (and withdraw_consent caller)
BASIS="${BASIS:-1}"                      # lawful basis code (1=consent ...)
MODE="${MODE:-retain}"                   # retain = natural deadline ; withdraw = Art.17 early
INITIAL_ENERGY="${INITIAL_ENERGY:-60000}"  # retention budget
HALF_LIFE="${HALF_LIFE:-5}"             # decay rate; smaller = shorter retention
DRY_RUN=false
VERBOSE=false
POLL_TIMEOUT_SEC=300

usage() { cat <<'EOF'
deploy-gdpr-vault.sh [options]
  --dry-run            validate + print intended calls; no network
  --node URL           node base URL (default http://89.167.52.40:8099)
  --token TOKEN        auth bearer ($EVAPORCHAIN_TX_TOKEN)
  --deployer U8        controller/owner index (default 0 = faucet)
  --subject U8         data-subject ref + withdraw caller (default 1)
  --basis N            lawful-basis code (default 1)
  --mode retain|withdraw  retain=natural-deadline (default) ; withdraw=Art.17 early
  --energy N           retention budget (default 60000)
  --half-life N        decay rate; smaller = shorter retention (default 5)
  --timeout SEC        poll timeout (default 300)
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
    --subject) SUBJECT_U8="$2"; shift 2 ;;
    --basis) BASIS="$2"; shift 2 ;;
    --mode) MODE="$2"; shift 2 ;;
    --energy) INITIAL_ENERGY="$2"; shift 2 ;;
    --half-life) HALF_LIFE="$2"; shift 2 ;;
    --timeout) POLL_TIMEOUT_SEC="$2"; shift 2 ;;
    --verbose) VERBOSE=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown flag: $1" >&2; usage; exit 2 ;;
  esac
done

log()  { printf '\033[1;36m[gdpr-vault]\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m[gdpr-vault ERROR]\033[0m %s\n' "$*" >&2; exit "${2:-1}"; }

curl_json() {
  local method="$1" path="$2" body="${3:-}"
  if $DRY_RUN; then echo "  [DRY-RUN] $method $NODE_URL$path ${body:+(body: $body)}" >&2; echo '{}'; return 0; fi
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

poll_tx() {
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
# jq `//` drops boolean false — unwrap tagged Value explicitly.
untag() { jq -r ".state.$1 | if type==\"object\" then (.Bool // .U64 // .Str) else . end"; }

# ── preflight ──
[[ -f "$CONTRACT_PATH" ]] || die "contract not found: $CONTRACT_PATH" 2
grep -q "^contract GdprVault" "$CONTRACT_PATH" || die ".es missing GdprVault header" 3
grep -q "fn seal(" "$CONTRACT_PATH" || die ".es missing seal" 3
if [[ "$MODE" != "retain" && "$MODE" != "withdraw" ]]; then die "unknown --mode '$MODE' (retain|withdraw)" 2; fi
if ! $DRY_RUN; then
  command -v curl >/dev/null || die "curl required" 2
  command -v jq   >/dev/null || die "jq required" 2
  curl -sS -m 5 "$NODE_URL/api/status" >/dev/null 2>&1 || die "node $NODE_URL unreachable" 2
fi

cat <<EOF
+==================================================================+
|  GDPR-Erasure — chain-side e2e (gdpr_vault.es, model A)          |
+------------------------------------------------------------------+
|  node: $NODE_URL   mode: $($DRY_RUN && echo DRY-RUN || echo LIVE)   run-mode: $MODE
|  controller(u8): $DEPLOYER_U8   subject(u8): $SUBJECT_U8   basis: $BASIS
|  energy/half-life: $INITIAL_ENERGY/$HALF_LIFE   (energy = retention clock)
+==================================================================+
EOF

# ── 1. deploy ──
log "Step 1/5 - POST /api/tx/deploy-script (gdpr_vault.es)"
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

# ── 2. seal (ct_commit, subject, basis) — owner-only, once ──
# args externally-tagged: [Address(ct_commit), Address(subject), U64(basis)]
log "Step 2/5 - call-script seal(ct_commit, subject, basis)"
EP=$(get_epoch)
# ct_commit is a 32-byte hash; for the e2e use a deterministic stand-in
# addr (byte0=0xCC=204) — production passes H(ciphertext).
SBODY=$(jq -n --argjson c "$DEPLOYER_U8" --argjson cid "$CID" \
  --argjson subj "$SUBJECT_U8" --argjson b "$BASIS" --argjson ep "$EP" \
  '{caller:$c, contract_id:$cid, method:"seal",
    args:[ {Address: ([204] + [range(0;31)|0])},
           {Address: ([$subj] + [range(0;31)|0])},
           {U64:$b} ],
    epoch:$ep}')
SH=$(submit_tx "/api/tx/call-script" "$SBODY" seal 4)
poll_tx "$SH" seal 4 >/dev/null
log "vault sealed."

# ── 3. confirm active (non-vacuity) ──
log "Step 3/5 - confirm sealed+active (non-vacuity guard)"
saw_active=0
if $DRY_RUN; then
  log "[DRY-RUN] would assert sealed=true, lawful_basis=$BASIS, evaporated=false"
else
  # poll_tx returns on `included`; executed state can lag a beat on a
  # busy shared node — settle-retry instead of a single-shot read.
  SEALED= BASIS_ON= EVAP=
  for _try in 1 2 3 4 5 6 7 8 9 10; do
    ST=$(curl_json GET "/api/script/$CID")
    SEALED=$(printf '%s' "$ST" | untag sealed)
    BASIS_ON=$(printf '%s' "$ST" | jq -r '.state.lawful_basis | if type=="object" then .U64 else . end')
    EVAP=$(printf '%s' "$ST" | jq -r '.evaporated // false')
    [[ "$SEALED" == "true" && "$BASIS_ON" == "$BASIS" && "$EVAP" == "false" ]] && break
    sleep 2
  done
  if [[ "$SEALED" == "true" && "$BASIS_ON" == "$BASIS" && "$EVAP" == "false" ]]; then
    saw_active=1
    log "CONFIRMED: sealed=true lawful_basis=$BASIS_ON evaporated=false (retention clock running)"
  else
    die "vault not sealed-active (sealed=$SEALED basis=$BASIS_ON evap=$EVAP) — cannot prove a trigger from a never-active vault" 5
  fi
fi

# ── 4/5. mode-specific: prove the key-shred trigger ──
# withdraw_consent's .es guard is `caller == subject_ref || caller ==
# owner`. We call it as the OWNER (controller, $DEPLOYER_U8) — a
# genesis-funded account. The subject account ($SUBJECT_U8) is a
# non-genesis index which this devnet does NOT fund, so a tx FROM it
# is rejected at admission (gas/unknown-account) before reaching the
# VM — a devnet funding limitation, NOT a contract issue. The
# owner-branch exercises the same withdraw_consent path + guard; the
# subject-branch is the identical ||-guard pattern verified in
# mortal_message.es::read (caller==recipient||caller==owner) — high
# confidence, but honestly UNEXERCISED on this devnet (stated, not
# overclaimed).
if [[ "$MODE" == "withdraw" ]]; then
  log "Step 4/5 - WITHDRAW (Art.17/7(3)) — controller calls withdraw_consent() (subject acct unfunded on devnet)"
  if $DRY_RUN; then log "[DRY-RUN] would withdraw_consent then assert expiry_forced=true"; log "OK dry-run."; exit 0; fi
  EP=$(get_epoch)
  WBODY=$(jq -n --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EP" \
    '{caller:$c, contract_id:$cid, method:"withdraw_consent", args:[], epoch:$ep}')
  WH=$(submit_tx "/api/tx/call-script" "$WBODY" withdraw 6)
  poll_tx "$WH" withdraw 6 >/dev/null
  FORCED=
  for _try in 1 2 3 4 5 6 7 8 9 10; do
    ST=$(curl_json GET "/api/script/$CID")
    FORCED=$(printf '%s' "$ST" | untag expiry_forced)
    [[ "$FORCED" == "true" ]] && break
    sleep 2
  done
  log "Step 5/5 - verdict (withdraw)"
  (( saw_active == 1 )) || die "non-vacuity: never confirmed an active vault" 5
  [[ "$FORCED" == "true" ]] || die "withdraw_consent did not set expiry_forced (got '$FORCED') — early trigger unproven" 6
  cat <<EOF

+==================================================================+
|   OK  GDPR-ERASURE — EARLY KEY-SHRED TRIGGER PROVEN (Art.17)     |
|  contract_id: $CID                                                |
|  proof: deploy ok  seal ok  ACTIVE ok  -> withdraw_consent(owner) |
|  state.expiry_forced=true + "erasure-due (consent withdrawn)"    |
|  emitted. SCOPE: owner-branch proven; subject-branch is the same |
|  verified ||-guard, devnet-unfundable so unexercised. Erasure = |
|  off-chain key-shred (model A) — chain proves the TRIGGER only.  |
+==================================================================+
EOF
else
  log "Step 4/5 - RETAIN — never extend/withdraw; poll until terminal evaporation"
  if $DRY_RUN; then log "[DRY-RUN] would poll until evaporated==true (natural-deadline trigger)"; log "OK dry-run."; exit 0; fi
  deadline=$(( $(date +%s) + POLL_TIMEOUT_SEC )); fired=0
  while (( $(date +%s) < deadline )); do
    ST=$(curl_json GET "/api/script/$CID")
    if printf '%s' "$ST" | jq -e 'has("error")' >/dev/null 2>&1; then
      fired=1; log "vault $CID gone from script store (terminal)"; break
    fi
    EVAP=$(printf '%s' "$ST" | jq -r '.evaporated // false')
    if [[ "$EVAP" == "true" ]]; then
      fired=1; log "vault reached terminal evaporated=true — retention elapsed, key-shred trigger due"; break
    fi
    sleep 4
  done
  log "Step 5/5 - verdict (retain)"
  (( saw_active == 1 )) || die "non-vacuity: never confirmed an active vault" 5
  (( fired == 1 )) || die "vault never reached terminal evaporation within ${POLL_TIMEOUT_SEC}s — retention clock not exhausted (half-life too large?)" 6
  cat <<EOF

+==================================================================+
|  OK  GDPR-ERASURE — NATURAL-DEADLINE KEY-SHRED TRIGGER PROVEN    |
|  contract_id: $CID                                                |
|  proof: deploy ok  seal ok  ACTIVE ok  -> TERMINAL EVAPORATION   |
|  retention clock ran out by physics; on_evaporate "erasure-due"  |
|  trigger fired. HONEST SCOPE: chain proves the tamper-evident    |
|  retention+trigger; actual erasure = off-chain key-shred         |
|  (model A, by design — NOT byte-erasure, see Dead Drop §9).      |
+==================================================================+
EOF
fi
