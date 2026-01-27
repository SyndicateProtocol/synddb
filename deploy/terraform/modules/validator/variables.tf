variable "project_id" {
  description = "GCP project ID"
  type        = string
}

variable "zone" {
  description = "GCP zone for the validator VM"
  type        = string
}

variable "name_prefix" {
  description = "Prefix for resource names"
  type        = string
  default     = "synddb"
}

variable "instance_index" {
  description = "Index for multiple validator instances (0, 1, 2...)"
  type        = number
  default     = 0
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
  description = "Validator container image URI"
  type        = string
}

# Sequencer connection
variable "sequencer_url" {
  description = "Sequencer URL for pubkey discovery"
  type        = string
}

# GCS Configuration
variable "gcs_bucket" {
  description = "GCS bucket for batch fetching"
  type        = string
}

# Instance Configuration
variable "machine_type" {
  description = "Machine type"
  type        = string
  default     = "n2d-standard-2"
}

# Bridge Signer Configuration
variable "enable_bridge_signer" {
  description = "Enable bridge signer mode"
  type        = bool
  default     = false
}

variable "bridge_contract_address" {
  description = "Bridge contract address"
  type        = string
  default     = ""
}

variable "bridge_chain_id" {
  description = "Bridge chain ID"
  type        = number
  default     = 0
}

# TEE Bootstrap Configuration (null = disabled)
variable "tee_bootstrap" {
  description = "TEE key bootstrap configuration. Set to null to disable."
  type = object({
    key_manager_address  = string # TeeKeyManager contract address
    rpc_url              = string # RPC URL for bootstrap transactions
    chain_id             = number # Chain ID for bootstrap transactions
    proof_service_url    = string # URL of proof service for attestation proofs
    attestation_audience = string # Expected audience for attestation tokens
  })
  default = null
}

# Logging
variable "rust_log" {
  description = "Rust log level (e.g., info, debug, warn, error, or module-specific like synddb_validator=debug)"
  type        = string
  default     = "info"
}

variable "labels" {
  description = "Labels to apply"
  type        = map(string)
  default     = {}
}
