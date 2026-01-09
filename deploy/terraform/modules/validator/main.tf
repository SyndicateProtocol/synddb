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
      BIND_ADDRESS  = "0.0.0.0:8080"
      SEQUENCER_URL = var.sequencer_url

      # Database paths (container-local storage, state lost on restart)
      # For production, mount a persistent disk at /data
      DATABASE_PATH              = "/data/validator.db"
      STATE_DB_PATH              = "/data/validator_state.db"
      PENDING_CHANGESETS_DB_PATH = "/data/pending_changesets.db"

      # Fetcher configuration
      FETCHER_TYPE = "gcs"
      GCS_BUCKET   = var.gcs_bucket
      GCS_PREFIX   = "sequencer"

      # Sync configuration
      SYNC_INTERVAL      = "1s"
      BATCH_SYNC_ENABLED = "true"

      # Logging
      LOG_JSON = "true"
      RUST_LOG = var.rust_log
    },
    # Bridge signer configuration (if enabled)
    var.enable_bridge_signer ? {
      BRIDGE_SIGNER           = "true"
      BRIDGE_CONTRACT_ADDRESS = var.bridge_contract_address
      BRIDGE_CHAIN_ID         = tostring(var.bridge_chain_id)
    } : {},
    # TEE bootstrap configuration (if enabled)
    var.tee_bootstrap != null ? merge(
      {
        ENABLE_KEY_BOOTSTRAP    = "true"
        BRIDGE_CONTRACT_ADDRESS = var.tee_bootstrap.bridge_address
        RELAYER_URL             = var.tee_bootstrap.relayer_url
        BOOTSTRAP_RPC_URL       = var.tee_bootstrap.rpc_url
        BOOTSTRAP_CHAIN_ID      = tostring(var.tee_bootstrap.chain_id)
        PROOF_SERVICE_URL       = var.tee_bootstrap.proof_service_url
        ATTESTATION_AUDIENCE    = var.tee_bootstrap.attestation_audience
      },
      # Image signature for on-chain verification (optional)
      var.tee_bootstrap.image_signature != null ? { IMAGE_SIGNATURE = var.tee_bootstrap.image_signature } : {}
    ) : {}
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
  static_internal_ip    = var.static_internal_ip
  service_account_email = var.service_account_email
  container_image       = var.container_image
  environment_variables = local.env_vars
  labels                = var.labels
  tags                  = ["${var.name_prefix}-validator"]
}
