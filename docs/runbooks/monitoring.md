# Cluster Monitoring — Operator Runbook

**Lane T1.21** — Cluster monitoring (Prometheus + Grafana + alerts). The chain side ships a working `/metrics` endpoint at `evaporchain-node`; the paymaster ships its own. This runbook is the scrape + dashboard + alert side: how to consume those endpoints from Prometheus / Grafana / Alertmanager.

Pairs with: `MAINNET_READINESS.md` T1.21, `docs/runbooks/cluster-deploy.md` (cluster bring-up).

---

## What's already exposed

### Chain side (`evaporchain-node`)

`GET /metrics` on the same port as the JSON API (default `:8081`). Auth-gated behind `EVAPORCHAIN_ADMIN_KEY` (`require_admin_auth` middleware at `crates/evaporchain-node/src/api.rs:1397`). Returns Prometheus text exposition (no `Content-Type` header negotiation needed; `text/plain` works).

Metric surface (25 series, names below). All are `gauge` or `counter`; no histograms in V1.

```
# Chain liveness
evaporchain_block_height          gauge    — Current block height
evaporchain_epoch                 gauge    — Current epoch
evap_block_height                 gauge    — Alias of evaporchain_block_height (dashboard-compat)
evap_epoch                        gauge    — Alias of evaporchain_epoch

# Throughput
evaporchain_tps                   gauge    — Current TPS (rolling)
evaporchain_peak_tps              gauge    — Peak TPS observed
evaporchain_total_transactions    counter  — Cumulative tx count
evaporchain_avg_block_exec_ms     gauge    — Mean block-execution time (ms)
evaporchain_avg_gas_per_block     gauge    — Mean gas-used per block

# State
evaporchain_active_objects        gauge    — Live state objects
evaporchain_ghost_count           gauge    — Evaporated ghost records
evaporchain_accounts              gauge    — Total accounts

# Network
evaporchain_peer_count            gauge    — Currently-connected libp2p peers

# Node
evaporchain_uptime_seconds        counter  — Node uptime in seconds

# Finality (consensus)
evap_finalized_height                     gauge    — Highest BLS-finalised height
evap_finality_gap_seconds                 gauge    — Per-height commit→finalise duration
evap_unfinalised_height_count             gauge    — Heights committed but not finalised
evap_worst_unfinalised_gap_seconds        gauge    — Drives EvapFinalityStalled alert

# Validator set
evap_active_validators            gauge    — Un-jailed validators
evap_validator_set_size           gauge    — Active + jailed
evap_consensus_round              gauge    — Current round inside the active height

# Doctrine / fee market
evaporchain_fee_base_ppm          gauge    — Singh-Lyapunov base fee in ppm
evaporchain_epv_live_versions     gauge    — Live EPV protocol versions
evaporchain_autopoietic_viability gauge    — 2=Viable 1=Stressed 0=Inviable
evaporchain_consensus_phase       gauge    — 3=LivenessStable 2=SafetyStable 1=Frozen 0=Chaotic
```

### Paymaster side (`evaporchain-paymaster`)

`GET /metrics` on the paymaster's own port. **No auth on the paymaster's `/metrics`** — the paymaster runs as a separate process and exposes its own surface; gate via firewall/network.

Series prefix: `evaporchain_paymaster_*`. Source of truth: `crates/evaporchain-paymaster/src/lib.rs::prometheus_metrics` (line 1292).

```
evaporchain_paymaster_sponsorships_total{status="ok|already_signed|invalid_user_sig|rate_limited|nonce_io|audit_io|other"}  counter
evaporchain_paymaster_sponsorships_idempotent_replay_total                                                                  counter
evaporchain_paymaster_next_nonce                                                                                            gauge
evaporchain_paymaster_active_senders                                                                                        gauge
evaporchain_paymaster_uptime_seconds                                                                                        counter
```

---

## Setting `EVAPORCHAIN_ADMIN_KEY` on each node

The chain's `/metrics` returns `403` if `EVAPORCHAIN_ADMIN_KEY` is unset. To scrape, set a strong random key on every node and pass it as a bearer token from Prometheus.

```bash
# On each node, before starting the node binary:
EVAPORCHAIN_ADMIN_KEY="$(openssl rand -hex 32)" \
  ~/EvaporChain/target/release/evaporchain-node --api --api-port 8081 …
```

Keep the key out of shell history (`set +o history` first, or use a `.env` file with mode `0600`).

To verify the endpoint accepts the key:

```bash
curl -fsS -H "Authorization: Bearer $EVAPORCHAIN_ADMIN_KEY" http://<node-ip>:8081/metrics | head
```

If the response is `{"error":"admin endpoints disabled: ..."}`, the env var isn't reaching the process; check the systemd unit / wrapper.

---

## Prometheus scrape config

Drop-in `prometheus.yml` snippet for the 5-node testnet-1 cluster:

