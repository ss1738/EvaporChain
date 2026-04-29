terraform {
  required_version = ">= 1.6"
  backend "s3" {
    # Use: terraform init -backend-config=backend.hcl
    # backend.hcl should contain bucket, key, region, endpoint for your S3-compatible store
  }
}

module "hetzner" {
  source = "../../modules/hetzner"

  hcloud_token            = var.hcloud_token
  cluster_name            = "evaporchain-testnet"
  validator_count         = var.validator_count
  server_type             = var.server_type
  locations               = var.locations
  data_volume_gb          = var.data_volume_gb
  operator_ssh_public_key = var.operator_ssh_public_key
  admin_cidr_blocks       = var.admin_cidr_blocks
  rpc_allowed_cidrs       = ["0.0.0.0/0"]
  metrics_allowed_cidrs   = ["10.0.0.0/16"]
  binary_download_url     = var.binary_download_url
  genesis_config_b64      = var.genesis_config_b64
  validator_node_keys_b64 = var.validator_node_keys_b64
  deploy_monitoring       = true
  grafana_admin_password  = var.grafana_admin_password
}
