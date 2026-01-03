# SyndDB GCP Marketplace Deployment
# Self-contained package with all resources inlined

terraform {
  required_version = ">= 1.5.0"

  required_providers {
    google = {
      source  = "hashicorp/google"
      version = ">= 5.0.0"
    }
    google-beta = {
      source  = "hashicorp/google-beta"
      version = ">= 5.0.0"
    }
    random = {
      source  = "hashicorp/random"
      version = ">= 3.0.0"
    }
  }
}

provider "google" {
  project = var.project_id
  region  = var.region
}

provider "google-beta" {
  project = var.project_id
  region  = var.region
}

locals {
  name_prefix = var.goog_cm_deployment_name
  labels = {
    deployment = var.goog_cm_deployment_name
    managed-by = "marketplace"
    product    = "synddb"
  }
}

# Enable required APIs
resource "google_project_service" "apis" {
  for_each = toset([
    "compute.googleapis.com",
    "confidentialcomputing.googleapis.com",
    "iamcredentials.googleapis.com",
    "storage.googleapis.com",
    "run.googleapis.com",
  ])

  project            = var.project_id
  service            = each.value
  disable_on_destroy = false
}

# ============================================================================
# Networking
# ============================================================================

resource "google_compute_network" "synddb" {
  name                    = "${local.name_prefix}-network"
  project                 = var.project_id
  auto_create_subnetworks = false
  routing_mode            = "REGIONAL"

  depends_on = [google_project_service.apis]
}

resource "google_compute_subnetwork" "synddb" {
  name                     = "${local.name_prefix}-subnet"
  project                  = var.project_id
  region                   = var.region
  network                  = google_compute_network.synddb.id
  ip_cidr_range            = "10.0.0.0/24"
  private_ip_google_access = true
}

resource "google_compute_firewall" "internal" {
  name    = "${local.name_prefix}-allow-internal"
  project = var.project_id
  network = google_compute_network.synddb.id

  allow {
    protocol = "tcp"
    ports    = ["0-65535"]
  }
  allow {
    protocol = "udp"
    ports    = ["0-65535"]
  }
  allow {
    protocol = "icmp"
  }

  source_ranges = ["10.0.0.0/24"]
}

resource "google_compute_firewall" "health_checks" {
  name    = "${local.name_prefix}-allow-health-checks"
  project = var.project_id
  network = google_compute_network.synddb.id

  allow {
    protocol = "tcp"
    ports    = ["8080", "8433"]
  }

  source_ranges = ["35.191.0.0/16", "130.211.0.0/22"]
  target_tags   = ["${local.name_prefix}-sequencer", "${local.name_prefix}-validator"]
}

resource "google_compute_router" "synddb" {
  name    = "${local.name_prefix}-router"
  project = var.project_id
  region  = var.region
  network = google_compute_network.synddb.id
}

resource "google_compute_router_nat" "synddb" {
  name                               = "${local.name_prefix}-nat"
  project                            = var.project_id
  router                             = google_compute_router.synddb.name
  region                             = var.region
  nat_ip_allocate_option             = "AUTO_ONLY"
  source_subnetwork_ip_ranges_to_nat = "ALL_SUBNETWORKS_ALL_IP_RANGES"
}

# ============================================================================
# IAM
# ============================================================================

resource "google_service_account" "sequencer" {
  project      = var.project_id
  account_id   = "${local.name_prefix}-seq"
  display_name = "SyndDB Sequencer (${local.name_prefix})"
}

resource "google_service_account" "validator" {
  project      = var.project_id
  account_id   = "${local.name_prefix}-val"
  display_name = "SyndDB Validator (${local.name_prefix})"
}

resource "google_service_account" "proof_service" {
  count        = var.tee_bootstrap != null ? 1 : 0
  project      = var.project_id
  account_id   = "${local.name_prefix}-proof"
  display_name = "SyndDB Proof Service (${local.name_prefix})"
}

resource "google_project_iam_member" "sequencer_cc" {
  project = var.project_id
  role    = "roles/confidentialcomputing.workloadUser"
  member  = "serviceAccount:${google_service_account.sequencer.email}"
}

