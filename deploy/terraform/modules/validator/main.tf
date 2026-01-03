terraform {
  required_providers {
    google = {
      source  = "hashicorp/google"
      version = ">= 5.0.0"
    }
  }
}

locals {
  instance_name = var.instance_index > 0 ? "${var.name_prefix}-validator-${var.instance_index}" : "${var.name_prefix}-validator"

  # Build environment variables for the validator
  env_vars = merge(
    {
      # Core configuration
      BIND_ADDRESS = "0.0.0.0:8080"

      # Fetcher configuration
      FETCHER_TYPE = var.fetcher_type
      GCS_BUCKET   = var.gcs_bucket
      GCS_PREFIX   = var.gcs_prefix

      # Sync configuration
      SYNC_INTERVAL      = var.sync_interval
      BATCH_SYNC_ENABLED = tostring(var.batch_sync_enabled)

      # Logging
      LOG_JSON = tostring(var.log_json)
    },
    # Sequencer connection
    var.sequencer_url != "" ? {
      SEQUENCER_URL = var.sequencer_url
    } : {},
    var.sequencer_pubkey != "" ? {
      SEQUENCER_PUBKEY = var.sequencer_pubkey
    } : {},
    # Bridge signer configuration (if enabled)
    var.enable_bridge_signer ? {
      BRIDGE_SIGNER           = "true"
      BRIDGE_CONTRACT_ADDRESS = var.bridge_contract_address
      BRIDGE_CHAIN_ID         = tostring(var.bridge_chain_id)
    } : {},
    # TEE bootstrap configuration (if enabled)
    var.enable_key_bootstrap ? {
      ENABLE_KEY_BOOTSTRAP            = "true"
      TEE_KEY_MANAGER_CONTRACT_ADDRESS = var.tee_key_manager_address
      BOOTSTRAP_RPC_URL               = var.bootstrap_rpc_url
      BOOTSTRAP_CHAIN_ID              = tostring(var.bootstrap_chain_id)
      PROOF_SERVICE_URL               = var.proof_service_url
      ATTESTATION_AUDIENCE            = var.attestation_audience
    } : {}
  )
}

module "confidential_vm" {
  source = "../confidential-vm"

  project_id            = var.project_id
  name                  = local.instance_name
  zone                  = var.zone
  machine_type          = var.machine_type
  network_self_link     = var.network_self_link
  subnet_self_link      = var.subnet_self_link
  service_account_email = var.service_account_email
  container_image       = var.container_image
  environment_variables = local.env_vars
  labels                = var.labels
  tags                  = ["${var.name_prefix}-validator"]
}
