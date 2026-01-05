# Required Marketplace variables
variable "project_id" {
  description = "GCP project ID for deployment"
  type        = string
}

variable "goog_cm_deployment_name" {
  description = "Marketplace deployment name (automatically provided)"
  type        = string
}

# Region and zone
variable "region" {
  description = "GCP region for resources"
  type        = string
  default     = "us-central1"
}

variable "zone" {
  description = "GCP zone for compute instances"
  type        = string
  default     = "us-central1-a"
}

# Container images
variable "sequencer_image" {
  description = "Sequencer container image URI"
  type        = string
}

variable "validator_image" {
  description = "Validator container image URI"
  type        = string
}

variable "proof_service_image" {
  description = "Proof service container image URI (required when TEE bootstrap is enabled)"
  type        = string
  default     = ""
}

variable "sp1_network_private_key" {
  description = "SP1 Network private key for proof generation (Secp256k1 key with PROVE tokens)"
  type        = string
  sensitive   = true
  default     = ""
}

variable "relayer_image" {
  description = "Relayer container image URI (required when relayer is enabled)"
  type        = string
  default     = ""
}

# Storage
variable "gcs_bucket_name" {
  description = "GCS bucket name for batch storage"
  type        = string
}

# Machine type (used for both sequencer and validator)
variable "machine_type" {
  description = "Machine type for VMs (must be n2d-* for AMD SEV)"
  type        = string
  default     = "n2d-standard-2"

  validation {
    condition     = can(regex("^n2d-", var.machine_type))
    error_message = "Machine type must be n2d-* for Confidential Space (AMD SEV)."
  }
}

# Bridge contract (used by TEE bootstrap and relayer)
variable "bridge_contract_address" {
  description = "Bridge contract address (shared across all services)"
  type        = string
  default     = ""
}

variable "bridge_chain_id" {
  description = "Chain ID for the bridge contract"
  type        = number
  default     = 0
}

# TEE Bootstrap (null = disabled, automatically deploys proof service when enabled)
variable "tee_bootstrap" {
  description = "TEE key bootstrap configuration. Set to null to disable."
  type = object({
    relayer_url          = string # Relayer URL for key registration
    rpc_url              = string # RPC URL for verifying key registration
    attestation_audience = string # Expected audience for attestation tokens
  })
  default = null
}

# Relayer configuration (null = disabled)
variable "relayer_config" {
  description = "Relayer configuration. Set to null to disable."
  type = object({
    rpc_url               = string       # RPC URL for transaction submission
    required_audience     = string       # Audience string (e.g., https://example.com/app)
    allowed_image_digests = list(string) # Allowed TEE image digests
  })
  default = null
}

# Logging
variable "rust_log" {
  description = "Log level for Rust components (info, debug, warn, error)"
  type        = string
  default     = "info"
}
