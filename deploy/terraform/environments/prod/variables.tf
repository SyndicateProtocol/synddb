variable "project_id" {
  description = "GCP project ID"
  type        = string
}

variable "region" {
  description = "GCP region"
  type        = string
  default     = "us-central1"
}

variable "zone" {
  description = "GCP zone"
  type        = string
  default     = "us-central1-a"
}

# Container images (should be pinned versions)
variable "sequencer_image" {
  description = "Sequencer container image URI"
  type        = string
}

variable "validator_image" {
  description = "Validator container image URI"
  type        = string
}

variable "proof_service_image" {
  description = "Proof service container image URI"
  type        = string
}

variable "relayer_image" {
  description = "Relayer container image URI"
  type        = string
  default     = ""
}

# Storage
variable "gcs_bucket_name" {
  description = "GCS bucket name for batch storage"
  type        = string
}

# Instance configuration
variable "sequencer_machine_type" {
  description = "Machine type for sequencer"
  type        = string
  default     = "n2d-standard-4"
}

variable "validator_machine_type" {
  description = "Machine type for validator"
  type        = string
  default     = "n2d-standard-4"
}

variable "validator_count" {
  description = "Number of validator instances"
  type        = number
  default     = 1
}

# Artifact Registry
variable "artifact_registry_location" {
  description = "Artifact Registry location"
  type        = string
}

variable "artifact_registry_repository" {
  description = "Artifact Registry repository name"
  type        = string
}

# TEE Bootstrap (null = disabled)
variable "tee_bootstrap" {
  description = "TEE key bootstrap configuration. Set to null to disable."
  type = object({
    bridge_address       = string # Bridge contract address for key registration
    relayer_url          = string # Relayer URL for key registration
    rpc_url              = string # RPC URL for verifying key registration
    chain_id             = number # Chain ID for EIP-712 signatures
    attestation_audience = string # Expected audience for attestation tokens
  })
  default = null
}

# Bridge signer (optional)
variable "enable_bridge_signer" {
  description = "Enable bridge signer on validators"
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

# Batching (production tuning)
variable "batch_max_messages" {
  description = "Max messages per batch"
  type        = number
  default     = 100
}

variable "batch_flush_interval" {
  description = "Batch flush interval"
  type        = string
  default     = "2s"
}

# Relayer configuration (null = disabled)
variable "relayer_config" {
  description = "Relayer configuration. Set to null to disable."
  type = object({
    rpc_url               = string       # RPC URL for transaction submission
    chain_id              = number       # Chain ID for EIP-712 domain
    bridge_address        = string       # Bridge contract address for key registration
    required_audience     = string       # Audience string (e.g., https://example.com/app)
    allowed_image_digests = list(string) # Allowed TEE image digests
  })
  default = null
}

# Labels
variable "labels" {
  description = "Labels to apply to all resources"
  type        = map(string)
  default = {
    environment = "prod"
    managed-by  = "terraform"
    project     = "synddb"
  }
}
