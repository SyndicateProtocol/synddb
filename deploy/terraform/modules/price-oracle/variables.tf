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
    bridge_address       = string           # Bridge contract address for key registration
    relayer_url          = string           # Relayer URL for key registration
    rpc_url              = string           # RPC URL for verifying key registration
    chain_id             = number           # Chain ID for EIP-712 signatures
    proof_service_url    = string           # URL of proof service for attestation proofs
    attestation_audience = string           # Expected audience for attestation tokens
    image_signature      = optional(string) # secp256k1 signature over keccak256(image_digest) (65 bytes r||s||v, hex)
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
