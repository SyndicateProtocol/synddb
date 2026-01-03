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

variable "gcs_prefix" {
  description = "GCS path prefix"
  type        = string
  default     = "sequencer"
}

# Instance Configuration
variable "machine_type" {
  description = "Machine type"
  type        = string
  default     = "n2d-standard-2"
}

variable "use_debug_image" {
  description = "Use debug image with SSH access"
  type        = bool
  default     = false
}

variable "use_spot_instance" {
  description = "Use spot instance"
  type        = bool
  default     = false
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
variable "log_json" {
  description = "Enable JSON logging"
  type        = bool
  default     = true
}

variable "labels" {
  description = "Labels to apply"
  type        = map(string)
  default     = {}
}
