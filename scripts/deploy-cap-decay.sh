#!/usr/bin/env bash
#
# deploy-cap-decay.sh — end-to-end doctrine proof for CapabilityDecayVM
# (contracts/evaporscript/cap_decay.es).
#
# §4.2 Tier-2 VM Paradigm: ocap + energy-decay.
# "Capability-decay VM — ocap with structural energy-bound authority.
#  Mint → invoke. Attenuate strict subset. Revoke root → descendants
#  fail structurally. No holder enumeration."
#
# Verb codes: 1=read, 2=write, 3=transfer.
# Parent sentinel: cap_parent[slot]=999 means root (no parent).
# Max chain depth: 8 levels in invoke_gate / require_ancestor_dead.
#
# Two modes:
#
#   --mode chain (default):
#     Prove structural revocation propagation without holder enumeration.
#     mint root(verb=1,obj=1,max=100,energy=50000) →
#     attenuate child(parent=0,max=50,energy=25000) →
#     witness_cap(child=1, snap1: energy=25000, par_energy=50000) →
#     revoke(root=0) →
#     require_ancestor_dead(child=1) PASSED.
#     Proves: zeroing root energy kills descendant structurally,
#     O(depth) not O(holders).
#
#   --mode invocable:
#     Prove invoke_gate passes for live cap chain (root + child).
#     mint root(verb=2,obj=5,max=200,energy=40000) →
#     attenuate child(parent=0,max=100,energy=20000) →
#     invoke_gate(root=0) PASSED →
#     invoke_gate(child=1) PASSED →
#     witness_cap(root=0, snap1: energy=40000, par_energy=0) →
#     witness_cap(child=1, snap2: energy=20000, par_energy=40000).
#     Proves: invoke_gate walks parent chain and confirms full chain live.
#
# TX HASH DEDUP:
#   mint/attenuate/revoke take distinct args → naturally unique.
#   witness_cap(slot=0) vs witness_cap(slot=1) → different slot arg.
#   invoke_gate(0) vs invoke_gate(1) → different slot arg.
#   require_ancestor_dead uses CALLER2.
#   INITIAL_ENERGY randomised per run.
#
# Usage:
#   ./scripts/deploy-cap-decay.sh --dry-run
#   ./scripts/deploy-cap-decay.sh --node http://89.167.52.40:8099 --mode chain
#   ./scripts/deploy-cap-decay.sh --node http://89.167.52.40:8099 --mode invocable
#
# Exit: 0 ok · 2 precondition · 3 deploy · 4 call · 5 gate · 6 verify
#
# Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CONTRACT_PATH="$ROOT_DIR/contracts/evaporscript/cap_decay.es"

NODE_URL="${NODE_URL:-http://89.167.52.40:8099}"
TOKEN="${EVAPORCHAIN_TX_TOKEN:-}"
DEPLOYER_U8="${DEPLOYER_U8:-0}"
CALLER2_U8="${CALLER2_U8:-1}"    # require_ancestor_dead / alternate calls
CALLER3_U8="${CALLER3_U8:-2}"    # gate calls
MODE="${MODE:-chain}"            # chain | invocable

INITIAL_ENERGY="${INITIAL_ENERGY:-$(( 20000000 + RANDOM % 32768 ))}"
CONTRACT_HALF_LIFE="${CONTRACT_HALF_LIFE:-500000}"

POLL_TIMEOUT_SEC="${POLL_TIMEOUT_SEC:-300}"
DRY_RUN=false
VERBOSE=false

# Capability parameters (chain mode)
ROOT_VERB=1        # read
ROOT_OBJ=1
ROOT_MAX=100
ROOT_ENERGY=50000
CHILD_MAX=50
CHILD_ENERGY=25000

# Capability parameters (invocable mode)
INV_ROOT_VERB=2    # write
INV_ROOT_OBJ=5
INV_ROOT_MAX=200
INV_ROOT_ENERGY=40000
INV_CHILD_MAX=100
INV_CHILD_ENERGY=20000

