variable "hcloud_token" {
  description = "Hetzner Cloud API token"
  type        = string
  sensitive   = true
}

variable "cluster_name" {
  description = "Name prefix for all resources (e.g. 'evaporchain-testnet')"
  type        = string
  default     = "evaporchain-testnet"
  validation {
    condition     = can(regex("^[a-z0-9-]{3,32}$", var.cluster_name))
    error_message = "cluster_name must be lowercase alphanumeric + hyphens, 3-32 chars"
  }
}

variable "validator_count" {
  description = "Number of validator nodes to deploy"
  type        = number
  default     = 4
  validation {
    condition     = var.validator_count >= 2 && var.validator_count <= 50
    error_message = "validator_count must be between 2 and 50"
  }
}

variable "server_type" {
  description = "Hetzner server type (cx21=2vCPU/4GB €5/mo, cx31=2vCPU/8GB €10/mo, cx41=4vCPU/16GB €19/mo)"
  type        = string
  default     = "cx21"
}

variable "image" {
  description = "OS image for validators"
  type        = string
  default     = "ubuntu-24.04"
}

variable "locations" {
  description = "Hetzner datacenter locations to spread validators across"
  type        = list(string)
  default     = ["nbg1", "fsn1", "hel1"]
}

variable "network_zone" {
  description = "Hetzner network zone"
  type        = string
  default     = "eu-central"
}

variable "data_volume_gb" {
  description = "Size of the persistent data volume per validator (GB)"
  type        = number
  default     = 40
  validation {
    condition     = var.data_volume_gb >= 10 && var.data_volume_gb <= 10240
    error_message = "data_volume_gb must be 10–10240"
  }
}

variable "operator_ssh_public_key" {
  description = "SSH public key for operator access to all nodes"
  type        = string
}

variable "admin_cidr_blocks" {
  description = "CIDR blocks allowed to SSH into validators"
  type        = list(string)
  default     = ["0.0.0.0/0"]
}

variable "rpc_allowed_cidrs" {
  description = "CIDR blocks allowed to reach the HTTP RPC/API port"
  type        = list(string)
  default     = ["0.0.0.0/0"]
}

variable "metrics_allowed_cidrs" {
  description = "CIDR blocks allowed to scrape Prometheus metrics"
  type        = list(string)
  default     = ["10.0.0.0/16"]
}

variable "p2p_port" {
  description = "P2P listen port"
  type        = number
  default     = 30333
}

variable "rpc_port" {
  description = "HTTP API / RPC listen port"
  type        = number
  default     = 8080
}

variable "binary_download_url" {
  description = "URL to download the evaporchain-node binary (GitHub Release asset or S3)"
  type        = string
}

variable "genesis_config_b64" {
  description = "Base64-encoded genesis.json content"
  type        = string
  sensitive   = true
}

variable "validator_node_keys_b64" {
  description = "List of base64-encoded node private keys, one per validator. Must match validator_count."
  type        = list(string)
  default     = []
  sensitive   = true
}

variable "deploy_monitoring" {
  description = "Whether to deploy a monitoring server (Prometheus + Grafana)"
  type        = bool
  default     = true
}

variable "grafana_admin_password" {
  description = "Grafana admin password (only used if deploy_monitoring = true)"
  type        = string
  default     = ""
  sensitive   = true
}
