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
  description = "Proof service container image URI (optional)"
  type        = string
  default     = ""
}

# Storage
variable "gcs_bucket_name" {
  description = "GCS bucket name for batch storage"
  type        = string
}

variable "gcs_prefix" {
  description = "Path prefix within the bucket"
  type        = string
  default     = "sequencer"
}

# Machine types
variable "sequencer_machine_type" {
  description = "Machine type for sequencer VM"
  type        = string
  default     = "n2d-standard-2"
}

variable "validator_machine_type" {
  description = "Machine type for validator VM"
  type        = string
  default     = "n2d-standard-2"
}

# TEE Bootstrap
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

variable "attestation_audience" {
  description = "Expected audience for attestation tokens"
  type        = string
  default     = ""
}

# Proof service
variable "deploy_proof_service" {
  description = "Deploy GPU proof service (requires GPU quota)"
  type        = bool
  default     = false
}

# Logging
variable "rust_log" {
  description = "Log level for Rust components (info, debug, warn, error)"
  type        = string
  default     = "info"
}
