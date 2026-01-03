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

# GPU Configuration
variable "gpu_type" {
  description = "GPU type"
  type        = string
  default     = "nvidia-l4"
}

variable "gpu_count" {
  description = "Number of GPUs"
  type        = number
  default     = 1
}

# Resource Limits
variable "cpu_limit" {
  description = "CPU limit"
  type        = string
  default     = "8"
}

variable "memory_limit" {
  description = "Memory limit"
  type        = string
  default     = "32Gi"
}

# Scaling
variable "timeout_seconds" {
  description = "Request timeout in seconds"
  type        = number
  default     = 900
}

variable "max_instances" {
  description = "Maximum number of instances"
  type        = number
  default     = 1
}

variable "min_instances" {
  description = "Minimum number of instances"
  type        = number
  default     = 0
}

variable "concurrency" {
  description = "Maximum concurrent requests per instance"
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
