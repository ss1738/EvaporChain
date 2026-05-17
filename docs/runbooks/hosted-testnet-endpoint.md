# Runbook — Hosted Testnet Endpoint (enterprise-pilot sandbox)

**Purpose.** Stand up one stable, publicly-reachable EvaporChain
**testnet sandbox** so external parties (enterprise pilots, the GDPR
pilot kit, SDK users) integrate via a hosted API instead of running a
node. This is the #1 enterprise-adoption requirement (enterprises
integrate against an endpoint; they do not run chains). It directly
unblocks `sdk/examples/gdpr-erasure-pilot` end-to-end.

**Scope.** This is a *single-node mock-consensus testnet sandbox* — the
correct target for third-party pilots. It is **not** the mainnet/soak
cluster (`evaporchain-tailscale-5node-1`, T3.1) and must never be
confused with it. Operator executes; this runbook is preparation.

**Status of inputs.** Every node flag / API path / auth fact below was
**verified live this session** (the SFSV / Dead Drop / EvaporCash /
GDPR e2es). Operator-decision points are called out explicitly and are
**not** pre-decided here.

---

## 0. What "done" looks like

- `https://<your-domain>/api/status` returns chain JSON over TLS from
  the public internet.
- `register → login → bearer` works; `deploy-script` / `call-script` /
  `GET /api/script/:id` work for an external client.
- The node restarts on crash/reboot and its data survives.
- `cd sdk/examples/gdpr-erasure-pilot && node pilot.mjs --node
  https://<your-domain>` completes exit 0.

---

## 1. Operator decisions (decide BEFORE deploying — not pre-decided)

| Decision | Options / notes |
|---|---|
| **Host** | A box you control with a public IP. The existing FINGAURD VPS pattern (a small cloud VM) is the model; any 2 vCPU / 4 GB / 40 GB Linux VM is ample for a mock-consensus single node. **Not** a Mini (tailnet-only, and Mini-1 is ~98% disk). |
| **DNS + TLS** | A subdomain you own (e.g. `testnet.<domain>`), terminated by a reverse proxy (Caddy = automatic Lets-Encrypt, or nginx + certbot). The node serves plain HTTP on `--api-port`; the proxy adds TLS. |
| **Funded-account model for pilots** | Genesis funds **only** `addr_from_byte(0)` (the all-zeros faucet acct, balance `u64::MAX/2`). Pilots deploy as `--deployer 0`. The faucet endpoint is **admin-gated** (`EVAPORCHAIN_ADMIN_KEY`, fail-closed). Decision: (a) pilots share `deployer 0` (simplest for a sandbox), or (b) set `EVAPORCHAIN_ADMIN_KEY` and fund per-pilot accounts. (a) is recommended for a throwaway sandbox. |
| **Reset cadence** | A sandbox should be wipe-and-redeploy on a schedule (e.g. weekly) — decide and document; pilots must not assume persistence of *their* contracts beyond it. |
| **Mock vs real prove** | Testnet sandbox = `--mock-prove` (fast, no Nova). Do **not** use `--prove` here (that is mainnet-class). |

---

## 2. Build the node (Minis only — never the VPS, never a Mac laptop)

Per project rule, `cargo build` runs on the M4 Minis. Build a release
binary and copy it to the host:

```bash
# on a Mini (ssh per ~/mac-mini-cluster-access.md)
cd ~/EvaporChain && git pull --ff-only origin main
cargo build --release -p evaporchain-node     # produces target/release/evaporchain-node
# copy the binary to the host (scp from a machine that can reach both;
# the Mini→host hop may need a relay — base64-over-ssh is the proven
# fallback if scp silently fails, per evaporchain_mini_file_transfer).
```

The host runs the prebuilt binary only — no toolchain on the VPS.

---

## 3. Run as a durable service (systemd)

`/etc/systemd/system/evaporchain-testnet.service`:

```ini
[Unit]
Description=EvaporChain testnet sandbox node
After=network-online.target

[Service]
User=evapor
# Persistent data dir (NOT /tmp — survives restarts within a reset cycle)
Environment=RUST_LOG=info
# If decision 1.faucet=(b): Environment=EVAPORCHAIN_ADMIN_KEY=<secret>
ExecStart=/opt/evaporchain/evaporchain-node \
  --api --api-port 8099 \
  --mock-prove --mock-consensus --no-da-enforcement \
  --data-dir /var/lib/evaporchain/testnet \
  --block-interval-ms 1000
Restart=always
RestartSec=5
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
```

```bash
sudo mkdir -p /var/lib/evaporchain/testnet /opt/evaporchain
sudo cp evaporchain-node /opt/evaporchain/ && sudo chown -R evapor /var/lib/evaporchain
sudo systemctl daemon-reload && sudo systemctl enable --now evaporchain-testnet
journalctl -u evaporchain-testnet -f      # watch genesis + first blocks
```

Flag rationale (all verified this session): `--api --api-port` serves
the HTTP API; `--mock-prove --mock-consensus` = single-node testnet
(no Nova, no BFT quorum needed); `--no-da-enforcement` avoids DA-stall
on a single node; `--data-dir` persistent (the smoke node used `/tmp`
which loses state on reboot — do not do that here);
`--block-interval-ms 1000` is a calm sandbox cadence (the smoke node
used 400; 1000 lowers load for a long-lived box).

---

## 4. TLS reverse proxy (Caddy example — simplest)

`/etc/caddy/Caddyfile`:

```
testnet.<your-domain> {
    reverse_proxy 127.0.0.1:8099
}
```

```bash
sudo systemctl reload caddy     # auto-provisions Lets-Encrypt cert
```

(nginx + certbot is equivalent if preferred; the node itself does no
TLS.)

---

## 5. Smoke-verify (the acceptance test)

```bash
curl -sS https://testnet.<your-domain>/api/status | jq '{chain_name,block_height,epoch}'
# register/login → bearer → a real script round-trip:
cd sdk && npm ci && npm run build
cd examples/gdpr-erasure-pilot
node pilot.mjs --node https://testnet.<your-domain> \
  --record "pilot smoke" --retention-energy 60000 --half-life 5 --mode retain
# expect: exit 0 — deploy → seal → terminal evaporation → crypto-shred proof
```

If the pilot completes exit 0, the endpoint is enterprise-pilot-ready
and the previously node-blocked GDPR e2e is closed.

---

## 6. Honest caveats (do not let a pilot misread these)

- **Mock-consensus, single node.** Fine for integration pilots; it is
  **not** a security/decentralisation demonstration. Say so to pilots.
- **No byte-erasure.** Per the verified Dead Drop §9 finding, the chain
  proves *terminal evaporation / tamper-evident lifecycle*, not
  byte-deletion. The GDPR value is crypto-shred + provable trigger
  (see `research/GDPR_ERASURE_ARCHITECTURE.md`) — pilots must be
  briefed with that honest scope, not "the chain erases data."
- **Sandbox, not SLA.** Resets on the documented cadence; no uptime
  guarantee. State this in any pilot agreement.
- **Security.** Public HTTP API: keep `EVAPORCHAIN_ADMIN_KEY` secret
  (or unset → admin endpoints fail-closed); rate-limit at the proxy;
  this box holds no production keys and is not the mainnet cluster.

---

## 7. Decommission / reset

```bash
sudo systemctl stop evaporchain-testnet
sudo rm -rf /var/lib/evaporchain/testnet/*       # wipe sandbox state
sudo systemctl start evaporchain-testnet         # fresh genesis
```

Announce resets to active pilots in advance.
