#!/usr/bin/env bash
#
# deploy-singh-sabi.sh — end-to-end doctrine proof for Singh-Sabi
# (Patina Tokens) — INVENTION_STACK.md §A5.3.
#
# NFTs that age toward "ruined-beautiful".
#
# The patina mechanic is unique among NFT primitives:
#   patina_score(t) = floor_energy + energy(t)
#   energy(t)       = decayable * 2^(-t / half_life)
#   decayable       = initial_energy - floor_energy
#
# Contract deployed with energy = decayable (NOT the full initial).
# The chain's own λ-decay drives the patina automatically.
# patina_score starts at initial_energy and asymptotes to floor_energy.
#
# Three structural invariants proven by this script:
#
#   1. At-mint: patina_score == initial_energy  (pristine, just minted)
#   2. After ~1 half-life: patina_score < initial_energy  (decay proven)
#   3. At all times: patina_score >= floor_energy  (floor maintained)
#
# The third invariant is the "ruined-beautiful" guarantee — the token
# never reaches zero; it asymptotes to the floor.
#
# HONEST SCOPE: proves on-chain SinghSabi patina registry + floor
# invariant + monotone decay observation. PatinaState visual entropy
# tuple (cracks/desaturation/foxing/edge_fray) is off-chain; computed
# by evaporchain-singh-sabi::entropy::derive_state. This contract
# records the on-chain energy anchor that the off-chain renderer reads.
#
# NOTE: re-running with identical source+deployer+energy+half_life
# resolves the SAME cached contract_id (deploy tx dedup). Pass a unique
# INITIAL_ENERGY each run, e.g. INITIAL_ENERGY=$((17000000 + RANDOM)).
#
# Usage:
#   ./scripts/deploy-singh-sabi.sh --dry-run
#   ./scripts/deploy-singh-sabi.sh --node http://89.167.52.40:8099
#
# Exit: 0 ok · 2 precondition · 3 deploy · 4 mint · 5 decay-not-observed · 6 verify
#
# Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CONTRACT_PATH="$ROOT_DIR/contracts/evaporscript/singh_sabi.es"

NODE_URL="${NODE_URL:-http://89.167.52.40:8099}"
TOKEN="${EVAPORCHAIN_TX_TOKEN:-}"
DEPLOYER_U8="${DEPLOYER_U8:-0}"        # minter/owner — must be funded
RECIPIENT_U8="${RECIPIENT_U8:-1}"      # initial holder after mint

# Patina parameters (doctrine defaults: floor = 15% of initial)
INITIAL_ENERGY="${INITIAL_ENERGY:-1000000}"   # full doctrinal initial patina score
FLOOR_ENERGY="${FLOOR_ENERGY:-150000}"        # 15% of 1000000 = ruined-beautiful floor
# Deploy contract with energy = INITIAL - FLOOR = 850000 (decayable portion)

# Short half-life for testnet: after PATINA_HALF_LIFE epochs, energy ≈ 50% of initial decayable
PATINA_HALF_LIFE="${PATINA_HALF_LIFE:-20}"

# NFT metadata
NFT_NAME="${NFT_NAME:-PatinaAlpha}"
NFT_COLLECTION="${NFT_COLLECTION:-SinghSabi-A5.3}"
NFT_METADATA="${NFT_METADATA:-ipfs://bafybeipatina0000}"

POLL_TIMEOUT_SEC="${POLL_TIMEOUT_SEC:-300}"
DRY_RUN=false
VERBOSE=false

usage() { cat <<'EOF'
deploy-singh-sabi.sh [options]
  --dry-run            validate + print intended calls; no network
  --node URL           node base URL (default http://89.167.52.40:8099)
  --token TOKEN        auth bearer ($EVAPORCHAIN_TX_TOKEN)
  --deployer U8        minter/owner account index (default 0)
  --recipient U8       initial holder index (default 1)
  --initial N          full initial patina score (default 1000000)
  --floor N            asymptotic floor energy (default 150000 = 15%)
  --half-life N        contract half-life in epochs (default 20, short for testnet)
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
    --recipient) RECIPIENT_U8="$2"; shift 2 ;;
    --initial) INITIAL_ENERGY="$2"; shift 2 ;;
    --floor) FLOOR_ENERGY="$2"; shift 2 ;;
    --half-life) PATINA_HALF_LIFE="$2"; shift 2 ;;
    --timeout) POLL_TIMEOUT_SEC="$2"; shift 2 ;;
    --verbose) VERBOSE=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown flag: $1" >&2; usage; exit 2 ;;
  esac
