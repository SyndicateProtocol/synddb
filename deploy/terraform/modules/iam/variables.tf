variable "project_id" {
  description = "GCP project ID"
  type        = string
}

variable "name_prefix" {
  description = "Prefix for service account names"
  type        = string
  default     = "synddb"
}

variable "gcs_bucket_name" {
  description = "GCS bucket name for IAM bindings"
  type        = string
}

variable "artifact_registry_location" {
  description = "Location of Artifact Registry repository"
  type        = string
  default     = ""
}

variable "artifact_registry_repository" {
  description = "Name of Artifact Registry repository"
  type        = string
  default     = ""
}
