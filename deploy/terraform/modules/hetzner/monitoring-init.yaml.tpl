#cloud-config
# EvaporChain monitoring node bootstrap (Prometheus + Grafana)

write_files:
  - path: /etc/prometheus/prometheus.yml
    content: |
      global:
        scrape_interval: 15s
        evaluation_interval: 15s

      scrape_configs:
        - job_name: 'evaporchain'
          static_configs:
            - targets:
              %{ for ip in validator_ips ~}
              - '${ip}:${metrics_port}'
              %{ endfor ~}
          relabel_configs:
            - source_labels: [__address__]
              target_label: instance
    permissions: "0644"

  - path: /etc/grafana/provisioning/datasources/prometheus.yaml
    content: |
      apiVersion: 1
      datasources:
        - name: Prometheus
          type: prometheus
          access: proxy
          url: http://localhost:9090
          isDefault: true
    permissions: "0644"

  - path: /etc/grafana/provisioning/dashboards/evaporchain.yaml
    content: |
      apiVersion: 1
      providers:
        - name: EvaporChain
          orgId: 1
          folder: EvaporChain
          type: file
          options:
            path: /var/lib/grafana/dashboards
    permissions: "0644"

  - path: /etc/systemd/system/prometheus.service
    content: |
      [Unit]
      Description=Prometheus
      After=network.target

      [Service]
      User=prometheus
      ExecStart=/usr/local/bin/prometheus \
        --config.file=/etc/prometheus/prometheus.yml \
        --storage.tsdb.path=/data/prometheus \
        --storage.tsdb.retention.time=30d \
        --web.listen-address=0.0.0.0:9090
      Restart=on-failure

      [Install]
      WantedBy=multi-user.target
    permissions: "0644"

runcmd:
  - mkdir -p /data/prometheus /var/lib/grafana/dashboards /etc/prometheus /etc/grafana/provisioning/datasources /etc/grafana/provisioning/dashboards

  # Install Prometheus
  - |
    PROM_VER=2.51.2
    curl -fsSL "https://github.com/prometheus/prometheus/releases/download/v$PROM_VER/prometheus-$PROM_VER.linux-amd64.tar.gz" \
      -o /tmp/prometheus.tar.gz
    tar -xzf /tmp/prometheus.tar.gz -C /tmp
    install -m 0755 /tmp/prometheus-$PROM_VER.linux-amd64/prometheus /usr/local/bin/prometheus
    useradd --system --shell /bin/false prometheus || true
    chown -R prometheus:prometheus /data/prometheus

  # Install Grafana
  - |
    apt-get install -y apt-transport-https software-properties-common
    wget -q -O /usr/share/keyrings/grafana.key https://apt.grafana.com/gpg.key
    echo "deb [signed-by=/usr/share/keyrings/grafana.key] https://apt.grafana.com stable main" \
      > /etc/apt/sources.list.d/grafana.list
    apt-get update -q
    apt-get install -y grafana
    grafana-cli admin reset-admin-password "${grafana_password}"

  # Copy EvaporChain dashboard
  - |
    cat > /var/lib/grafana/dashboards/evaporchain.json << 'DASHEOF'
    {}
    DASHEOF

  # Start services
  - systemctl daemon-reload
  - systemctl enable prometheus grafana-server
  - systemctl start prometheus grafana-server

  # Firewall: Prometheus + Grafana only accessible from within network
  - ufw --force enable
  - ufw allow 22/tcp
  - ufw allow from 10.0.0.0/16 to any port 9090
  - ufw allow 3000/tcp

package_upgrade: true
packages:
  - curl
  - ufw
  - wget
