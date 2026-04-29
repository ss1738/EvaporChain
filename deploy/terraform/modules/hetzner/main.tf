terraform {
  required_providers {
    hcloud = {
      source  = "hetznercloud/hcloud"
      version = "~> 1.47"
    }
  }
  required_version = ">= 1.6"
}

provider "hcloud" {
  token = var.hcloud_token
}

# ── Network ───────────────────────────────────────────────────────────────────

resource "hcloud_network" "evaporchain" {
  name     = "${var.cluster_name}-net"
  ip_range = "10.0.0.0/16"
  labels   = local.common_labels
}

resource "hcloud_network_subnet" "validators" {
  network_id   = hcloud_network.evaporchain.id
  type         = "cloud"
  network_zone = var.network_zone
  ip_range     = "10.0.1.0/24"
}

# ── SSH Key ───────────────────────────────────────────────────────────────────

resource "hcloud_ssh_key" "operator" {
  name       = "${var.cluster_name}-operator"
  public_key = var.operator_ssh_public_key
}

# ── Firewall ──────────────────────────────────────────────────────────────────

resource "hcloud_firewall" "validator" {
  name   = "${var.cluster_name}-validator"
  labels = local.common_labels

  rule {
    direction  = "in"
    protocol   = "tcp"
    port       = "22"
    source_ips = var.admin_cidr_blocks
    description = "SSH from operator IPs"
  }

  rule {
    direction  = "in"
    protocol   = "tcp"
    port       = tostring(var.p2p_port)
    source_ips = ["0.0.0.0/0", "::/0"]
    description = "EvaporChain P2P"
  }

  rule {
    direction  = "in"
    protocol   = "tcp"
    port       = tostring(var.rpc_port)
    source_ips = var.rpc_allowed_cidrs
    description = "EvaporChain RPC/HTTP API"
  }

  rule {
    direction  = "in"
    protocol   = "tcp"
    port       = "9090"
    source_ips = var.metrics_allowed_cidrs
    description = "Prometheus metrics"
  }

  rule {
    direction  = "out"
    protocol   = "tcp"
    port       = "any"
    destination_ips = ["0.0.0.0/0", "::/0"]
  }

  rule {
    direction  = "out"
    protocol   = "udp"
    port       = "any"
    destination_ips = ["0.0.0.0/0", "::/0"]
  }
}

# ── Placement Group (spread validators across physical hosts) ─────────────────

resource "hcloud_placement_group" "validators" {
  name   = "${var.cluster_name}-spread"
  type   = "spread"
  labels = local.common_labels
}

# ── Volumes (persistent chain data) ──────────────────────────────────────────

resource "hcloud_volume" "validator" {
  count    = var.validator_count
  name     = "${var.cluster_name}-validator-${count.index}-data"
  size     = var.data_volume_gb
  location = element(var.locations, count.index % length(var.locations))
  labels   = merge(local.common_labels, { index = tostring(count.index) })
  format   = "ext4"
}

# ── Validator Servers ─────────────────────────────────────────────────────────

resource "hcloud_server" "validator" {
  count       = var.validator_count
  name        = "${var.cluster_name}-validator-${count.index}"
  server_type = var.server_type
  image       = var.image
  location    = element(var.locations, count.index % length(var.locations))
  ssh_keys    = [hcloud_ssh_key.operator.id]
  firewall_ids = [hcloud_firewall.validator.id]
  placement_group_id = hcloud_placement_group.validators.id
  labels      = merge(local.common_labels, {
    role  = "validator"
    index = tostring(count.index)
  })

  network {
    network_id = hcloud_network.evaporchain.id
    ip         = "10.0.1.${count.index + 10}"
    alias_ips  = []
  }

  public_net {
    ipv4_enabled = true
    ipv6_enabled = true
  }

  user_data = templatefile("${path.module}/cloud-init.yaml.tpl", {
    validator_index = count.index
    cluster_name    = var.cluster_name
    p2p_port        = var.p2p_port
    rpc_port        = var.rpc_port
    binary_url      = var.binary_download_url
    genesis_b64     = var.genesis_config_b64
    bootstrap_peers = join(",", [
      for i in range(var.validator_count) :
      "10.0.1.${i + 10}:${var.p2p_port}" if i != count.index
    ])
    node_key_b64 = length(var.validator_node_keys_b64) > count.index ? var.validator_node_keys_b64[count.index] : ""
  })

  depends_on = [hcloud_network_subnet.validators]
}

resource "hcloud_volume_attachment" "validator" {
  count     = var.validator_count
  volume_id = hcloud_volume.validator[count.index].id
  server_id = hcloud_server.validator[count.index].id
  automount = true
}

# ── Monitoring (optional single Grafana/Prometheus host) ─────────────────────

resource "hcloud_server" "monitoring" {
  count       = var.deploy_monitoring ? 1 : 0
  name        = "${var.cluster_name}-monitoring"
  server_type = "cx21"
  image       = var.image
  location    = var.locations[0]
  ssh_keys    = [hcloud_ssh_key.operator.id]
  labels      = merge(local.common_labels, { role = "monitoring" })

  public_net {
    ipv4_enabled = true
    ipv6_enabled = true
  }

  network {
    network_id = hcloud_network.evaporchain.id
    ip         = "10.0.1.200"
    alias_ips  = []
  }

  user_data = templatefile("${path.module}/monitoring-init.yaml.tpl", {
    cluster_name     = var.cluster_name
    validator_ips    = [for i in range(var.validator_count) : "10.0.1.${i + 10}"]
    metrics_port     = 9090
    grafana_password = var.grafana_admin_password
  })

  depends_on = [hcloud_network_subnet.validators]
}

# ── Locals ────────────────────────────────────────────────────────────────────

locals {
  common_labels = {
    project     = "evaporchain"
    cluster     = var.cluster_name
    managed_by  = "terraform"
  }
}
