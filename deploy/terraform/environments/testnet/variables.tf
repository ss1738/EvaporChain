variable "hcloud_token" {
  type      = string
  sensitive = true
}

variable "validator_count" {
  type    = number
  default = 4
}

variable "server_type" {
  type    = string
  default = "cx21"
}

variable "locations" {
  type    = list(string)
  default = ["nbg1", "fsn1", "hel1"]
}

variable "data_volume_gb" {
  type    = number
  default = 40
}

variable "operator_ssh_public_key" {
  type = string
}

variable "admin_cidr_blocks" {
  type    = list(string)
  default = ["0.0.0.0/0"]
}

variable "binary_download_url" {
  type = string
}

variable "genesis_config_b64" {
  type      = string
  sensitive = true
}

variable "validator_node_keys_b64" {
  type      = list(string)
  default   = []
  sensitive = true
}

variable "grafana_admin_password" {
  type      = string
  sensitive = true
}
