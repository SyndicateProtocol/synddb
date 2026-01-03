variable "project_id" {
  description = "GCP project ID"
  type        = string
}

variable "zone" {
  description = "GCP zone for the sequencer VM"
  type        = string
}

variable "name_prefix" {
  description = "Prefix for resource names"
  type        = string
  default     = "synddb"
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
  description = "Sequencer container image URI"
  type        = string
}

# GCS Configuration
variable "gcs_bucket" {
  description = "GCS bucket for batch storage"
  type        = string
}

# Instance Configuration
variable "machine_type" {
  description = "Machine type"
  type        = string
  default     = "n2d-standard-2"
}

# TEE Bootstrap Configuration (null = disabled)
variable "tee_bootstrap" {
  description = "TEE key bootstrap configuration. Set to null to disable."
  type = object({
    key_manager_address  = string # TeeKeyManager contract address
    rpc_url              = string # RPC URL for bootstrap transactions
    chain_id             = number # Chain ID for bootstrap transactions
    proof_service_url    = string # URL of GPU proof service
    attestation_audience = string # Expected audience for attestation tokens
  })
  default = null
}

# Batching Configuration
variable "batch_max_messages" {
  description = "Maximum messages per batch"
  type        = number
  default     = 50
}

variable "batch_max_bytes" {
  description = "Maximum bytes per batch"
  type        = number
  default     = 1048576
}

variable "batch_flush_interval" {
  description = "Batch flush interval"
  type        = string
  default     = "5s"
}

# Logging
variable "rust_log" {
  description = "Rust log level (e.g., info, debug, warn, error, or module-specific like synddb_sequencer=debug)"
  type        = string
  default     = "info"
}

variable "labels" {
  description = "Labels to apply"
  type        = map(string)
  default     = {}
}
