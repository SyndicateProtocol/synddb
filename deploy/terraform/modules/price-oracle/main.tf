terraform {
  required_providers {
    google = {
      source  = "hashicorp/google"
      version = ">= 5.0.0"
    }
  }
}

locals {
  # Build environment variables for the price oracle
  env_vars = merge(
    {
      # Core configuration
      DATABASE_PATH = "/data/price-oracle.db"
      SEQUENCER_URL = var.sequencer_url

      # Daemon configuration
      FETCH_INTERVAL = tostring(var.fetch_interval)
      ASSETS         = join(",", var.assets)

      # Logging
      RUST_LOG = var.rust_log
    },
    # API keys (if provided)
    var.coingecko_api_key != "" ? {
      COINGECKO_API_KEY = var.coingecko_api_key
    } : {},
    var.cmc_api_key != "" ? {
      CMC_API_KEY = var.cmc_api_key
    } : {},
    # Chain monitor configuration (if enabled)
    var.chain_monitor != null ? {
      CHAIN_MONITOR_ENABLED  = "true"
      CHAIN_MONITOR_RPC_URL  = var.chain_monitor.rpc_url
      CHAIN_MONITOR_CONTRACT = var.chain_monitor.contract_address
      CHAIN_MONITOR_POLL     = tostring(var.chain_monitor.poll_interval)
    } : {},
    # TEE bootstrap configuration (if enabled)
    var.tee_bootstrap != null ? {
      ENABLE_KEY_BOOTSTRAP             = "true"
      TEE_KEY_MANAGER_CONTRACT_ADDRESS = var.tee_bootstrap.key_manager_address
      BOOTSTRAP_RPC_URL                = var.tee_bootstrap.rpc_url
      BOOTSTRAP_CHAIN_ID               = tostring(var.tee_bootstrap.chain_id)
      PROOF_SERVICE_URL                = var.tee_bootstrap.proof_service_url
      ATTESTATION_AUDIENCE             = var.tee_bootstrap.attestation_audience
    } : {}
  )
}

module "confidential_vm" {
  source = "../confidential-vm"

  project_id            = var.project_id
  name                  = "${var.name_prefix}-price-oracle"
  zone                  = var.zone
  machine_type          = var.machine_type
  network_self_link     = var.network_self_link
  subnet_self_link      = var.subnet_self_link
  service_account_email = var.service_account_email
  container_image       = var.container_image
  environment_variables = local.env_vars
  labels                = var.labels
  tags                  = ["${var.name_prefix}-price-oracle"]
}
