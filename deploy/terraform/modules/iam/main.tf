terraform {
  required_providers {
    google = {
      source  = "hashicorp/google"
      version = ">= 5.0.0"
    }
  }
}

# Sequencer service account
resource "google_service_account" "sequencer" {
  project      = var.project_id
  account_id   = "${var.name_prefix}-sequencer"
  display_name = "SyndDB Sequencer"
  description  = "Service account for SyndDB sequencer running in Confidential Space"
}

# Validator service account
resource "google_service_account" "validator" {
  project      = var.project_id
  account_id   = "${var.name_prefix}-validator"
  display_name = "SyndDB Validator"
  description  = "Service account for SyndDB validator running in Confidential Space"
}

# Proof service account
resource "google_service_account" "proof_service" {
  project      = var.project_id
  account_id   = "${var.name_prefix}-proof"
  display_name = "SyndDB Proof Service"
  description  = "Service account for SyndDB proof generation service"
}

# Relayer service account
resource "google_service_account" "relayer" {
  project      = var.project_id
  account_id   = "${var.name_prefix}-relayer"
  display_name = "SyndDB Relayer"
  description  = "Service account for SyndDB gas funding relayer"
}

# Price Oracle service account
resource "google_service_account" "price_oracle" {
  project      = var.project_id
  account_id   = "${var.name_prefix}-price-oracle"
  display_name = "SyndDB Price Oracle"
  description  = "Service account for SyndDB price oracle running in Confidential Space"
}

# Confidential Computing workload user role (required for attestation)
resource "google_project_iam_member" "sequencer_cc_workload" {
  project = var.project_id
  role    = "roles/confidentialcomputing.workloadUser"
  member  = "serviceAccount:${google_service_account.sequencer.email}"
}

resource "google_project_iam_member" "validator_cc_workload" {
  project = var.project_id
  role    = "roles/confidentialcomputing.workloadUser"
  member  = "serviceAccount:${google_service_account.validator.email}"
}

resource "google_project_iam_member" "price_oracle_cc_workload" {
  project = var.project_id
  role    = "roles/confidentialcomputing.workloadUser"
  member  = "serviceAccount:${google_service_account.price_oracle.email}"
}

# Logging permissions
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

resource "google_project_iam_member" "proof_service_logging" {
  project = var.project_id
  role    = "roles/logging.logWriter"
  member  = "serviceAccount:${google_service_account.proof_service.email}"
}

resource "google_project_iam_member" "relayer_logging" {
  project = var.project_id
  role    = "roles/logging.logWriter"
  member  = "serviceAccount:${google_service_account.relayer.email}"
}

resource "google_project_iam_member" "price_oracle_logging" {
  project = var.project_id
  role    = "roles/logging.logWriter"
  member  = "serviceAccount:${google_service_account.price_oracle.email}"
}

# GCS permissions - sequencer writes, validator reads
resource "google_storage_bucket_iam_member" "sequencer_storage" {
  bucket = var.gcs_bucket_name
  role   = "roles/storage.objectAdmin"
  member = "serviceAccount:${google_service_account.sequencer.email}"
}

resource "google_storage_bucket_iam_member" "validator_storage" {
  bucket = var.gcs_bucket_name
  role   = "roles/storage.objectViewer"
  member = "serviceAccount:${google_service_account.validator.email}"
}

# Artifact Registry reader (for pulling container images)
resource "google_artifact_registry_repository_iam_member" "sequencer_ar" {
  count      = var.artifact_registry_repository != "" ? 1 : 0
  project    = var.project_id
  location   = var.artifact_registry_location
  repository = var.artifact_registry_repository
  role       = "roles/artifactregistry.reader"
  member     = "serviceAccount:${google_service_account.sequencer.email}"
}

resource "google_artifact_registry_repository_iam_member" "validator_ar" {
  count      = var.artifact_registry_repository != "" ? 1 : 0
  project    = var.project_id
  location   = var.artifact_registry_location
  repository = var.artifact_registry_repository
  role       = "roles/artifactregistry.reader"
  member     = "serviceAccount:${google_service_account.validator.email}"
}

resource "google_artifact_registry_repository_iam_member" "price_oracle_ar" {
  count      = var.artifact_registry_repository != "" ? 1 : 0
  project    = var.project_id
  location   = var.artifact_registry_location
  repository = var.artifact_registry_repository
  role       = "roles/artifactregistry.reader"
  member     = "serviceAccount:${google_service_account.price_oracle.email}"
}
