terraform {
  required_providers {
    google = {
      source  = "hashicorp/google"
      version = ">= 5.0.0"
    }
  }
}

locals {
  # Build environment variables for the sequencer
  env_vars = merge(
    {
      # Core configuration
      BIND_ADDRESS   = "0.0.0.0:8433"
      PUBLISHER_TYPE = "gcs"
      GCS_BUCKET     = var.gcs_bucket
      GCS_PREFIX     = "sequencer"

      # Batching
      BATCH_MAX_MESSAGES   = tostring(var.batch_max_messages)
      BATCH_MAX_BYTES      = tostring(var.batch_max_bytes)
      BATCH_FLUSH_INTERVAL = var.batch_flush_interval

      # Logging
      LOG_JSON = "true"
      RUST_LOG = var.rust_log
    },
    # TEE bootstrap configuration (if enabled)
    var.tee_bootstrap != null ? {
      ENABLE_KEY_BOOTSTRAP    = "true"
      BRIDGE_CONTRACT_ADDRESS = var.tee_bootstrap.bridge_address
      RELAYER_URL             = var.tee_bootstrap.relayer_url
      BOOTSTRAP_RPC_URL       = var.tee_bootstrap.rpc_url
      BOOTSTRAP_CHAIN_ID      = tostring(var.tee_bootstrap.chain_id)
      PROOF_SERVICE_URL       = var.tee_bootstrap.proof_service_url
      ATTESTATION_AUDIENCE    = var.tee_bootstrap.attestation_audience
    } : {}
  )
}

module "confidential_vm" {
  source = "../confidential-vm"

  project_id            = var.project_id
  name                  = "${var.name_prefix}-sequencer"
  zone                  = var.zone
  machine_type          = var.machine_type
  network_self_link     = var.network_self_link
  subnet_self_link      = var.subnet_self_link
  service_account_email = var.service_account_email
  container_image       = var.container_image
  environment_variables = local.env_vars
  labels                = var.labels
  tags                  = ["${var.name_prefix}-sequencer"]
}