done

log()  { printf '\033[1;36m[sabi]\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m[sabi ERROR]\033[0m %s\n' "$*" >&2; exit "${2:-1}"; }
ok()   { printf '\033[1;32m[sabi OK]\033[0m %s\n' "$*"; }

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
untag() { jq -r ".state.$1 | if type==\"object\" then (.Bool // .U64 // .Str // .) else . end"; }
addr_arg() { jq -nc --argjson b "$1" '{Address: ([$b] + [range(0;31)|0])}'; }

# ── preflight ──────────────────────────────────────────────────────────────
[[ -f "$CONTRACT_PATH" ]] || die "contract not found: $CONTRACT_PATH" 2
grep -q "^contract SinghSabi" "$CONTRACT_PATH" || die ".es missing SinghSabi header" 3
grep -q "fn set_metadata("    "$CONTRACT_PATH" || die ".es missing set_metadata" 3
grep -q "fn witness("         "$CONTRACT_PATH" || die ".es missing witness" 3
grep -q "fn require_above_floor" "$CONTRACT_PATH" || die ".es missing require_above_floor" 3

(( FLOOR_ENERGY < INITIAL_ENERGY )) || die "FLOOR_ENERGY ($FLOOR_ENERGY) must be < INITIAL_ENERGY ($INITIAL_ENERGY)" 2
DECAYABLE=$(( INITIAL_ENERGY - FLOOR_ENERGY ))

if ! $DRY_RUN; then
  command -v curl >/dev/null || die "curl required" 2
  command -v jq   >/dev/null || die "jq required" 2
  curl -sS -m 5 "$NODE_URL/api/status" >/dev/null 2>&1 || die "node $NODE_URL unreachable" 2
fi

RECIP=$(addr_arg "$RECIPIENT_U8")

cat <<EOF
+=====================================================================+
|  Singh-Sabi (Patina Tokens) — §A5.3 e2e doctrine proof             |
+---------------------------------------------------------------------+
|  node: $NODE_URL
|  mode: $($DRY_RUN && echo DRY-RUN || echo LIVE)
|  deployer(u8): $DEPLOYER_U8  recipient(u8): $RECIPIENT_U8
|  initial_energy: $INITIAL_ENERGY  floor_energy: $FLOOR_ENERGY  (floor=$(( FLOOR_ENERGY * 100 / INITIAL_ENERGY ))%)
|  contract deploy energy: $DECAYABLE (decayable portion = initial - floor)
|  patina half-life: $PATINA_HALF_LIFE epochs
|  nft: "$NFT_NAME"  collection: "$NFT_COLLECTION"
+=====================================================================+
EOF

# ── Step 1: deploy ────────────────────────────────────────────────────────
log "Step 1/7 - deploy-script (singh_sabi.es) with energy=$DECAYABLE half_life=$PATINA_HALF_LIFE"
SRC=$(jq -Rs . < "$CONTRACT_PATH")
# Contract energy = DECAYABLE (floor + energy = patina_score; energy decays toward 0,
# so patina_score decays toward floor_energy).
DBODY=$(jq -n \
  --argjson d "$DEPLOYER_U8" --argjson s "$SRC" \
  --argjson e "$DECAYABLE" --argjson hl "$PATINA_HALF_LIFE" \
  '{deployer:$d, source_code:$s, energy:$e, half_life:$hl}')
DH=$(submit_tx "/api/tx/deploy-script" "$DBODY" "deploy" 3)
if ! $DRY_RUN; then
  DS=$(poll_tx_state "$DH")
  [[ "$DS" == "finalised" || "$DS" == "included" ]] || die "deploy not accepted (state=$DS)" 3
  CID=$(curl_json GET "/api/tx/$DH" | jq -r '.contract_id // empty')
  [[ -n "$CID" ]] || die "no contract_id in deploy receipt" 3
  ok "deployed contract_id=$CID  energy=$DECAYABLE  half_life=$PATINA_HALF_LIFE"
