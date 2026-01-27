variable "project_id" {
  description = "GCP project ID"
  type        = string
}

variable "region" {
  description = "Cloud Run region"
  type        = string
  default     = "us-central1"
}

variable "service_name" {
  description = "Cloud Run service name"
  type        = string
  default     = "synddb-relayer"
}

variable "container_image" {
  description = "Relayer container image URI"
  type        = string
}

variable "service_account_email" {
  description = "Service account email"
  type        = string
}

# Blockchain Configuration
variable "rpc_url" {
  description = "RPC URL for transaction submission"
  type        = string
}

variable "chain_id" {
  description = "Chain ID for EIP-712 domain"
  type        = number
}

variable "bridge_address" {
  description = "Bridge contract address (for key registration)"
  type        = string
}

# Application Configuration
variable "required_audience" {
  description = "Audience string that identifies the application (e.g., https://example.com/app)"
  type        = string
}

variable "allowed_image_digests" {
  description = "List of allowed TEE image digests for key registration"
  type        = list(string)
  default     = []
}

# Secret Configuration
variable "private_key" {
  description = "Relayer private key (hex-encoded). Leave empty to set via Secret Manager console."
  type        = string
  default     = ""
  sensitive   = true
}

# Resource Limits
variable "cpu_limit" {
  description = "CPU limit"
  type        = string
  default     = "1"
}

variable "memory_limit" {
  description = "Memory limit"
  type        = string
  default     = "512Mi"
}

# Scaling
variable "timeout_seconds" {
  description = "Request timeout in seconds"
  type        = number
  default     = 300
}

variable "max_instances" {
  description = "Maximum number of instances"
  type        = number
  default     = 2
}

variable "min_instances" {
  description = "Minimum number of instances"
  type        = number
  default     = 0
}

variable "concurrency" {
  description = "Maximum concurrent requests per instance"
  type        = number
  default     = 80
}

# Access Control
variable "allow_unauthenticated" {
  description = "Allow unauthenticated invocations"
  type        = bool
  default     = false
}

variable "ingress" {
  description = "Ingress settings: all, internal, internal-and-cloud-load-balancing"
  type        = string
  default     = "internal"

  validation {
    condition     = contains(["all", "internal", "internal-and-cloud-load-balancing"], var.ingress)
    error_message = "Ingress must be one of: all, internal, internal-and-cloud-load-balancing."
  }
}

# VPC Connector (optional)
variable "vpc_connector" {
  description = "VPC connector name for private networking"
  type        = string
  default     = ""
}

variable "vpc_egress" {
  description = "VPC egress: all-traffic or private-ranges-only"
  type        = string
  default     = "private-ranges-only"
}

variable "rust_log" {
  description = "Rust log level"
  type        = string
  default     = "info"
}

variable "labels" {
  description = "Labels to apply"
  type        = map(string)
  default     = {}
}

variable "deletion_protection" {
  description = "Enable deletion protection for the Cloud Run service"
  type        = bool
  default     = true
}

variable "invoker_service_accounts" {
  description = "List of service account emails allowed to invoke the relayer"
  type        = list(string)
  default     = []
}
