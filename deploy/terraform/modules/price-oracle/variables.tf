variable "project_id" {
  description = "GCP project ID"
  type        = string
}

variable "zone" {
  description = "GCP zone"
  type        = string
}

variable "name_prefix" {
  description = "Prefix for resource names"
  type        = string
}

variable "network_self_link" {
  description = "Self-link of the VPC network"
  type        = string
}

variable "subnet_self_link" {
  description = "Self-link of the subnet"
  type        = string
}

variable "service_account_email" {
  description = "Service account email"
  type        = string
}

variable "container_image" {
  description = "Price oracle container image URI"
  type        = string
}

variable "sequencer_url" {
  description = "URL to the sequencer (internal)"
  type        = string
}

variable "machine_type" {
  description = "Machine type (must be n2d-* for Confidential Space)"
  type        = string
  default     = "n2d-standard-2"
}

# Price sources
variable "coingecko_api_key" {
  description = "CoinGecko API key (optional)"
  type        = string
  default     = ""
  sensitive   = true
}

variable "cmc_api_key" {
  description = "CoinMarketCap API key (optional)"
  type        = string
  default     = ""
  sensitive   = true
}

# Daemon configuration
variable "fetch_interval" {
  description = "Price fetch interval in seconds"
  type        = number
  default     = 60
}

variable "assets" {
  description = "List of assets to track (e.g., BTC, ETH, SOL)"
  type        = list(string)
  default     = ["BTC", "ETH"]
}

# Chain monitor (for on-chain price requests)
variable "chain_monitor" {
  description = "Chain monitor configuration for on-chain price requests. Set to null to disable."
  type = object({
    rpc_url          = string
    contract_address = string
    poll_interval    = optional(number, 5)
  })
  default = null
}

# TEE bootstrap (optional - for registering app TEE key)
variable "tee_bootstrap" {
  description = "TEE key bootstrap configuration. Set to null to disable."
  type = object({
    key_manager_address  = string
    rpc_url              = string
    chain_id             = number
    proof_service_url    = string
    attestation_audience = string
  })
  default = null
}

variable "rust_log" {
  description = "Rust log level"
  type        = string
  default     = "info"
}

variable "labels" {
  description = "Labels to apply to resources"
  type        = map(string)
  default     = {}
}