resource "google_project_iam_member" "validator_cc" {
  project = var.project_id
  role    = "roles/confidentialcomputing.workloadUser"
  member  = "serviceAccount:${google_service_account.validator.email}"
}

resource "google_project_iam_member" "sequencer_logging" {
  project = var.project_id
  role    = "roles/logging.logWriter"
  member  = "serviceAccount:${google_service_account.sequencer.email}"
}

resource "google_project_iam_member" "validator_logging" {
  project = var.project_id
  role    = "roles/logging.logWriter"
  member  = "serviceAccount:${google_service_account.validator.email}"
}

# ============================================================================
# Storage
# ============================================================================

resource "google_storage_bucket" "synddb" {
  name                        = var.gcs_bucket_name
  project                     = var.project_id
  location                    = var.region
  uniform_bucket_level_access = true
  labels                      = local.labels

  depends_on = [google_project_service.apis]
}

resource "google_storage_bucket_iam_member" "sequencer_storage" {
  bucket = google_storage_bucket.synddb.name
  role   = "roles/storage.objectAdmin"
  member = "serviceAccount:${google_service_account.sequencer.email}"
}

resource "google_storage_bucket_iam_member" "validator_storage" {
  bucket = google_storage_bucket.synddb.name
  role   = "roles/storage.objectViewer"
  member = "serviceAccount:${google_service_account.validator.email}"
}

# ============================================================================
# Sequencer VM
# ============================================================================

resource "google_compute_instance" "sequencer" {
  name         = "${local.name_prefix}-sequencer"
  machine_type = var.machine_type
  zone         = var.zone
  project      = var.project_id

  confidential_instance_config {
    enable_confidential_compute = true
    confidential_instance_type  = "SEV"
  }

  shielded_instance_config {
    enable_secure_boot          = true
    enable_vtpm                 = true
    enable_integrity_monitoring = true
  }

  scheduling {
    on_host_maintenance = "TERMINATE"
  }

  boot_disk {
    initialize_params {
      image = "projects/confidential-space-images/global/images/family/confidential-space"
      size  = 20
      type  = "pd-ssd"
    }
  }

  network_interface {
    network    = google_compute_network.synddb.self_link
    subnetwork = google_compute_subnetwork.synddb.self_link
  }

  service_account {
    email  = google_service_account.sequencer.email
    scopes = ["cloud-platform"]
  }

  metadata = merge(
    {
      "tee-image-reference"        = var.sequencer_image
      "tee-restart-policy"         = "OnFailure"
      "tee-container-log-redirect" = "true"
      "tee-env-BIND_ADDRESS"   = "0.0.0.0:8433"
      "tee-env-PUBLISHER_TYPE" = "gcs"
      "tee-env-GCS_BUCKET"     = google_storage_bucket.synddb.name
      "tee-env-GCS_PREFIX"     = "sequencer"
      "tee-env-LOG_JSON"       = "true"
      "tee-env-RUST_LOG"       = var.rust_log
    },
    var.tee_bootstrap != null ? {
      "tee-env-ENABLE_KEY_BOOTSTRAP"             = "true"
      "tee-env-TEE_KEY_MANAGER_CONTRACT_ADDRESS" = var.tee_bootstrap.key_manager_address
      "tee-env-BOOTSTRAP_RPC_URL"                = var.tee_bootstrap.rpc_url
      "tee-env-BOOTSTRAP_CHAIN_ID"               = tostring(var.tee_bootstrap.chain_id)
      "tee-env-ATTESTATION_AUDIENCE"             = var.tee_bootstrap.attestation_audience
      "tee-env-PROOF_SERVICE_URL"                = google_cloud_run_v2_service.proof_service[0].uri
    } : {}
  )

  labels = local.labels
  tags   = ["${local.name_prefix}-sequencer"]

  allow_stopping_for_update = true

  depends_on = [
    google_compute_subnetwork.synddb,
    google_storage_bucket_iam_member.sequencer_storage,
    google_project_iam_member.sequencer_cc,
  ]
}

# ============================================================================
# Validator VM
# ============================================================================

