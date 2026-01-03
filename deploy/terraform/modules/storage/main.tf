terraform {
  required_providers {
    google = {
      source  = "hashicorp/google"
      version = ">= 5.0.0"
    }
  }
}

resource "google_storage_bucket" "synddb" {
  name          = var.bucket_name
  project       = var.project_id
  location      = var.location
  storage_class = var.storage_class
  force_destroy = var.force_destroy

  # Uniform bucket-level access (recommended for security)
  uniform_bucket_level_access = true

  # Versioning
  versioning {
    enabled = var.versioning_enabled
  }

  # Lifecycle rule for auto-cleanup (optional)
  dynamic "lifecycle_rule" {
    for_each = var.lifecycle_delete_age_days > 0 ? [1] : []
    content {
      condition {
        age = var.lifecycle_delete_age_days
      }
      action {
        type = "Delete"
      }
    }
  }

  labels = var.labels
}
