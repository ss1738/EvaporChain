# Production Env-Var Checklist

One-page pre-launch checklist of every environment variable an operator
must set BEFORE binding `evaporchain-node` to a public port. Unset env
vars typically mean **unauthenticated access** to the corresponding
endpoint surface; the chain prints loud stderr warnings at startup
when this happens but does not refuse to start.

Cross-references AUDIT_2026_05_06.md "live security gaps" closures
(CRITICAL-2, CRITICAL-3) — each env var below corresponds to an
audit-flagged enforcement point.

---

## 1. Required env vars (production)

### 1.1 `EVAPORCHAIN_ADMIN_KEY`

**Gates:** `/api/admin/drain`, `/api/admin/undrain`, `/api/admin/drain/status`,
`/metrics`, `/api/proof_replay`, `/api/network/ban`, `/api/network/unban`,
`/api/network/banned`, plus other admin-tagged endpoints.

**Failure mode if unset:** node prints a red startup warning, admin
endpoints return `503 Service Unavailable` with `"error": "admin
endpoints disabled: EVAPORCHAIN_ADMIN_KEY not configured"`. **Not
unauthenticated — fail-closed.** Operators starting without this env
var cannot drain the node, query metrics, or run proof-replay.

**Strong value:**
```bash
EVAPORCHAIN_ADMIN_KEY=$(openssl rand -hex 32)
```

**Smoke check (from a different shell):**
```bash
# unauth attempt → 401
curl -s -X POST http://<node>:8081/api/admin/drain
# expected: {"error":"unauthorized: invalid admin key"}

# auth attempt → 200
curl -s -X POST http://<node>:8081/api/admin/drain \
  -H "Authorization: Bearer $EVAPORCHAIN_ADMIN_KEY"
```

---

### 1.2 `EVAPORCHAIN_ORACLE_KEY`

**Gates:** `POST /api/oracle/ingest` — sensor/oracle data point
ingestion as a CreateObject transaction.

**Failure mode if unset:** oracle endpoint returns 401. **Fail-closed.**

**Strong value:**
```bash
EVAPORCHAIN_ORACLE_KEY=$(openssl rand -hex 32)
```

**Smoke check:**
```bash
curl -s -X POST http://<node>:8081/api/oracle/ingest \
  -H "Authorization: Bearer $EVAPORCHAIN_ORACLE_KEY" \
  -H "Content-Type: application/json" \
  -d '{"source":"smoke-test","object_id":"smoke-1","energy":100,"half_life":100,"data":"{}"}'
```

---

### 1.3 `EVAPORCHAIN_MCP_API_TOKEN` (NEW — closes AUDIT CRITICAL-2 second half, 2026-05-11)

**Gates:** MCP-channel POST routes — `/api/tx/*`, `/api/faucet`,
`/api/contracts/*`, `/api/fork_cert/prove`, `/api/mera/commit`. These
are the endpoints the MCP server (`evaporchain-mcp`) forwards AI-agent
requests to.

**Failure mode if unset:** **pass-through — endpoints are
UNAUTHENTICATED.** This is intentional for local dev / non-MCP
deployments. For production with MCP exposed to agents, this is the
single switch that closes the AI-prompt-injection attack surface.

**Coupling:** the SAME token must be set on both the node AND the MCP
process. The MCP attaches it as `Authorization: Bearer <token>` to
every outgoing request; the node verifies it with constant-time
compare via `subtle::ConstantTimeEq`.

**Strong value:**
```bash
EVAPORCHAIN_MCP_API_TOKEN=$(openssl rand -hex 32)
# write to a file referenced by BOTH the node systemd unit AND the
# mcp systemd unit:
echo "$EVAPORCHAIN_MCP_API_TOKEN" > /etc/evaporchain/mcp.token
chmod 600 /etc/evaporchain/mcp.token
```

**Banner check at node startup:**
```
evaporchain-node: MCP channel auth ENFORCED — POST /api/tx/*, /api/faucet,
                  /api/contracts/*, /api/fork_cert/prove, /api/mera/commit
                  require Authorization: Bearer EVAPORCHAIN_MCP_API_TOKEN.
```

If the banner is absent at startup, the env var was unset/empty — fix
before continuing.

