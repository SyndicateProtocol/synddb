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

# TEE Bootstrap (null = disabled, automatically deploys proof service when enabled)
variable "tee_bootstrap" {
  description = "TEE key bootstrap configuration. Set to null to disable."
  type = object({
    key_manager_address  = string # TeeKeyManager contract address
    rpc_url              = string # RPC URL for bootstrap transactions
    chain_id             = number # Chain ID for bootstrap transactions
    attestation_audience = string # Expected audience for attestation tokens
  })
  default = null
}

# Logging
variable "rust_log" {
  description = "Log level for Rust components (info, debug, warn, error)"
  type        = string
  default     = "info"
}
