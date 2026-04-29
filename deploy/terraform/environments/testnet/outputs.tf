output "validator_public_ips" {
  value = module.hetzner.validator_public_ips
}

output "monitoring_public_ip" {
  value = module.hetzner.monitoring_public_ip
}

output "grafana_url" {
  value = module.hetzner.grafana_url
}

output "prometheus_url" {
  value = module.hetzner.prometheus_url
}

output "bootstrap_peers" {
  value = module.hetzner.p2p_bootstrap_peers
}

output "ssh_command_validator_0" {
  value = module.hetzner.ssh_command_validator_0
}
