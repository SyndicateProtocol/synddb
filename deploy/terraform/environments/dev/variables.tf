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
  description = "Proof service container image URI"
  type        = string
  default     = ""
}

# Storage
variable "gcs_bucket_name" {
  description = "GCS bucket name for batch storage"
  type        = string
}

variable "gcs_prefix" {
  description = "GCS path prefix"
  type        = string
  default     = "sequencer"
}

# Instance configuration
variable "sequencer_machine_type" {
  description = "Machine type for sequencer"
  type        = string
  default     = "n2d-standard-2"
}

variable "validator_machine_type" {
  description = "Machine type for validator"
  type        = string
  default     = "n2d-standard-2"
}

variable "use_spot_instances" {
  description = "Use spot instances for cost savings"
  type        = bool
  default     = true
}

variable "use_debug_images" {
  description = "Use debug images with SSH access"
  type        = bool
  default     = true
}

# Lifecycle
variable "lifecycle_delete_age_days" {
  description = "Delete GCS objects older than this many days (0 = disabled)"
  type        = number
  default     = 7
}

# Artifact Registry (optional)
variable "artifact_registry_location" {
  description = "Artifact Registry location"
  type        = string
  default     = ""
}

variable "artifact_registry_repository" {
  description = "Artifact Registry repository name"
  type        = string
  default     = ""
}

# TEE Bootstrap (disabled for dev by default)
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
  description = "Attestation audience"
  type        = string
  default     = ""
}

# Proof service
variable "deploy_proof_service" {
  description = "Deploy GPU proof service"
  type        = bool
  default     = false
}

# SSH access for debug
variable "allowed_ssh_ranges" {
  description = "CIDR ranges allowed for SSH (e.g., your IP)"
  type        = list(string)
  default     = []
}

# Labels
variable "labels" {
  description = "Labels to apply to all resources"
  type        = map(string)
  default = {
    environment = "dev"
    managed-by  = "terraform"
    project     = "synddb"
  }
}