```yaml
# Chain nodes — `/metrics` is admin-auth-gated.
- job_name: 'evaporchain-chain'
  scrape_interval: 15s
  scheme: http
  bearer_token_file: /etc/prometheus/evaporchain-admin-key  # mode 0600
  static_configs:
    - targets:
        - '100.119.53.101:8081'   # satyawan  (Mini 1)
        - '100.113.253.72:8081'   # apsarth   (Mini 2)
        - '100.103.216.125:8081'  # ironman   (Mini 3)
        - '100.66.208.20:8081'    # hel-1
        - '100.91.235.22:8081'    # hel-2
      labels:
        cluster: testnet-1

# Paymaster — `/metrics` is unauthenticated; gate via network.
- job_name: 'evaporchain-paymaster'
  scrape_interval: 30s
  static_configs:
    - targets:
        - '<paymaster-host>:<port>'
      labels:
        cluster: testnet-1
```

`bearer_token_file` is preferred over inline `bearer_token` so the key isn't in the Prometheus config repo. Create it on the Prometheus host:

```bash
sudo install -m 600 -o prometheus -g prometheus /dev/stdin /etc/prometheus/evaporchain-admin-key <<< "$EVAPORCHAIN_ADMIN_KEY"
```

The same `EVAPORCHAIN_ADMIN_KEY` value must be set on every chain node AND on the Prometheus host's bearer-token file.

---

## Baseline Grafana dashboard

Drop `scripts/grafana-dashboards/evaporchain-chain.json` into your Grafana provisioning directory (`/etc/grafana/provisioning/dashboards/`) or import via the UI. Panels:

| Panel | Query |
|---|---|
| Block height per node | `evaporchain_block_height` (legend = instance) |
| Cluster height spread | `max(evaporchain_block_height) - min(evaporchain_block_height)` |
| Finalisation lag | `evap_worst_unfinalised_gap_seconds` |
| Peer count per node | `evaporchain_peer_count` |
| Active vs ghost objects | `evaporchain_active_objects`, `evaporchain_ghost_count` |
| TPS (current vs peak) | `evaporchain_tps`, `evaporchain_peak_tps` |
| Avg block exec (ms) | `evaporchain_avg_block_exec_ms` |
| Consensus phase | `evaporchain_consensus_phase` (singlestat, value-map 3→Liveness 2→Safety 1→Frozen 0→Chaotic) |
| Autopoietic viability | `evaporchain_autopoietic_viability` |
| Validator set size vs active | `evap_validator_set_size`, `evap_active_validators` |

---

## Alert rules — starter set

`evaporchain.rules.yml` for Prometheus / Alertmanager:

```yaml
groups:
  - name: evaporchain-cluster
    rules:
      # Height has not advanced for 60s → consensus stalled or node frozen.
      - alert: EvaporChainHeightStalled
        expr: |
          (
            max_over_time(evaporchain_block_height[60s])
            - min_over_time(evaporchain_block_height[60s])
          ) == 0
        for: 60s
        labels:
          severity: critical
        annotations:
          summary: "Chain height on {{$labels.instance}} not advancing"
          description: "Block height frozen for ≥60s. Check consensus + peer connectivity."

      # Per-node peer count drops to <2 → near-isolation.
      - alert: EvaporChainPeerLossNearIsolation
        expr: evaporchain_peer_count < 2
        for: 120s
        labels:
          severity: warning
        annotations:
          summary: "Node {{$labels.instance}} has <2 peers"
          description: "Sustained low peer count; check banlist + gossipsub topics."

      # Cluster height spread >5 blocks → fork or one node lagging.
      - alert: EvaporChainClusterHeightSpread
        expr: max(evaporchain_block_height) - min(evaporchain_block_height) > 5
        for: 60s
        labels:
          severity: warning
        annotations:
          summary: "Cluster height spread >5 blocks"
          description: "One or more nodes lagging the leader. Possible fork or slow sync."

      # Finality gap exceeds 30s → BLS aggregation stalled.
      - alert: EvapFinalityStalled
        expr: evap_worst_unfinalised_gap_seconds > 30
        for: 30s
        labels:
          severity: critical
        annotations:
          summary: "Finality stalled on {{$labels.instance}}"
          description: "Heights committed but not BLS-finalised for >30s."

      # Autopoietic viability dropped below Viable.
      - alert: EvaporChainViabilityDegraded
        expr: evaporchain_autopoietic_viability < 2
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "Chain viability degraded (= {{$value}})"
          description: "Singh-Lyapunov autopoietic viability is Stressed (1) or Inviable (0)."

      # Consensus phase != LivenessStable for >10m.
      - alert: EvaporChainConsensusPhaseDrift
        expr: evaporchain_consensus_phase < 3
        for: 10m
        labels:
          severity: warning
        annotations:
          summary: "Consensus phase regression on {{$labels.instance}}"
          description: "RG phase map dropped from LivenessStable (3); current value {{$value}}."
```

Thresholds are starter values; tune against the post-T3.1 testnet-1 trace before mainnet.

---

## Cross-references

- `MAINNET_READINESS.md` T1.21
- `crates/evaporchain-node/src/api.rs:15830` — `get_prometheus_metrics` source of truth
- `crates/evaporchain-paymaster/src/lib.rs:1292` — paymaster's `prometheus_metrics()`
- `docs/runbooks/cluster-deploy.md` — cluster bring-up
- `scripts/grafana-dashboards/evaporchain-chain.json` — paired dashboard (this PR)
- `scripts/prometheus-scrape-config.example.yml` — paired scrape config (this PR)
