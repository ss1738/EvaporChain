# EvaporChain Observer Stack

Prometheus + Grafana + Alertmanager for the 3-Mini testnet (`satyawan.local`,
`apsarth.local`, `ironman.local`). Run on any one Mini — it scrapes the
others over Tailscale / `.local` mDNS.

## Launch

```bash
cd deploy/observer
GRAFANA_ADMIN_PASSWORD=$(openssl rand -hex 16) docker compose up -d
```

Surfaces (substitute the host running the stack):

- Prometheus: <http://HOST:9090>
- Grafana: <http://HOST:3000> — login `admin` / value of `GRAFANA_ADMIN_PASSWORD`
- Alertmanager: <http://HOST:9093>

The Prometheus datasource and the **EvaporChain — Operator Overview** dashboard
are auto-provisioned. The dashboard lives in the `EvaporChain` Grafana folder.

## Stop / restart / wipe

```bash
docker compose stop                  # graceful
docker compose down                  # remove containers
docker compose down -v               # also wipe TSDB + Grafana state
docker compose pull && docker compose up -d   # update images
```

## Updating dashboards

Edit `deploy/grafana/dashboards/evaporchain-overview.json`. Grafana re-reads
the file every 30s (see `provisioning/dashboards/dashboards.yml`). To add a
second dashboard, drop another `*.json` next to it.

## Adding new metrics

1. Emit the metric from `crates/evaporchain-node/src/api.rs` in
   `get_prometheus_metrics`. Use the `evap_*` prefix, write `# HELP` + `# TYPE`
   lines, then the sample.
2. Reference it from the dashboard or alert YAML.
3. `curl http://NODE:8080/metrics | grep evap_<name>` to confirm.

## Alert routing

Rules live under `deploy/prometheus/alerts/*.yml`. They fire into Alertmanager,
which currently routes everything to a Slack webhook placeholder in
`deploy/prometheus/alertmanager.yml`. Replace `https://hooks.slack.com/REPLACE_ME`
with the real channel webhook before public testnet launch.

`amtool check-config deploy/prometheus/alertmanager.yml` validates the file.
`promtool check rules deploy/prometheus/alerts/*.yml` validates rules.

## Hard-coded scrape targets

`prometheus.yml` lists the three Mini hostnames directly — no service
discovery. If the cluster grows, switch to file-based or DNS SD instead of
maintaining the static list.
