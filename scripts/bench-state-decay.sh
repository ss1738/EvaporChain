#!/usr/bin/env bash
#
# bench-state-decay.sh — the reproducible state-decay benchmark (v2).
#
# CLAIM UNDER TEST: EvaporChain's energy-decay primitive makes deployed
# on-chain contract objects expire BY CONSTRUCTION — no rent, no
# restore tx, no operator action — whereas the SAME workload without
# decay persists monotonically (the model every other L1 is stuck
# with). See research/BENCH_STATE_DECAY.md.
#
# CORRECT INSTRUMENT (v1 used /api/status.active_objects — WRONG: that
# counts the ~4 genesis demo objects, NOT deployed script contracts;
# and data_dir_bytes is restart-noisy). v2 measures the primitive
# DIRECTLY: deploy N copies of bench_object.es, poll each to a
# terminal tx state and capture its contract_id, then over time count
# how many of THOSE SPECIFIC cids are still live (GET /api/script/:id
# not evaporated). decay → live_count → ~0; nodecay → live_count ≈ N.
# Unfooled by genesis objects or restart noise.
#
# RIGOR: every deploy is polled to finalised/included before counting
# (no fire-and-forget); token auto-re-minted on auth expiry (sessions
# are in-memory, wiped on node restart); deploys are spaced (avoids
# the documented rapid-fire account-0 contention).
#
# HONEST SCOPE: single-node --mock-consensus sandbox; SMALL bounded N
# (mechanical-correctness verification of the primitive + instrument).
# This is NOT the at-scale adversarial state-spam test — that needs
# infra headroom this 2-vCPU sandbox structurally lacks (repeatedly
# demonstrated); at-scale stays honestly gated, not faked. Exit:
# 0 ok · 2 precondition · 3 workload-could-not-land.

set -euo pipefail

NODE_URL="${NODE_URL:-http://89.167.52.40:8099}"
TOKEN="${EVAPORCHAIN_TX_TOKEN:-}"
DEPLOYER_U8="${DEPLOYER_U8:-0}"
REGIME="${REGIME:-both}"            # decay | nodecay | both
COUNT="${COUNT:-12}"               # deploys per regime (SMALL, bounded)
DECAY_ENERGY="${DECAY_ENERGY:-200}"
DECAY_HALF_LIFE="${DECAY_HALF_LIFE:-3}"
NODECAY_ENERGY="${NODECAY_ENERGY:-9000000000}"
NODECAY_HALF_LIFE="${NODECAY_HALF_LIFE:-9000000000}"
DEPLOY_SPACING_S="${DEPLOY_SPACING_S:-2}"   # anti-contention spacing
SETTLE_S="${SETTLE_S:-2}"
SAMPLE_ROUNDS="${SAMPLE_ROUNDS:-8}"         # liveness samples after deploys
SAMPLE_GAP_S="${SAMPLE_GAP_S:-6}"
OUT="${OUT:-/tmp/bench-state-decay.csv}"

log() { printf '\033[1;36m[bench]\033[0m %s\n' "$*"; }
die() { printf '\033[1;31m[bench ERROR]\033[0m %s\n' "$*" >&2; exit "${2:-1}"; }

command -v curl >/dev/null || die "curl required" 2
command -v jq   >/dev/null || die "jq required" 2
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ES="$SCRIPT_DIR/../contracts/evaporscript/bench_object.es"
[[ -f "$ES" ]] || die "bench_object.es not found at $ES" 2
curl -sS -m5 "$NODE_URL/api/status" >/dev/null 2>&1 || die "node $NODE_URL unreachable" 2
SRC=$(jq -Rs . < "$ES")

mint_token() {
  local u="bench-$(date +%s%N|cut -c1-15)@example.com"
  curl -s -m12 -X POST -H 'Content-Type: application/json' \
    -d "{\"email\":\"$u\",\"password\":\"benchpass123\",\"display_name\":\"bench\"}" \
    "$NODE_URL/api/auth/register" >/dev/null 2>&1 || true
  curl -s -m12 -X POST -H 'Content-Type: application/json' \
    -d "{\"email\":\"$u\",\"password\":\"benchpass123\"}" \
    "$NODE_URL/api/auth/login" | jq -r '.token // empty'
}
[[ -n "$TOKEN" ]] || TOKEN=$(mint_token)
[[ -n "$TOKEN" ]] || die "could not obtain a session token" 2