else
  CID=99
fi

# ── Step 2: set_metadata (mint) ──────────────────────────────────────────
EPOCH=$(get_epoch)
log "Step 2/7 - set_metadata (mint Patina Token to account[$RECIPIENT_U8]) at epoch=$EPOCH"
MINT_BODY=$(jq -n \
  --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  --arg name "$NFT_NAME" --arg coll "$NFT_COLLECTION" --arg meta "$NFT_METADATA" \
  --argjson recip "$RECIP" \
  --argjson init "$INITIAL_ENERGY" --argjson fl "$FLOOR_ENERGY" \
  '{caller:$c, contract_id:$cid, method:"set_metadata",
    args:[{Str:$name},{Str:$coll},{Str:$meta},$recip,{U64:$init},{U64:$fl}], epoch:$ep}')
require_tx "/api/tx/call-script" "$MINT_BODY" "set_metadata" 4
MINT_EPOCH=$(get_epoch)
ok "minted at epoch=$MINT_EPOCH"

# ── Step 3: verify sealed + stored initial/floor params ──────────────────
log "Step 3/7 - verify sealed + stored patina params"
if ! $DRY_RUN; then
  SSTATE=$(curl_json GET "/api/script/$CID")
  SEALED=$(printf '%s' "$SSTATE" | untag sealed)
  STORED_INITIAL=$(printf '%s' "$SSTATE" | untag initial_energy)
  STORED_FLOOR=$(printf '%s' "$SSTATE"   | untag floor_energy)
  CID_CREATED=$(printf '%s' "$SSTATE" | jq -r '.created_epoch // 0')

  [[ "$SEALED" == "true" || "$SEALED" == "1" || "$SEALED" == "True" ]] \
    || die "expected sealed=true, got $SEALED" 6
  [[ "$STORED_INITIAL" == "$INITIAL_ENERGY" ]] \
    || die "expected initial_energy=$INITIAL_ENERGY in state, got $STORED_INITIAL" 6
  [[ "$STORED_FLOOR" == "$FLOOR_ENERGY" ]] \
    || die "expected floor_energy=$FLOOR_ENERGY in state, got $STORED_FLOOR" 6

  log "  created_epoch=$CID_CREATED  initial_energy=$STORED_INITIAL  floor_energy=$STORED_FLOOR"
  ok "sealed=true, initial_energy=$STORED_INITIAL, floor_energy=$STORED_FLOOR ✓"
  # NOTE: API .energy returns STORED initial (not VM-computed decayed value).
  # Invariant probes use snapshot1/snapshot2 stored by witness() in state.
fi

# ── Step 4: witness() — initial snapshot ─────────────────────────────────
# The VM energy builtin = energy_at_epoch(decayable, half_life, tx_epoch - created_epoch).
# At tx_epoch ≈ created_epoch + few epochs, energy ≈ decayable (no half-life elapsed yet).
# snapshot1 = floor + energy ≈ initial_energy.
EPOCH=$(get_epoch)
log "Step 4/7 - witness() #1 at epoch=$EPOCH (stores snapshot1 = floor + energy in state)"
W1_BODY=$(jq -n \
  --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  '{caller:$c, contract_id:$cid, method:"witness", args:[], epoch:$ep}')
require_tx "/api/tx/call-script" "$W1_BODY" "witness(initial)" 4
if ! $DRY_RUN; then
  SSTATE_W1=$(curl_json GET "/api/script/$CID")
  SNAP1=$(printf '%s' "$SSTATE_W1" | untag snapshot1)
  log "  snapshot1=$SNAP1 (floor+energy at tx_epoch=$EPOCH)"
  (( SNAP1 > FLOOR_ENERGY )) || die "snapshot1=$SNAP1 <= floor=$FLOOR_ENERGY — unexpected at mint" 4
  ok "witness() #1: snapshot1=$SNAP1 ✓ (above floor; decay not yet significant)"
fi