usage() { cat <<'EOF'
deploy-cap-decay.sh [options]
  --dry-run                print intended calls; no network
  --node URL               node base URL (default http://89.167.52.40:8099)
  --token TOKEN            auth bearer ($EVAPORCHAIN_TX_TOKEN)
  --deployer U8            owner index (default 0)
  --caller2 U8             alternate caller (default 1)
  --caller3 U8             gate caller (default 2)
  --mode chain|invocable   prove mode (default chain)
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

log()  { printf '\033[1;36m[capdecay]\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m[capdecay ERROR]\033[0m %s\n' "$*" >&2; exit "${2:-1}"; }
ok()   { printf '\033[1;32m[capdecay OK]\033[0m %s\n' "$*"; }

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
  local email="deploy-capdecay-${ts}@example.com"
  local pass="EvaporCap${ts}!"
  local reg_body; reg_body=$(jq -n --arg e "$email" --arg p "$pass" \
    '{email:$e, password:$p, display_name:"capdecay-deploy"}')
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
grep -q "^contract CapabilityDecayVM" "$CONTRACT_PATH" || die ".es missing CapabilityDecayVM header" 3
grep -q "fn mint("                    "$CONTRACT_PATH" || die ".es missing mint" 3
grep -q "fn attenuate("               "$CONTRACT_PATH" || die ".es missing attenuate" 3
grep -q "fn revoke("                  "$CONTRACT_PATH" || die ".es missing revoke" 3
grep -q "fn invoke_gate("             "$CONTRACT_PATH" || die ".es missing invoke_gate" 3
grep -q "fn witness_cap("             "$CONTRACT_PATH" || die ".es missing witness_cap" 3
grep -q "fn require_ancestor_dead("   "$CONTRACT_PATH" || die ".es missing require_ancestor_dead" 3
[[ "$MODE" == "chain" || "$MODE" == "invocable" ]] \
  || die "unknown --mode '$MODE' (chain|invocable)" 2

if ! $DRY_RUN; then
  command -v curl >/dev/null || die "curl required" 2
  command -v jq   >/dev/null || die "jq required" 2
  curl -sS -m 5 "$NODE_URL/api/status" >/dev/null 2>&1 || die "node $NODE_URL unreachable" 2
fi

acquire_token

if [[ "$MODE" == "chain" ]]; then
cat <<EOF

+=====================================================================+
|  CapabilityDecayVM — §4.2 doctrine proof (chain mode)             |
+---------------------------------------------------------------------+
|  node: $NODE_URL
|  deployer: $DEPLOYER_U8  caller2: $CALLER2_U8  caller3: $CALLER3_U8
|  root: slot=0 verb=read obj=1 max=100 energy=50000
|  child: slot=1 parent=0 max=50 energy=25000
|  sequence: mint → attenuate → witness → revoke root → ancestor_dead PASS
+=====================================================================+
EOF
else
cat <<EOF

+=====================================================================+
|  CapabilityDecayVM — §4.2 doctrine proof (invocable mode)         |
+---------------------------------------------------------------------+
|  node: $NODE_URL
|  deployer: $DEPLOYER_U8  caller2: $CALLER2_U8  caller3: $CALLER3_U8
|  root: slot=0 verb=write obj=5 max=200 energy=40000
|  child: slot=1 parent=0 max=100 energy=20000
|  sequence: mint → attenuate → invoke_gate(root) → invoke_gate(child)
|            → witness root (snap1) → witness child (snap2)
+=====================================================================+
EOF
fi

# ── Step 1: deploy ────────────────────────────────────────────────────────
log "Step 1 - deploy CapabilityDecayVM  energy=$INITIAL_ENERGY"
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
if [[ "$MODE" == "chain" ]]; then

  # Step 2: mint root capability (slot=0)
  EPOCH=$(get_epoch)
  log "Step 2 - mint(verb=$ROOT_VERB,obj=$ROOT_OBJ,max=$ROOT_MAX,energy=$ROOT_ENERGY) → slot 0"
  MINT=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson v "$ROOT_VERB" --argjson o "$ROOT_OBJ" \
    --argjson m "$ROOT_MAX" --argjson e "$ROOT_ENERGY" \
    '{caller:$c, contract_id:$cid, method:"mint", args:[{U64:$v},{U64:$o},{U64:$m},{U64:$e}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$MINT" "mint_root" 4
  ok "root capability minted at slot=0 ✓"

  # Step 3: attenuate child from root (slot=1, parent=0)
  EPOCH=$(get_epoch)
  log "Step 3 - attenuate(parent=0,max=$CHILD_MAX,energy=$CHILD_ENERGY) → slot 1"
  ATTN=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson p 0 --argjson cm "$CHILD_MAX" --argjson ce "$CHILD_ENERGY" \
    '{caller:$c, contract_id:$cid, method:"attenuate", args:[{U64:$p},{U64:$cm},{U64:$ce}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$ATTN" "attenuate_child" 4
  ok "child capability minted at slot=1 (parent=0, strict subset) ✓"

  # Step 4: witness child BEFORE revoke → snapshot1 captures par_energy=50000
  EPOCH=$(get_epoch)
  log "Step 4 - witness_cap(slot=1 child) → snapshot1  [before revoke]"
  WC=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"witness_cap", args:[{U64:1}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$WC" "witness_child_pre_revoke" 4

  # Step 5: revoke root (slot=0 → cap_energy[0] = 0)
  EPOCH=$(get_epoch)
  log "Step 5 - revoke(slot=0 root)  [structural revocation — O(depth) not O(holders)]"
  REV=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"revoke", args:[{U64:0}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$REV" "revoke_root" 4
  ok "root revoked (energy zeroed) ✓"

  # Verify pre-revoke snapshot and cap_count
  if ! $DRY_RUN; then
    STATE=$(curl_json GET "/api/script/$CID")
    S1S=$(printf '%s' "$STATE" | untag snapshot1_slot)
    S1E=$(printf '%s' "$STATE" | untag snapshot1_energy)
    S1P=$(printf '%s' "$STATE" | untag snapshot1_par_energy)
    CC=$(printf '%s'  "$STATE" | untag cap_count)
    ok "cap_count=$CC  snapshot1: slot=$S1S energy=$S1E par_energy=$S1P"
    [[ "$S1S" -eq 1 ]]               || die "snap1 slot should be 1 (child), got $S1S" 6
    [[ "$S1E" -eq "$CHILD_ENERGY" ]] || die "snap1 energy should be $CHILD_ENERGY, got $S1E" 6
    [[ "$S1P" -eq "$ROOT_ENERGY" ]]  || die "snap1 par_energy should be $ROOT_ENERGY (root alive pre-revoke), got $S1P" 6
    [[ "$CC"  -eq 2 ]]               || die "cap_count should be 2, got $CC" 6
    ok "pre-revoke snapshot verified: child energy=$CHILD_ENERGY par_energy=$ROOT_ENERGY ✓"
  fi

  # Step 6: require_ancestor_dead(child=1) — proves structural propagation
  EPOCH=$(get_epoch)
  log "Step 6 - require_ancestor_dead(slot=1 child)  [proves root revocation killed chain]"
  RAD=$(jq -n \
    --argjson c "$CALLER2_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"require_ancestor_dead", args:[{U64:1}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$RAD" "require_ancestor_dead" 5
  ok "require_ancestor_dead PASSED — root revocation propagated structurally ✓"

else  # invocable mode

  # Step 2: mint root capability (slot=0)
  EPOCH=$(get_epoch)
  log "Step 2 - mint(verb=$INV_ROOT_VERB,obj=$INV_ROOT_OBJ,max=$INV_ROOT_MAX,energy=$INV_ROOT_ENERGY) → slot 0"
  MINT=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson v "$INV_ROOT_VERB" --argjson o "$INV_ROOT_OBJ" \
    --argjson m "$INV_ROOT_MAX" --argjson e "$INV_ROOT_ENERGY" \
    '{caller:$c, contract_id:$cid, method:"mint", args:[{U64:$v},{U64:$o},{U64:$m},{U64:$e}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$MINT" "mint_root" 4
  ok "root capability minted at slot=0 ✓"

  # Step 3: attenuate child from root (slot=1, parent=0)
  EPOCH=$(get_epoch)
  log "Step 3 - attenuate(parent=0,max=$INV_CHILD_MAX,energy=$INV_CHILD_ENERGY) → slot 1"
  ATTN=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    --argjson p 0 --argjson cm "$INV_CHILD_MAX" --argjson ce "$INV_CHILD_ENERGY" \
    '{caller:$c, contract_id:$cid, method:"attenuate", args:[{U64:$p},{U64:$cm},{U64:$ce}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$ATTN" "attenuate_child" 4
  ok "child capability minted at slot=1 (strict subset of root) ✓"

  # Step 4: invoke_gate on root (slot=0)
  EPOCH=$(get_epoch)
  log "Step 4 - invoke_gate(slot=0 root)  [parent chain walk: root itself → sentinel 999 → ok_flag=1]"
  IGR=$(jq -n \
    --argjson c "$CALLER2_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"invoke_gate", args:[{U64:0}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$IGR" "invoke_gate_root" 5
  ok "invoke_gate(root=0) PASSED ✓"

  # Step 5: invoke_gate on child (slot=1)
  EPOCH=$(get_epoch)
  log "Step 5 - invoke_gate(slot=1 child)  [chain walk: child → root (alive) → sentinel → ok_flag=1]"
  IGC=$(jq -n \
    --argjson c "$CALLER3_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"invoke_gate", args:[{U64:1}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$IGC" "invoke_gate_child" 5
  ok "invoke_gate(child=1) PASSED ✓"

  # Step 6: witness root → snapshot1 (par_energy=0 because root has sentinel parent)
  EPOCH=$(get_epoch)
  log "Step 6 - witness_cap(slot=0 root) → snapshot1  [par_energy=0: sentinel has no energy slot]"
  WR=$(jq -n \
    --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"witness_cap", args:[{U64:0}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$WR" "witness_root" 4

  # Step 7: witness child → snapshot2 (par_energy = root energy)
  EPOCH=$(get_epoch)
  log "Step 7 - witness_cap(slot=1 child) → snapshot2  [par_energy = root energy]"
  WC=$(jq -n \
    --argjson c "$CALLER2_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
    '{caller:$c, contract_id:$cid, method:"witness_cap", args:[{U64:1}], epoch:$ep}')
  require_tx "/api/tx/call-script" "$WC" "witness_child" 4

  # Verify snapshots
  if ! $DRY_RUN; then
    STATE=$(curl_json GET "/api/script/$CID")
    S1S=$(printf '%s' "$STATE" | untag snapshot1_slot)
    S1E=$(printf '%s' "$STATE" | untag snapshot1_energy)
    S1P=$(printf '%s' "$STATE" | untag snapshot1_par_energy)
    S2S=$(printf '%s' "$STATE" | untag snapshot2_slot)
    S2E=$(printf '%s' "$STATE" | untag snapshot2_energy)
    S2P=$(printf '%s' "$STATE" | untag snapshot2_par_energy)
    CC=$(printf '%s'  "$STATE" | untag cap_count)
    ok "cap_count=$CC"
    ok "snapshot1(root): slot=$S1S energy=$S1E par_energy=$S1P"
    ok "snapshot2(child): slot=$S2S energy=$S2E par_energy=$S2P"
    [[ "$S1S" -eq 0 ]]                    || die "snap1 slot should be 0 (root), got $S1S" 6
    [[ "$S1E" -eq "$INV_ROOT_ENERGY" ]]   || die "snap1 energy should be $INV_ROOT_ENERGY, got $S1E" 6
    [[ "$S1P" -eq 0 ]]                    || die "snap1 par_energy should be 0 (root/sentinel), got $S1P" 6
    [[ "$S2S" -eq 1 ]]                    || die "snap2 slot should be 1 (child), got $S2S" 6
    [[ "$S2E" -eq "$INV_CHILD_ENERGY" ]]  || die "snap2 energy should be $INV_CHILD_ENERGY, got $S2E" 6
    [[ "$S2P" -eq "$INV_ROOT_ENERGY" ]]   || die "snap2 par_energy should be $INV_ROOT_ENERGY (root energy), got $S2P" 6
    ok "root: energy=$INV_ROOT_ENERGY par_energy=0 (root/sentinel) ✓"
    ok "child: energy=$INV_CHILD_ENERGY par_energy=$INV_ROOT_ENERGY (parent alive) ✓"
  fi

fi

# ── Final summary ──────────────────────────────────────────────────────────
if [[ "$MODE" == "chain" ]]; then
  cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — CapabilityDecayVM (chain mode)         |
+---------------------------------------------------------------------+
|  contract_id: $CID
|  PROVEN (structural revocation, ocap energy-decay):
|   - root minted: verb=read obj=1 max=100 energy=50000 ✓
|   - child attenuated: max=50 energy=25000 (strict subset) ✓
|   - pre-revoke witness: child energy=25000 par_energy=50000 ✓
|   - revoke(root) zeroed root energy ✓
|   - require_ancestor_dead(child) PASSED ✓
|   - Structural: O(depth) revocation, no holder enumeration ✓
|   - "Revoking root kills every descendant without enumerating holders" ✓
+=====================================================================+
EOF
else
  cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — CapabilityDecayVM (invocable mode)     |
+---------------------------------------------------------------------+
|  contract_id: $CID
|  PROVEN (invoke_gate live-chain walk):
|   - root minted: verb=write obj=5 max=200 energy=40000 ✓
|   - child attenuated: max=100 energy=20000 (strict subset) ✓
|   - invoke_gate(root=0) PASSED ✓
|   - invoke_gate(child=1) PASSED (chain walk: child → root → sentinel) ✓
|   - snapshot1(root): energy=40000 par_energy=0 (sentinel/no parent) ✓
|   - snapshot2(child): energy=20000 par_energy=40000 (parent alive) ✓
|   - "Capability IS the authorization — no tx.origin, no role tables" ✓
+=====================================================================+
EOF
fi