**Smoke check:**
```bash
# without header → 401
curl -s -X POST http://<node>:8081/api/faucet -d '{}'
# {"error":"MCP channel auth required", "remedy":"Authorization: Bearer EVAPORCHAIN_MCP_API_TOKEN", ...}

# with header → bubbles through to the endpoint's own handling
curl -s -X POST http://<node>:8081/api/faucet \
  -H "Authorization: Bearer $EVAPORCHAIN_MCP_API_TOKEN" \
  -H "Content-Type: application/json" -d '{...}'
```

---

### 1.4 `EVAPORCHAIN_VALIDATOR_KEY_PASS_FILE`

**Gates:** Argon2id-encrypted at-rest BLS validator key
(`bls_key.bin` in the data dir). The passphrase decrypts the key on
startup.

**Failure mode if unset:** the node falls back to legacy in-process
keygen (re-generates the BLS key on every restart). **In a
multi-validator cluster this is a disaster** — your validator
identity changes between restarts, drops out of the active set, and
gets re-elected as a fresh entry.

**Strong value:**
```bash
echo "your-strong-passphrase" > /etc/evaporchain/bls.pass
chmod 600 /etc/evaporchain/bls.pass
export EVAPORCHAIN_VALIDATOR_KEY_PASS_FILE=/etc/evaporchain/bls.pass
```

(Reading from a file rather than the env var directly avoids
`/proc/<pid>/environ` exposure — see `validator-passphrase-migration.md`
runbook for the historical context.)

---

## 2. Optional env vars (TLS / observability)

### 2.1 `EVAPORCHAIN_TLS_CERT` + `EVAPORCHAIN_TLS_KEY`

**Gates:** HTTPS instead of plain HTTP on the API port.

**Failure mode if unset:** node binds plaintext HTTP — suitable only
for localhost or behind a TLS-terminating reverse proxy (nginx,
caddy). **NOT for direct internet exposure.**

**Set values:** PEM file paths.

---

### 2.2 `EVAPORCHAIN_MCP_REQUIRE_AUTH`

**Gates:** MCP server's startup decision. When set to `"true"` and
`EVAPORCHAIN_MCP_API_TOKEN` is unset, MCP refuses to start (rather
than silently running in dev mode).

**Recommended:** set this to `"true"` on production MCP processes so
a missing token at deploy time is a loud startup failure rather than
a silent token-less MCP that the node would silently accept.

---

## 3. Single-command pre-flight verifier

```bash
#!/usr/bin/env bash
# Save as scripts/verify-prod-env.sh and run on each node before
# `systemctl start evaporchain-validator`.
set -e
for v in \
  EVAPORCHAIN_ADMIN_KEY \
  EVAPORCHAIN_ORACLE_KEY \
  EVAPORCHAIN_MCP_API_TOKEN \
  EVAPORCHAIN_VALIDATOR_KEY_PASS_FILE; do
  if [ -z "${!v:-}" ]; then
    echo "✗ MISSING: $v"
    exit 1
  fi
  echo "✓ set: $v"
done
echo "All required env vars present."
```

---

## 4. Common mistakes

- **Setting the env var in the operator shell but not in the systemd
  unit.** `systemctl start` does not inherit your shell env. Use the
  unit's `Environment=` or `EnvironmentFile=` directive.
- **Setting `EVAPORCHAIN_MCP_API_TOKEN` on the node but not on the MCP
  server (or vice versa).** Both sides must agree; check by tailing
  MCP stderr for `Bearer-token (enforced)` AND node stderr for the
  `MCP channel auth ENFORCED` banner.
- **Re-using the same token across env vars.** Each gate is
  independent; if one leaks the other should still hold. Use 4
  different random values.
- **Committing tokens to the repo.** `.gitignore` covers
  `.env`-style files; never put a real token in a Cargo.toml or
  CI workflow file.

---

## 5. Cross-references

- AUDIT_2026_05_06.md CRITICAL-2 (MCP attack surface)
- AUDIT_2026_05_06.md CRITICAL-3 (admin endpoint auth-bypass)
- `docs/runbooks/validator-passphrase-migration.md` — at-rest key encryption
- `docs/runbooks/wasm-crypto-csp.md` — wallet extension key handling
- `crates/evaporchain-node/src/api.rs::require_admin_auth` — admin auth implementation
- `crates/evaporchain-node/src/api.rs::mcp_channel_auth_middleware` — MCP gate (PR #35)