resource "google_compute_instance" "validator" {
  name         = "${local.name_prefix}-validator"
  machine_type = var.machine_type
  zone         = var.zone
  project      = var.project_id

  confidential_instance_config {
    enable_confidential_compute = true
    confidential_instance_type  = "SEV"
  }

  shielded_instance_config {
    enable_secure_boot          = true
    enable_vtpm                 = true
    enable_integrity_monitoring = true
  }

  scheduling {
    on_host_maintenance = "TERMINATE"
  }

  boot_disk {
    initialize_params {
      image = "projects/confidential-space-images/global/images/family/confidential-space"
      size  = 20
      type  = "pd-ssd"
    }
  }

  network_interface {
    network    = google_compute_network.synddb.self_link
    subnetwork = google_compute_subnetwork.synddb.self_link
  }

  service_account {
    email  = google_service_account.validator.email
    scopes = ["cloud-platform"]
  }

  metadata = merge(
    {
      "tee-image-reference"        = var.validator_image
      "tee-restart-policy"         = "OnFailure"
      "tee-container-log-redirect" = "true"
      "tee-env-BIND_ADDRESS"  = "0.0.0.0:8080"
      "tee-env-FETCHER_TYPE"  = "gcs"
      "tee-env-GCS_BUCKET"    = google_storage_bucket.synddb.name
      "tee-env-GCS_PREFIX"    = "sequencer"
      "tee-env-SEQUENCER_URL" = "http://${google_compute_instance.sequencer.network_interface[0].network_ip}:8433"
      "tee-env-LOG_JSON"      = "true"
      "tee-env-RUST_LOG"      = var.rust_log
    },
    var.tee_bootstrap != null ? {
      "tee-env-ENABLE_KEY_BOOTSTRAP"             = "true"
      "tee-env-TEE_KEY_MANAGER_CONTRACT_ADDRESS" = var.tee_bootstrap.key_manager_address
      "tee-env-BOOTSTRAP_RPC_URL"                = var.tee_bootstrap.rpc_url
      "tee-env-BOOTSTRAP_CHAIN_ID"               = tostring(var.tee_bootstrap.chain_id)
      "tee-env-ATTESTATION_AUDIENCE"             = var.tee_bootstrap.attestation_audience
      "tee-env-PROOF_SERVICE_URL"                = google_cloud_run_v2_service.proof_service[0].uri
    } : {}
  )

  labels = local.labels
  tags   = ["${local.name_prefix}-validator"]

  allow_stopping_for_update = true

  depends_on = [
    google_compute_instance.sequencer,
    google_storage_bucket_iam_member.validator_storage,
    google_project_iam_member.validator_cc,
  ]
}

# ============================================================================
# Proof Service (deployed when TEE bootstrap is enabled)
# ============================================================================

resource "google_cloud_run_v2_service" "proof_service" {
  count    = var.tee_bootstrap != null ? 1 : 0
  provider = google-beta
  name     = "${local.name_prefix}-proof"
  location = var.region
  project  = var.project_id
  ingress  = "INGRESS_TRAFFIC_INTERNAL_ONLY"

  template {
    scaling {
      min_instance_count = 0
      max_instance_count = 1
    }

    timeout = "3600s"  # 60 min for Groth16/PLONK proofs

    service_account = google_service_account.proof_service[0].email

    containers {
      image = var.proof_service_image

      ports {
        container_port = 8080
      }

      resources {
        limits = {
          cpu              = "8"
          memory           = "32Gi"
          "nvidia.com/gpu" = "1"
        }
        cpu_idle          = false
        startup_cpu_boost = true
      }

      env {
        name  = "SP1_PROVER"
        value = "cuda"
      }

      env {
        name  = "LOG_JSON"
        value = "true"
      }

      startup_probe {
        http_get {
          path = "/health"
          port = 8080
        }
        initial_delay_seconds = 30
        period_seconds        = 10
        failure_threshold     = 30
      }
    }

    annotations = {
      "run.googleapis.com/gpu-type"          = "nvidia-l4"
      "run.googleapis.com/startup-cpu-boost" = "true"
    }

    max_instance_request_concurrency = 1
  }

  labels = local.labels

  depends_on = [google_project_service.apis]
}
