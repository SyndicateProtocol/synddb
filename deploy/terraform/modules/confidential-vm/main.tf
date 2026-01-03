terraform {
  required_providers {
    google = {
      source  = "hashicorp/google"
      version = ">= 5.0.0"
    }
  }
}

locals {
  # Build metadata map with tee-env-* prefixes for environment variables
  env_metadata = { for k, v in var.environment_variables : "tee-env-${k}" => v }

  # Base metadata for Confidential Space
  base_metadata = {
    "tee-image-reference"        = var.container_image
    "tee-restart-policy"         = var.restart_policy
    "tee-container-log-redirect" = "true"
  }

  # Combine base and environment metadata
  all_metadata = merge(local.base_metadata, local.env_metadata)
}

resource "google_compute_instance" "confidential_vm" {
  name         = var.name
  machine_type = var.machine_type
  zone         = var.zone
  project      = var.project_id

  # Confidential Computing configuration (AMD SEV)
  confidential_instance_config {
    enable_confidential_compute = true
    confidential_instance_type  = "SEV"
  }

  # Shielded VM configuration (required for Confidential Space)
  shielded_instance_config {
    enable_secure_boot          = true
    enable_vtpm                 = true
    enable_integrity_monitoring = true
  }

  # Scheduling configuration
  scheduling {
    on_host_maintenance = "TERMINATE"
    provisioning_model  = "STANDARD"
    automatic_restart   = true
  }

  # Boot disk with Confidential Space image (production only - no debug/SSH access)
  boot_disk {
    initialize_params {
      image = "projects/confidential-space-images/global/images/family/confidential-space"
      size  = var.boot_disk_size_gb
      type  = var.boot_disk_type
    }
  }

  # Network interface
  network_interface {
    network    = var.network_self_link
    subnetwork = var.subnet_self_link

    # External IP if explicitly requested
    dynamic "access_config" {
      for_each = var.enable_external_ip ? [1] : []
      content {
        # Ephemeral IP
      }
    }
  }

  # Service account
  service_account {
    email  = var.service_account_email
    scopes = ["cloud-platform"]
  }

  # Metadata for Confidential Space workload
  metadata = local.all_metadata

  labels = var.labels
  tags   = var.tags

  # Allow stopping for updates
  allow_stopping_for_update = true
}
