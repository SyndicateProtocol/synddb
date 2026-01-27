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