# Deploy one bench object, poll to terminal, return contract_id.
# Re-mints token once on auth expiry (node restart wipes sessions).
deploy_polled() {  # $1=energy $2=half_life -> prints cid or empty
  local e="$1" hl="$2" body resp th st cid tries
  body=$(jq -n --argjson d "$DEPLOYER_U8" --argjson s "$SRC" \
        --argjson e "$e" --argjson hl "$hl" \
        '{deployer:$d,source_code:$s,energy:$e,half_life:$hl}')
  for tries in 1 2; do
    resp=$(curl -s -m20 -X POST -H 'Content-Type: application/json' \
           -H "Authorization: Bearer $TOKEN" -d "$body" \
           "$NODE_URL/api/tx/deploy-script")
    th=$(printf '%s' "$resp" | jq -r '.tx_hash // empty')
    if [[ -z "$th" ]]; then
      if printf '%s' "$resp" | grep -qi 'auth'; then
        TOKEN=$(mint_token); continue   # session wiped → re-mint, retry once
      fi
      return 0   # rejected for another reason — caller counts as not-landed
    fi
    break
  done
  [[ -n "${th:-}" ]] || return 0
  local deadline=$(( $(date +%s) + 60 ))
  while (( $(date +%s) < deadline )); do
    st=$(curl -s -m8 -H "Authorization: Bearer $TOKEN" "$NODE_URL/api/tx/$th" \
         | jq -r '.state // "unknown"')
    case "$st" in
      finalised|included)
        cid=$(curl -s -m8 -H "Authorization: Bearer $TOKEN" "$NODE_URL/api/tx/$th" \
              | jq -r '.contract_id // empty')
        [[ -n "$cid" ]] && { printf '%s' "$cid"; return 0; } ;;
      rejected) return 0 ;;
    esac
    sleep 2
  done
}

# Count how many of the given cids are still live (state present, not evaporated).
live_count() {  # $@=cids -> "live evaporated"
  local cid live=0 gone=0 j
  for cid in "$@"; do
    j=$(curl -s -m8 "$NODE_URL/api/script/$cid" 2>/dev/null)
    if printf '%s' "$j" | jq -e '.state and (.evaporated != true)' >/dev/null 2>&1; then
      live=$((live+1))
    else
      gone=$((gone+1))
    fi
  done
  printf '%d %d' "$live" "$gone"
}

run_regime() {  # $1=name $2=energy $3=half_life
  local name="$1" e="$2" hl="$3" i cid; local -a cids=()
  log "regime=$name energy=$e half_life=$hl — deploying $COUNT (polled, spaced)"
  for ((i=1;i<=COUNT;i++)); do
    cid=$(deploy_polled "$e" "$hl")
    [[ -n "$cid" ]] && cids+=("$cid")
    sleep "$DEPLOY_SPACING_S"
  done
  local landed=${#cids[@]}
  log "  $name: $landed/$COUNT deploys landed (cids: ${cids[*]:0:6}$([[ $landed -gt 6 ]] && echo …))"
  (( landed > 0 )) || { log "  $name: ZERO landed — workload could not be created (node/contention)"; echo "$name,WORKLOAD_FAILED,0,0" >> "$OUT"; return 3; }
  sleep "$SETTLE_S"
  local r lc
  for ((r=0;r<SAMPLE_ROUNDS;r++)); do
    lc=$(live_count "${cids[@]}")
    echo "$name,$r,$landed,$lc" | tr ' ' ',' >> "$OUT"
    log "  $name sample $r: live/gone = $lc  (of $landed)"
    sleep "$SAMPLE_GAP_S"
  done
}

echo "regime,sample,landed,live,gone" > "$OUT"
cat <<EOF
+==================================================================+
|  EvaporChain state-decay benchmark v2 (correct instrument)        |
|  node=$NODE_URL  count=$COUNT (small/bounded)                      |
|  decay e=$DECAY_ENERGY hl=$DECAY_HALF_LIFE | nodecay e=$NODECAY_ENERGY hl=$NODECAY_HALF_LIFE
|  instrument: per-deployed-cid liveness (NOT status.active_objects) |
|  HONEST: single-node mock-consensus; small-N mechanical verify;    |
|  at-scale needs infra the sandbox lacks (gated, not faked).        |
+==================================================================+
EOF
rc=0
[[ "$REGIME" == decay   || "$REGIME" == both ]] && { run_regime decay   "$DECAY_ENERGY"   "$DECAY_HALF_LIFE"   || rc=$?; }
[[ "$REGIME" == nodecay || "$REGIME" == both ]] && { run_regime nodecay "$NODECAY_ENERGY" "$NODECAY_HALF_LIFE" || rc=$?; }

log "CSV: $OUT"
python3 - "$OUT" <<'PY' || true
import csv,sys
rows=[r for r in csv.DictReader(open(sys.argv[1]))]
print("\n=== VERDICT ===")
for rg in ('decay','nodecay'):
    s=[r for r in rows if r['regime']==rg and r['sample'] not in ('WORKLOAD_FAILED',)]
    if not s:
        f=[r for r in rows if r['regime']==rg]
        print(f"{rg:8s}: WORKLOAD_FAILED (0 deploys landed — node/contention, NOT a primitive result)" if f else f"{rg:8s}: not run"); continue
    first,last=s[0],s[-1]
    print(f"{rg:8s}: landed={last['landed']}  live {first['live']}→{last['live']}  gone {first['gone']}→{last['gone']}")
print("\nEXPECTED: decay live→~0 (objects evaporate by physics);")
print("          nodecay live≈landed (persist). If decay live stays ≈landed → primitive FALSIFIED.")
PY
exit $rc
