#cloud-config
# EvaporChain validator node bootstrap — rendered by Terraform templatefile()

write_files:
  - path: /etc/evaporchain/genesis.json
    encoding: b64
    content: ${genesis_b64}
    permissions: "0640"

  - path: /etc/evaporchain/node-key
    encoding: b64
    content: ${node_key_b64 != "" ? node_key_b64 : base64encode("placeholder-generate-at-runtime")}
    permissions: "0600"

  - path: /etc/systemd/system/evaporchain.service
    content: |
      [Unit]
      Description=EvaporChain Validator Node ${validator_index}
      After=network-online.target
      Wants=network-online.target

      [Service]
      User=evaporchain
      Group=evaporchain
      ExecStart=/usr/local/bin/evaporchain-node \
        --config /etc/evaporchain/genesis.json \
        --node-key /etc/evaporchain/node-key \
        --data-dir /data/evaporchain \
        --listen-addr 0.0.0.0:${p2p_port} \
        --rpc-addr 0.0.0.0:${rpc_port} \
        --metrics-addr 0.0.0.0:9090 \
        --bootstrap-peers ${bootstrap_peers} \
        --validator-index ${validator_index} \
        --log-format json
      Restart=on-failure
      RestartSec=5
      LimitNOFILE=65536
      StandardOutput=journal
      StandardError=journal
      SyslogIdentifier=evaporchain

      [Install]
      WantedBy=multi-user.target
    permissions: "0644"

  - path: /etc/logrotate.d/evaporchain
    content: |
      /var/log/evaporchain/*.log {
        daily
        rotate 7
        compress
        missingok
        notifempty
        postrotate
          systemctl kill -s HUP evaporchain.service
        endscript
      }
    permissions: "0644"

runcmd:
  # Wait for the data volume to be formatted and mounted (/dev/sdb → /data)
  - mkdir -p /data/evaporchain
  - |
    if ! mountpoint -q /data; then
      if [ -b /dev/disk/by-label/HC_Volume_${validator_index} ]; then
        mount /dev/disk/by-label/HC_Volume_${validator_index} /data
      fi
    fi

  # Create service user
  - useradd --system --shell /bin/false --home-dir /data/evaporchain evaporchain || true
  - chown -R evaporchain:evaporchain /data/evaporchain /etc/evaporchain

  # Download binary
  - |
    curl -fsSL "${binary_url}" -o /tmp/evaporchain-node.tar.gz
    tar -xzf /tmp/evaporchain-node.tar.gz -C /tmp
    install -m 0755 /tmp/evaporchain-node /usr/local/bin/evaporchain-node

  # Enable and start
  - systemctl daemon-reload
  - systemctl enable evaporchain.service
  - systemctl start evaporchain.service

  # ufw firewall
  - ufw --force enable
  - ufw allow 22/tcp
  - ufw allow ${p2p_port}/tcp
  - ufw allow ${rpc_port}/tcp
  - ufw allow from 10.0.0.0/16 to any port 9090

package_upgrade: true
packages:
  - curl
  - ufw
  - jq
  - htop
