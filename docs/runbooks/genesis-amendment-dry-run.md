# Genesis-Amendment Dry-Run Runbook (T1.23)

Validates the full LLSA upgrade path end-to-end on the live testnet cluster:
`evaporchain-llsa::apply_amendment` → EPV registry binding → `MultiAuditorVerifier` k-of-n.

**Scope:** testnet-1 only (3 Minis + 2 Hetzners). Uses `AlwaysAcceptVerifier` (the
substrate descope path). Production mainnet amendments require a real Coq kernel certificate.

**Acceptance:** `POST /api/llsa/apply_amendment` returns `{"status":"ok"}` on every node;
`GET /api/epv/status` shows the new version registered on every node; audit log entry written.

---

## Pre-flight

### 1. Confirm cluster is healthy

```bash
# Run from MacBook — check all 5 nodes
for NODE in \
    satyawansingh@100.119.53.101 \
    satyawan-mini-1@100.113.253.72 \
    satyawan-mini-2@100.103.216.125; do
    echo "=== $NODE ==="
    ssh -o IdentitiesOnly=yes -i ~/.ssh/id_ed25519 "$NODE" \
        "curl -s http://localhost:8080/api/health | python3 -m json.tool"
done
# Repeat for the two Hetzner nodes (ports as deployed).
```

All nodes must return `"status":"ok"` and `"is_synced":true`.

### 2. Record current EPV registry state

```bash
# On Mini 1 (representative node):
ssh -o IdentitiesOnly=yes -i ~/.ssh/id_ed25519 satyawansingh@100.119.53.101 \
    "curl -s http://localhost:8080/api/epv/status | python3 -m json.tool" \
    | tee /tmp/epv-before.json
```

Note the highest `id` in the versions list — call it `CURRENT_VERSION`. The dry-run
amendment will be `from_version: CURRENT_VERSION` → `to_version: CURRENT_VERSION + 1`.

Typical genesis state: `CURRENT_VERSION = 1` (seed registered at genesis).

### 3. Choose amendment parameters

| Parameter | Dry-run value | Notes |
|---|---|---|
| `from_version` | `CURRENT_VERSION` | Must be registered |
| `to_version` | `CURRENT_VERSION + 1` | Must NOT be registered |
| `step_new_descriptor_hex` | `"6472797275 6e2d762032"` | Hex of `"dryrun-v2"`; arbitrary for testnet |
| `to_version_seed_energy` | `1000000000` | 1B units — ~2× genesis seed |
| `activation_epoch` | current epoch | From `/api/epv/status` → `current_epoch` |
| `expected_invariant_hex` | `"0000...0000"` (64 zeroes) | Testnet invariant sentinel; production uses real Blake3 hash of Coq goal |

---

## Execution

### Step 1 — Compute amendment hash (optional verification)

The amendment hash is `blake3("evaporchain-llsa-amendment" || from_version_le8 || to_version_le8 || len_le8 || descriptor_bytes)`.

For the testnet descriptor `"dryrun-v2"` (9 bytes, hex `6472797275 6e2d763200`), confirm
determinism with the unit test:

```bash
ssh -o IdentitiesOnly=yes -i ~/.ssh/id_ed25519 satyawansingh@100.119.53.101 \
    "cd ~/EvaporChain && cargo test -p evaporchain-llsa -- amendment_hash_is_deterministic --nocapture 2>&1 | tail -5"
```

Expected: `test amendment_hash_is_deterministic ... ok`

### Step 2 — POST amendment to node 1

### Admin authentication

`/api/llsa/apply_amendment` is admin-gated. Set `EVAPORCHAIN_ADMIN_KEY` in the node's
launch env and pass it as `Authorization: Bearer <key>` on every admin request.
Default is fail-closed (no key set → endpoint returns "admin endpoints disabled").

Recommended: generate a strong random key and store it out-of-band:

```bash
export EVAPORCHAIN_ADMIN_KEY=$(python3 -c "import secrets; print(secrets.token_hex(32))")
# Persist out-of-band (1Password, KMS, etc.) — needed for every admin request.
```

```bash
MINI1=http://localhost:8080    # adjust the port (e.g. 8081 on the colo cluster)

curl -s -X POST "$MINI1/api/llsa/apply_amendment" \
     -H "Content-Type: application/json" \
     -H "Authorization: Bearer $EVAPORCHAIN_ADMIN_KEY" \
     -d '{
       "from_version": 1,
       "to_version":   2,
       "step_new_descriptor_hex": "647279 72756e2d7632",
       "to_version_seed_energy": 1000000000,
       "activation_epoch": 0,
       "expected_invariant_hex": "0000000000000000000000000000000000000000000000000000000000000000"
     }' | python3 -m json.tool
```

**Expected response:**

```json
{
    "status": "ok",
    "from_version": 1,
    "to_version": 2,
    "seed_energy": 1000000000,
    "total_versions": 2
}
```

**Error paths to exercise (run these first, then the success case):**

