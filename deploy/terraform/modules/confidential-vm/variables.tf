variable "project_id" {
  description = "GCP project ID"
  type        = string
}

variable "name" {
  description = "Instance name"
  type        = string
}

variable "zone" {
  description = "GCP zone for the instance"
  type        = string
}

variable "machine_type" {
  description = "Machine type (must be n2d-* for AMD SEV)"
  type        = string
  default     = "n2d-standard-2"

  validation {
    condition     = can(regex("^n2d-", var.machine_type))
    error_message = "Machine type must be n2d-* for Confidential Space (AMD SEV)."
  }
}

variable "network_self_link" {
  description = "Self-link of the VPC network"
  type        = string
}

variable "subnet_self_link" {
  description = "Self-link of the subnet"
  type        = string
}

variable "static_internal_ip" {
  description = "Static internal IP address to assign to the instance (optional)"
  type        = string
  default     = null
}

variable "service_account_email" {
  description = "Service account email for the instance"
  type        = string
}

variable "container_image" {
  description = "Container image URI (e.g., us-central1-docker.pkg.dev/project/repo/image:tag)"
  type        = string
}

variable "environment_variables" {
  description = "Environment variables for the container"
  type        = map(string)
  default     = {}
}

variable "boot_disk_size_gb" {
  description = "Size of the boot disk in GB"
  type        = number
  default     = 20
}

variable "boot_disk_type" {
  description = "Boot disk type"
  type        = string
  default     = "pd-ssd"
}

variable "restart_policy" {
  description = "Container restart policy: Always, OnFailure, Never"
  type        = string
  default     = "OnFailure"

  validation {
    condition     = contains(["Always", "OnFailure", "Never"], var.restart_policy)
    error_message = "Restart policy must be one of: Always, OnFailure, Never."
  }
}

variable "enable_external_ip" {
  description = "Assign external IP to the instance"
  type        = bool
  default     = false
}

variable "labels" {
  description = "Labels to apply to the instance"
  type        = map(string)
  default     = {}
}

variable "tags" {
  description = "Network tags for firewall rules"
  type        = list(string)
  default     = []
}