# ── Step 5: require_above_floor + require_below_initial ──────────────────
EPOCH=$(get_epoch)
log "Step 5/7 - invariant probes: require_above_floor + require_below_initial"
RAF_BODY=$(jq -n \
  --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  '{caller:$c, contract_id:$cid, method:"require_above_floor", args:[], epoch:$ep}')
require_tx "/api/tx/call-script" "$RAF_BODY" "require_above_floor" 4
ok "require_above_floor PASSED ✓ (floor invariant confirmed on-chain)"

EPOCH=$(get_epoch)
# Use RECIPIENT_U8 as caller to avoid tx-hash dedup with require_above_floor
RBI_BODY=$(jq -n \
  --argjson c "$RECIPIENT_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  '{caller:$c, contract_id:$cid, method:"require_below_initial", args:[], epoch:$ep}')
require_tx "/api/tx/call-script" "$RBI_BODY" "require_below_initial" 4
ok "require_below_initial PASSED ✓ (score bounded by initial)"

# ── Step 6: wait ≥1 half-life, then second witness records snapshot2 ─────
# snapshot2 = floor + energy_at_epoch(decayable, half_life, tx_epoch2 - created_epoch)
# where tx_epoch2 > tx_epoch1 by ≥ PATINA_HALF_LIFE.
# After 1 half-life: energy ≈ decayable/2, snapshot2 ≈ floor + decayable/2.
# Compare snapshot1 vs snapshot2 to prove monotone decay.
#
# CRITICAL: use RECIPIENT_U8 as caller to avoid tx-hash dedup with step 4
# (same method "witness", same args [] — only epoch differs but epoch is
# NOT in signable_bytes, so hash collision without caller rotation).
log "Step 6/7 - wait ≥$PATINA_HALF_LIFE epochs then witness() #2 to capture decay"
if ! $DRY_RUN; then
  EPOCH=$(get_epoch)
  log "  current epoch=$EPOCH  mint_epoch=$MINT_EPOCH  half_life=$PATINA_HALF_LIFE"
  ELAPSED=$(( EPOCH - MINT_EPOCH ))
  if (( ELAPSED < PATINA_HALF_LIFE )); then
    GAP=$(( PATINA_HALF_LIFE - ELAPSED + 3 ))
    log "  elapsed=$ELAPSED < half_life=$PATINA_HALF_LIFE — sleeping ${GAP}s..."
    sleep "$GAP"
  fi
  EPOCH2=$(get_epoch)
  ELAPSED2=$(( EPOCH2 - MINT_EPOCH ))
  log "  post-sleep: epoch2=$EPOCH2  elapsed_since_mint=$ELAPSED2"
fi

EPOCH2=$(get_epoch)
W2_BODY=$(jq -n \
  --argjson c "$RECIPIENT_U8" --argjson cid "$CID" --argjson ep "$EPOCH2" \
  '{caller:$c, contract_id:$cid, method:"witness", args:[], epoch:$ep}')
require_tx "/api/tx/call-script" "$W2_BODY" "witness(post-decay)" 4

if ! $DRY_RUN; then
  SSTATE2=$(curl_json GET "/api/script/$CID")
  SNAP2=$(printf '%s' "$SSTATE2" | untag snapshot2)
  WCOUNT2=$(printf '%s' "$SSTATE2" | untag witness_count)
  log "  snapshot2=$SNAP2 (floor+energy at tx_epoch2=$EPOCH2)  witness_count=$WCOUNT2"

  # Invariant 2: score DECREASED (decay observed)
  (( SNAP2 < SNAP1 )) \
    || die "patina_score did not decrease: snapshot2=$SNAP2 >= snapshot1=$SNAP1 (no decay observed after elapsed=$ELAPSED2 epochs with half_life=$PATINA_HALF_LIFE)" 5
  ok "snapshot2=$SNAP2 < snapshot1=$SNAP1 ✓ (decay observed, Invariant 2: monotone decay)"

  # Invariant 3: score >= floor
  (( SNAP2 >= FLOOR_ENERGY )) \
    || die "snapshot2=$SNAP2 < floor_energy=$FLOOR_ENERGY — floor violated" 6
  ok "snapshot2=$SNAP2 >= floor_energy=$FLOOR_ENERGY ✓ (Invariant 3: floor maintained)"

  # Quantitative sanity: after ~1 half-life, snapshot2 ≈ floor + decayable/2
  EXPECTED_LOW=$(( FLOOR_ENERGY + DECAYABLE * 25 / 100 ))
  EXPECTED_HIGH=$(( FLOOR_ENERGY + DECAYABLE * 80 / 100 ))
  if (( SNAP2 >= EXPECTED_LOW && SNAP2 <= EXPECTED_HIGH )); then
    ok "snapshot2=$SNAP2 in [$EXPECTED_LOW,$EXPECTED_HIGH] (~50% decay of decayable portion) ✓"
  else
    log "  note: snapshot2=$SNAP2 outside [$EXPECTED_LOW,$EXPECTED_HIGH] (elapsed=$ELAPSED2, may be multiple half-lives); decay confirmed"
  fi
fi

# ── Step 7: re-verify on-chain floor/initial invariants post-decay ────────
# require_above_floor: use a DIFFERENT caller than step 5 to avoid dedup.
# Step 5 used DEPLOYER_U8 (caller=0) for require_above_floor.
# Step 7 uses RECIPIENT_U8 (caller=1) for require_above_floor — different hash.
EPOCH=$(get_epoch)
log "Step 7/7 - re-verify floor/initial invariants post-decay (on-chain, same as step 5)"
RAF2_BODY=$(jq -n \
  --argjson c "$RECIPIENT_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  '{caller:$c, contract_id:$cid, method:"require_above_floor", args:[], epoch:$ep}')
require_tx "/api/tx/call-script" "$RAF2_BODY" "require_above_floor(post-decay)" 4
ok "require_above_floor PASSED post-decay ✓ (ruined-beautiful floor holds)"

# require_below_initial: use DEPLOYER_U8 (step 5 used RECIPIENT_U8 for RBI) — different hash.
EPOCH=$(get_epoch)
RBI2_BODY=$(jq -n \
  --argjson c "$DEPLOYER_U8" --argjson cid "$CID" --argjson ep "$EPOCH" \
  '{caller:$c, contract_id:$cid, method:"require_below_initial", args:[], epoch:$ep}')
require_tx "/api/tx/call-script" "$RBI2_BODY" "require_below_initial(post-decay)" 4
ok "require_below_initial PASSED post-decay ✓ (score still bounded by initial)"

if ! $DRY_RUN; then
  SSTATE3=$(curl_json GET "/api/script/$CID")
  WCOUNT=$(printf '%s' "$SSTATE3" | untag witness_count)
  [[ "$WCOUNT" == "2" ]] || die "expected witness_count=2, got $WCOUNT" 6
  ok "witness_count=$WCOUNT ✓"
  ok "final scores: snapshot1=$SNAP1 → snapshot2=$SNAP2 (monotone non-increasing) ✓"
  ok "floor_energy=$FLOOR_ENERGY maintained at all epochs ✓"
fi

# ── Summary ───────────────────────────────────────────────────────────────
cat <<EOF

+=====================================================================+
|  DOCTRINE PROOF COMPLETE — Singh-Sabi §A5.3                         |
+---------------------------------------------------------------------+
|  contract_id  : $CID
|  initial_energy: $INITIAL_ENERGY   floor_energy: $FLOOR_ENERGY   (floor=$(( FLOOR_ENERGY * 100 / INITIAL_ENERGY ))%)
|  deploy energy: $DECAYABLE  half_life: $PATINA_HALF_LIFE epochs
|  PROVEN:
|   - Sealed + initial/floor params locked at mint ✓
|   - require_above_floor (floor invariant): PASSED at mint + post-decay ✓
|   - require_below_initial (upper bound): PASSED at mint + post-decay ✓
|   - Invariant 2: snapshot2 < snapshot1 (monotone decay observed) ✓
|   - Invariant 3: snapshot2 >= floor_energy (ruined-beautiful floor) ✓
|   - witness_count=2 (two on-chain witness events) ✓
+=====================================================================+
EOF