```bash
# a. from_version not registered → expect "from_version 99 is not currently registered"
curl -s -X POST "$MINI1/api/llsa/apply_amendment" \
     -H "Content-Type: application/json" \
     -d '{"from_version":99,"to_version":100,"step_new_descriptor_hex":"aa","to_version_seed_energy":1000000000,"activation_epoch":0,"expected_invariant_hex":"0000000000000000000000000000000000000000000000000000000000000000"}' \
     | python3 -m json.tool

# b. to_version already registered (run this AFTER the successful amendment above) →
#    "to_version 2 is already registered (collision)"
curl -s -X POST "$MINI1/api/llsa/apply_amendment" \
     -H "Content-Type: application/json" \
     -d '{"from_version":1,"to_version":2,"step_new_descriptor_hex":"bb","to_version_seed_energy":1000000000,"activation_epoch":0,"expected_invariant_hex":"0000000000000000000000000000000000000000000000000000000000000000"}' \
     | python3 -m json.tool
```

### Step 3 — Verify EPV registry updated

```bash
ssh -o IdentitiesOnly=yes -i ~/.ssh/id_ed25519 satyawansingh@100.119.53.101 \
    "curl -s http://localhost:8080/api/epv/status | python3 -m json.tool" \
    | tee /tmp/epv-after.json
```

Check:
- `total_versions` increased by 1.
- New entry with `id: 2` appears in `versions[]`.
- `is_runnable: true` on the new version (it was just seeded; energy has not decayed).
- Old version `id: 1` is still `is_runnable: true`.

### Step 4 — Confirm old version not evicted

The amendment does NOT immediately evict `v1` — only EPV energy decay + prune does that.
Verify `v1` still passes `is_runnable`:

```bash
curl -s http://localhost:8080/api/epv/status | python3 -c "
import sys, json
d = json.load(sys.stdin)
for v in d['versions']:
    print(f'v{v[\"id\"]}: runnable={v[\"is_runnable\"]}, energy={v[\"remaining_energy\"]}')
"
```

Both `v1` and `v2` must show `runnable=True` at epoch 0.

### Step 5 — MultiAuditorVerifier k-of-n unit check

The production path for mainnet uses `MultiAuditorVerifier` with real auditor-signature
verifiers. The unit tests pin both the threshold-met and threshold-missed cases. Run
them to confirm the logic is intact on the cluster build:

```bash
ssh -o IdentitiesOnly=yes -i ~/.ssh/id_ed25519 satyawansingh@100.119.53.101 \
    "cd ~/EvaporChain && cargo test -p evaporchain-llsa 2>&1 | tail -10"
```

Expected: all `evaporchain-llsa` tests pass (`multi_auditor_*`, `amendment_*`, `happy_path_*`).

### Step 6 — Propagation check (optional, best-effort)

The EPV registry is per-node in the substrate substrate. Production mainnet will replicate
amendments via governance-tx broadcast. For the testnet dry-run, repeat Step 2 on each
of the other 4 nodes individually (each node starts from its own genesis registry).

If node API ports differ, substitute accordingly. The test validates the code path; registry
sync across nodes is a governance-layer concern for mainnet, not a substrate invariant.

---

## Rollback

There is no rollback for a registered EPV version — this is by design (EPV invariant:
registered versions can only be *evicted by energy decay*, not manually removed).

For testnet: restart the node without persistent state (`--reset-db`) to get a fresh
registry. This is safe because testnet state is disposable.

---

## Post-run report

Append the following to `SESSION_PROGRESS.md` after completing the dry-run:

```
## T1.23 — Genesis-amendment dry-run result

- Date: YYYY-MM-DD
- Cluster: evaporchain-tailscale-5node-1 (3 Minis + 2 Hetzners)
- Amendment: v1 → v2, descriptor=dryrun-v2, seed_energy=1_000_000_000
- Node tested: Mini 1 (http://localhost:8080)
- Pre-amendment EPV: { versions: [v1], total: 1 }
- Post-amendment EPV: { versions: [v1, v2], total: 2 }
- Error-path tests: from_version_absent ✅ / to_version_collision ✅
- LLSA unit tests: N passed / 0 failed
- Acceptance: PASS / FAIL
```

---

## Reference

| API | Method | Purpose |
|---|---|---|
| `/api/epv/status` | GET | List all registered versions + runnable status |
| `/api/epv/register` | POST | Direct version registration (bypasses proof gate) |
| `/api/epv/prune` | POST | Prune evaporated versions |
| `/api/llsa/apply_amendment` | POST | Apply proven amendment (substrate: AlwaysAcceptVerifier) |

Source: `crates/evaporchain-llsa/` — `amendment.rs`, `apply.rs`, `proof.rs`
Node handler: `crates/evaporchain-node/src/api.rs:5493` (`post_llsa_apply_amendment`)
EPV handler: `crates/evaporchain-node/src/api.rs:7052` (`get_epv_status`)

Mainnet: replace `AlwaysAcceptVerifier` with a `MultiAuditorVerifier` wrapping
real auditor-signature `ProofVerifier` impls. The API handler at `:5493` will need
to be extended to accept `proof_bytes` from the request body and route through the
production verifier. That is LLSA Layer 7 (post-audit, not in the current sprint).
