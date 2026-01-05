variable "project_id" {
  description = "GCP project ID"
  type        = string
}

variable "region" {
  description = "GCP region"
  type        = string
  default     = "us-central1"
}

variable "zone" {
  description = "GCP zone"
  type        = string
  default     = "us-central1-a"
}

# Container images
variable "sequencer_image" {
  description = "Sequencer container image URI"
  type        = string
}

variable "validator_image" {
  description = "Validator container image URI"
  type        = string
}

variable "proof_service_image" {
  description = "Proof service container image URI"
  type        = string
}

variable "enable_proof_service" {
  description = "Enable proof service for SP1 attestation proofs"
  type        = bool
  default     = false
}

variable "sp1_network_private_key" {
  description = "SP1 Network private key for proof generation (Secp256k1 key with PROVE tokens)"
  type        = string
  sensitive   = true
  default     = ""
}

variable "relayer_image" {
  description = "Relayer container image URI"
  type        = string
  default     = ""
}

variable "price_oracle_image" {
  description = "Price oracle container image URI"
  type        = string
  default     = ""
}

# Storage
variable "gcs_bucket_name" {
  description = "GCS bucket name for batch storage"
  type        = string
}

# Instance configuration
variable "sequencer_machine_type" {
  description = "Machine type for sequencer"
  type        = string
  default     = "n2d-standard-2"
}

variable "validator_machine_type" {
  description = "Machine type for validator"
  type        = string
  default     = "n2d-standard-2"
}

variable "price_oracle_machine_type" {
  description = "Machine type for price oracle"
  type        = string
  default     = "n2d-standard-2"
}

# Artifact Registry
variable "artifact_registry_location" {
  description = "Artifact Registry location"
  type        = string
}

variable "artifact_registry_repository" {
  description = "Artifact Registry repository name"
  type        = string
}

variable "app_artifact_registry_project" {
  description = "GCP project for app images (validator, price-oracle). Only needed if hosting custom images outside synddb-infra."
  type        = string
  default     = "synddb-infra"
}

# TEE Bootstrap (null = disabled)
variable "tee_bootstrap" {
  description = "TEE key bootstrap configuration. Set to null to disable."
  type = object({
    key_manager_address  = string # TeeKeyManager contract address
    rpc_url              = string # RPC URL for bootstrap transactions
    chain_id             = number # Chain ID for bootstrap transactions
    attestation_audience = string # Expected audience for attestation tokens
  })
  default = null
}

# Bridge signer (optional)
variable "enable_bridge_signer" {
  description = "Enable bridge signer on validators"
  type        = bool
  default     = false
}

variable "bridge_contract_address" {
  description = "Bridge contract address"
  type        = string
  default     = ""
}

variable "bridge_chain_id" {
  description = "Bridge chain ID"
  type        = number
  default     = 0
}

# Batching
variable "batch_max_messages" {
  description = "Max messages per batch"
  type        = number
  default     = 100
}

variable "batch_flush_interval" {
  description = "Batch flush interval"
  type        = string
  default     = "2s"
}

# Price Oracle configuration (null = disabled)
variable "price_oracle_config" {
  description = "Price oracle configuration. Set to null to disable."
  type = object({
    coingecko_api_key      = optional(string, "")
    cmc_api_key            = optional(string, "")
    fetch_interval         = optional(number, 60)
    assets                 = optional(list(string), ["BTC", "ETH"])
    chain_monitor_enabled  = optional(bool, false)
  })
  default = null
}

variable "price_oracle_contract_address" {
  description = "PriceOracle contract address (for chain monitor)"
  type        = string
  default     = ""
}

# Relayer configuration (null = disabled)
variable "relayer_config" {
  description = "Relayer configuration. Set to null to disable."
  type = object({
    rpc_url                = string       # RPC URL for transaction submission
    chain_id               = number       # Chain ID for EIP-712 domain
    key_manager_address    = string       # TeeKeyManager contract address
    treasury_address       = string       # GasTreasury contract address
    required_audience      = string       # Audience string (e.g., https://example.com/app)
    allowed_image_digests  = list(string) # Allowed TEE image digests
  })
  default = null
}

# Labels
variable "labels" {
  description = "Labels to apply to all resources"
  type        = map(string)
  default = {
    environment = "staging"
    managed-by  = "terraform"
    project     = "synddb"
  }
}
