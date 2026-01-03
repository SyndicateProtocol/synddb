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

# TEE Bootstrap Configuration
variable "enable_key_bootstrap" {
  description = "Enable TEE key bootstrapping"
  type        = bool
  default     = false
}

variable "tee_key_manager_address" {
  description = "TeeKeyManager contract address"
  type        = string
  default     = ""
}

variable "bootstrap_rpc_url" {
  description = "RPC URL for bootstrap transactions"
  type        = string
  default     = ""
}

variable "bootstrap_chain_id" {
  description = "Chain ID for bootstrap transactions"
  type        = number
  default     = 0
}

variable "proof_service_url" {
  description = "URL of GPU proof service"
  type        = string
  default     = ""
}

variable "attestation_audience" {
  description = "Expected audience for attestation tokens"
  type        = string
  default     = ""
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
