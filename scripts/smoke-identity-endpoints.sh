#!/usr/bin/env bash
# Smoke-test the identity dashboard endpoints against a running node.
# Curls every endpoint added in the launch-sprint, reports OK / FAIL
# per route, exits non-zero if any required endpoint fails.
#
# Usage:
#   scripts/smoke-identity-endpoints.sh                      # defaults to http://localhost:8080
#   scripts/smoke-identity-endpoints.sh https://node.example # override
#
# Required tools: curl, jq (optional — falls back to grep on absence)

set -u

NODE="${1:-http://localhost:8080}"
PASS=0
FAIL=0
SKIP=0
declare -a FAILED

bold() { printf '\033[1m%s\033[0m' "$1"; }
green() { printf '\033[32m%s\033[0m' "$1"; }
red() { printf '\033[31m%s\033[0m' "$1"; }
yellow() { printf '\033[33m%s\033[0m' "$1"; }

check() {
  local label="$1"
  local method="$2"
  local path="$3"
  local body="${4:-}"
  local expect="${5:-200}"

  local status
  if [ "$method" = "GET" ]; then
    status=$(curl -s -o /tmp/smoke.body -w '%{http_code}' --max-time 8 "${NODE}${path}" 2>/dev/null)
  else
    status=$(curl -s -o /tmp/smoke.body -w '%{http_code}' --max-time 8 \
      -X "$method" -H 'content-type: application/json' \
      -d "$body" "${NODE}${path}" 2>/dev/null)
  fi

  if [ "$status" = "$expect" ]; then
    printf '  %s  %-6s %-40s [%s]\n' "$(green '✓')" "$method" "$path" "$status"
    PASS=$((PASS+1))
  else
    printf '  %s  %-6s %-40s [%s]\n' "$(red '✗')" "$method" "$path" "$status"
    FAIL=$((FAIL+1))
    FAILED+=("$method $path → $status")
  fi
}

echo
bold "Smoke-testing EvaporChain identity endpoints against $NODE"
echo
echo

# ── Reach test ────────────────────────────────────────────────────────
echo "  $(bold "Liveness")"
ROOT_STATUS=$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 "${NODE}/api/identity" 2>/dev/null || echo "000")
if [ "$ROOT_STATUS" = "000" ]; then
  echo "  $(red '✗')  cannot reach $NODE — is the node running?"
  echo
  exit 2
fi

# ── Identity + spine + observability (GET) ────────────────────────────
echo
echo "  $(bold "Identity / spine / observability")"
check "identity"               GET  /api/identity
check "four_act"               GET  /api/four_act
check "light_cone"             GET  /api/light_cone
check "lambda_fold"            GET  /api/lambda_fold
check "lamport_time"           GET  /api/lamport_time
check "tur_liveness"           GET  /api/tur_liveness
check "mortis_cert_preview"    GET  /api/mortis_cert_preview
check "refresh_pool"           GET  /api/refresh_pool
check "sentinel/all"           GET  /api/sentinel/all

# ── Substrate query primitives (POST) ─────────────────────────────────
echo
echo "  $(bold "Substrate primitives")"
check "cmu_check"              GET  '/api/cmu_check?cmu_mb=300&excess_entropy_mb=100&entropy_rate_mb=200'
check "beacon"                 GET  /api/beacon/0
check "padic"                  POST /api/padic                  '{"x":12,"y":20}'
check "tropical_weight"        POST /api/tropical_weight        '{"energy":1000}'
check "eb_fs_challenge"        POST /api/eb_fs_challenge        '{"transcript_hex":"deadbeef","epoch":1,"epoch_energy":1000}'
check "singh_attractor"        POST /api/singh_attractor        '{"state_energy":50,"attractors":[{"center":50,"basin_radius":10}]}'
check "bell_beacon"            POST /api/bell_beacon            '{"e_ab":500,"e_ab_prime":-500,"e_a_prime_b":500,"e_a_prime_b_prime":500}'
check "allen_relation"         POST /api/allen_relation         '{"a":{"start":0,"end":10},"b":{"start":5,"end":15}}'
check "mdl_optimal"            POST /api/mdl_optimal            '{"items":[1,2,3,1,2,3],"max_shards":2}'
check "cslc_reconstruct"       POST /api/cslc_reconstruct       '{"counts":[10,20,30]}'
check "efh/h0"                 POST /api/efh/h0                 '{"energies":[1,5,3,8,2]}'
check "efh/bottleneck"         POST /api/efh/bottleneck         '{"diagram_a":[[1,5]],"diagram_b":[[1,5]]}'
check "prp/prove"              POST /api/prp/prove              '{"state_id_hex":"00000000000000000000000000000000000000000000000000000000000000aa","committed_energy":1000,"activated_epoch":0,"floor":10}'
check "crooks_refund"          POST /api/crooks_refund          '{"p_forward_ppm":800000,"p_reverse_ppm":400000,"work_extracted":1000,"beta_mb":10}'
check "causal_cone"            GET  '/api/causal_cone?head_hex=0000000000000000000000000000000000000000000000000000000000000000' 200
check "mcc_fork_choice"        GET  '/api/mcc_fork_choice?candidates=0000000000000000000000000000000000000000000000000000000000000000'

# ── HBCT lifecycle ────────────────────────────────────────────────────
echo
echo "  $(bold "HBCT launch wedge")"
check "hbct/state"             GET  /api/hbct/state
check "hbct/seed_demo"         POST /api/hbct/seed_demo         '{}'
check "hbct/state (post-seed)" GET  /api/hbct/state
check "hbct/tick"              POST /api/hbct/tick              '{"current_epoch":481253}'

# ── Sentinel demo loop ────────────────────────────────────────────────
echo
echo "  $(bold "Sentinel autonomic governance")"
check "sentinel/seed_demo"     POST /api/sentinel/seed_demo     '{}'
check "sentinel/seed_votes"    POST /api/sentinel/seed_votes    '{"current_epoch":1}'
check "sentinel/all"           GET  /api/sentinel/all

# ── Lambda-Fold verifier ──────────────────────────────────────────────
echo
echo "  $(bold "Lambda-Fold verifier")"
# Verify accepts identity hash + 0 energy when chain is at fold-identity.
check "lambda_fold/verify"     POST /api/lambda_fold/verify     '{"expected_acc_hash_hex":"0000000000000000000000000000000000000000000000000000000000000000","expected_remaining_energy":0}'

# ── Demo reset (exercise idempotency) ─────────────────────────────────
echo
echo "  $(bold "Demo reset")"
check "demo/reset"             POST /api/demo/reset             '{}'

# ── Summary ───────────────────────────────────────────────────────────
echo
TOTAL=$((PASS+FAIL+SKIP))
echo "  $(bold "Result"): $(green "$PASS pass") · $(red "$FAIL fail") · $TOTAL total"
if [ "$FAIL" -gt 0 ]; then
  echo
  echo "  $(red 'Failures:')"
  for f in "${FAILED[@]}"; do
    echo "    - $f"
  done
  exit 1
fi
echo
exit 0
