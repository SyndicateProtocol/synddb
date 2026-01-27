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
  default     = "proof-service"
}

variable "container_image" {
  description = "Proof service container image URI"
  type        = string
}

variable "service_account_email" {
  description = "Service account email"
  type        = string
}

# SP1 Network Prover
variable "sp1_network_private_key" {
  description = "SP1 Network private key for proof generation (Secp256k1 key with PROVE tokens)"
  type        = string
  sensitive   = true
}

# Resource Limits (small instance - heavy work offloaded to SP1 Network)
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
  description = "Request timeout in seconds (max 3600 for Groth16/PLONK proofs)"
  type        = number
  default     = 3600
}

variable "max_instances" {
  description = "Maximum number of instances. Cloud Run scales out when CPU utilization exceeds ~60% during local proof verification."
  type        = number
  default     = 3
}

variable "min_instances" {
  description = "Minimum number of instances"
  type        = number
  default     = 0
}

variable "concurrency" {
  description = "Maximum concurrent requests per instance. Set high since most request time is spent waiting on SP1 Network. Cloud Run auto-scales based on CPU utilization (~60% target) during verification."
  type        = number
  default     = 10
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

# VPC Connector (optional, for private networking)
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

variable "labels" {
  description = "Labels to apply"
  type        = map(string)
  default     = {}
}

variable "invoker_service_accounts" {
  description = "List of service account emails allowed to invoke the proof service"
  type        = list(string)
  default     = []
}
