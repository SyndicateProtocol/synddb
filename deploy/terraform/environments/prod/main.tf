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
  }

  # Remote state is required for production
  # backend "gcs" {
  #   bucket = "your-terraform-state-bucket"
  #   prefix = "synddb/prod"
  # }
}

provider "google" {
  project = var.project_id
  region  = var.region
}

provider "google-beta" {
  project = var.project_id
  region  = var.region
}

# Enable required APIs
resource "google_project_service" "apis" {
  for_each = toset([
    "compute.googleapis.com",
    "confidentialcomputing.googleapis.com",
    "iamcredentials.googleapis.com",
    "storage.googleapis.com",
    "run.googleapis.com",
    "artifactregistry.googleapis.com",
  ])

  project            = var.project_id
  service            = each.value
  disable_on_destroy = false
}

# Networking
module "networking" {
  source = "../../modules/networking"

  project_id  = var.project_id
  region      = var.region
  name_prefix = "synddb-prod"
  labels      = var.labels

  depends_on = [google_project_service.apis]
}

# Storage
module "storage" {
  source = "../../modules/storage"

  project_id         = var.project_id
  bucket_name        = var.gcs_bucket_name
  location           = var.region
  versioning_enabled = true  # Enable versioning in production
  force_destroy      = false # Prevent accidental deletion
  labels             = var.labels

  depends_on = [google_project_service.apis]
}

# IAM
module "iam" {
  source = "../../modules/iam"

  project_id                   = var.project_id
  name_prefix                  = "synddb-prod"
  gcs_bucket_name              = module.storage.bucket_name
  artifact_registry_location   = var.artifact_registry_location
  artifact_registry_repository = var.artifact_registry_repository

  depends_on = [module.storage]
}

# Proof Service (required for production TEE bootstrap)
module "proof_service" {
  source = "../../modules/proof-service"

  project_id            = var.project_id
  region                = var.region
  service_name          = "synddb-prod-proof"
  container_image       = var.proof_service_image
  service_account_email = module.iam.proof_service_account_email
  ingress               = "internal"
  allow_unauthenticated = false
  min_instances         = 0  # Scale to zero when idle
  max_instances         = 1  # Only one proof at a time
  labels                = var.labels

  depends_on = [module.iam]
}

# Sequencer
module "sequencer" {
  source = "../../modules/sequencer"

  project_id            = var.project_id
  zone                  = var.zone
  name_prefix           = "synddb-prod"
  network_self_link     = module.networking.network_self_link
  subnet_self_link      = module.networking.subnet_self_link
  service_account_email = module.iam.sequencer_service_account_email
  container_image       = var.sequencer_image
  gcs_bucket            = module.storage.bucket_name
  gcs_prefix            = var.gcs_prefix
  machine_type          = var.sequencer_machine_type
  use_debug_image       = false  # Production: no debug
  use_spot_instance     = false  # Production: stable instances
  batch_max_messages    = var.batch_max_messages
  batch_flush_interval  = var.batch_flush_interval
  enable_key_bootstrap  = var.enable_key_bootstrap
  tee_key_manager_address = var.tee_key_manager_address
  bootstrap_rpc_url     = var.bootstrap_rpc_url
  bootstrap_chain_id    = var.bootstrap_chain_id
  proof_service_url     = module.proof_service.service_url
  attestation_audience  = var.attestation_audience
  labels                = var.labels

  depends_on = [module.iam, module.networking, module.proof_service]
}

# Validators (multiple instances supported)
module "validator" {
  count  = var.validator_count
  source = "../../modules/validator"

  project_id            = var.project_id
  zone                  = var.zone
  name_prefix           = "synddb-prod"
  instance_index        = count.index
  network_self_link     = module.networking.network_self_link
  subnet_self_link      = module.networking.subnet_self_link
  service_account_email = module.iam.validator_service_account_email
  container_image       = var.validator_image
  gcs_bucket            = module.storage.bucket_name
  gcs_prefix            = var.gcs_prefix
  sequencer_url         = "http://${module.sequencer.internal_ip}:8433"
  machine_type          = var.validator_machine_type
  use_debug_image       = false
  use_spot_instance     = false
  enable_bridge_signer  = var.enable_bridge_signer
  bridge_contract_address = var.bridge_contract_address
  bridge_chain_id       = var.bridge_chain_id
  enable_key_bootstrap  = var.enable_key_bootstrap
  tee_key_manager_address = var.tee_key_manager_address
  bootstrap_rpc_url     = var.bootstrap_rpc_url
  bootstrap_chain_id    = var.bootstrap_chain_id
  proof_service_url     = module.proof_service.service_url
  attestation_audience  = var.attestation_audience
  labels                = var.labels

  depends_on = [module.iam, module.networking, module.sequencer, module.proof_service]
}
