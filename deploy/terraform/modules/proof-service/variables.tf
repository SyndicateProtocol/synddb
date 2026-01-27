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

# GPU Configuration (for RISC Zero)
variable "enable_gpu" {
  description = "Enable GPU for RISC Zero proving. Defaults to true."
  type        = bool
  default     = true
}

variable "gpu_type" {
  description = "GPU type for RISC Zero proving (nvidia-l4, nvidia-tesla-t4)"
  type        = string
  default     = "nvidia-l4"
}

variable "gpu_count" {
  description = "Number of GPUs to attach"
  type        = number
  default     = 1
}

# Resource Limits
# RISC Zero GPU proving requires larger instances with GPU resources
variable "cpu_limit" {
  description = "CPU limit. For GPU instances, minimum 4 cores recommended."
  type        = string
  default     = "8"
}

variable "memory_limit" {
  description = "Memory limit. For GPU instances, 32Gi recommended."
  type        = string
  default     = "32Gi"
}

# Scaling
variable "timeout_seconds" {
  description = "Request timeout in seconds (max 3600 for Groth16 proofs)"
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
  description = "Maximum concurrent requests per instance. For RISC Zero GPU proving, set to 1 since GPU proving is resource-intensive."
  type        = number
  default     = 1
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
