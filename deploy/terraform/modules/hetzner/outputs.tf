output "validator_public_ips" {
  description = "Public IPv4 addresses of all validator nodes"
  value       = [for s in hcloud_server.validator : s.ipv4_address]
}

output "validator_private_ips" {
  description = "Private IPv4 addresses of all validator nodes (10.0.1.x)"
  value       = [for s in hcloud_server.validator : s.network[*].ip]
}

output "validator_names" {
  description = "Server names for all validators"
  value       = [for s in hcloud_server.validator : s.name]
}

output "monitoring_public_ip" {
  description = "Public IP of the monitoring server (empty if deploy_monitoring = false)"
  value       = var.deploy_monitoring ? hcloud_server.monitoring[0].ipv4_address : ""
}

output "grafana_url" {
  description = "Grafana dashboard URL"
  value       = var.deploy_monitoring ? "http://${hcloud_server.monitoring[0].ipv4_address}:3000" : ""
}

output "prometheus_url" {
  description = "Prometheus URL"
  value       = var.deploy_monitoring ? "http://${hcloud_server.monitoring[0].ipv4_address}:9090" : ""
}

output "network_id" {
  description = "Hetzner network ID"
  value       = hcloud_network.evaporchain.id
}

output "ssh_command_validator_0" {
  description = "SSH command to connect to the first validator"
  value       = "ssh root@${hcloud_server.validator[0].ipv4_address}"
}

output "p2p_bootstrap_peers" {
  description = "Bootstrap peer list for external nodes"
  value = join(",", [
    for s in hcloud_server.validator :
    "${s.ipv4_address}:${var.p2p_port}"
  ])
}
