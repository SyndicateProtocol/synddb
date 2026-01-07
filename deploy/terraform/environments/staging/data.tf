# Image Info Lookups
#
# These data sources fetch image digests and secp256k1 signatures from the registry.
# The script resolves tags to digests and fetches signatures from OCI artifact referrers.
# Signatures are attached to images during CI builds using `oras attach`.
#
# Requirements:
#   - oras CLI installed (https://oras.land)
#
# The synddb registry (us-central1-docker.pkg.dev/synddb-infra/synddb) is public,
# so no authentication is needed.
#
# =============================================================================
# KNOWN LIMITATIONS / TODOs
# =============================================================================
#
# TODO(security): Digest accumulation - the update-image-digest.sh script only
#   ADDS new digest hashes, never removes old ones. Over time, the contract will
#   accumulate many allowed digests. This may be intentional (for rollbacks) but
#   creates a security concern if old vulnerable images remain allowed. Consider:
#   - Periodic manual review of allowed digests
#   - Automated removal of digests older than N versions
#   - Alert when digest count exceeds threshold
#
# TODO(operations): JWK key rotation alerting - Google rotates their attestation
#   signing keys (JWK) periodically. Rotation frequency is not publicly documented
#   (Google says "regularly"). The JWKS endpoint typically has 2+ active keys to
#   allow transition periods. JWK rotation does NOT affect running instances with
#   already-registered keys - it only impacts NEW key registrations (instance
#   restarts or new deployments). If we haven't added the new JWK hash to the
#   contract, new registrations will fail with UntrustedJwkHash. Consider:
#   - Weekly scheduled job to fetch JWKS from the well-known endpoint
#   - Compare key IDs against on-chain trustedJwkHashes mapping
#   - Alert when new keys appear (so we can add them before instances restart)
#   - JWKS endpoint: https://www.googleapis.com/service_accounts/v1/metadata/jwk/signer@confidentialspace-sign.iam.gserviceaccount.com
#   - See: https://cloud.google.com/confidential-computing/confidential-space/docs/connect-external-resources
#
# TODO(operations): Digest review alerting - no notification when image digests
#   are updated on-chain. Consider:
#   - Monitor AllowedImageDigestHashAdded/Removed events on-chain
#   - Alert to Slack/PagerDuty when digests change
#   - Include human-readable image tag in alert (requires reverse lookup)
#
# =============================================================================

locals {
  # Path to the image info lookup script
  image_info_script = "${path.module}/../../scripts/get-image-info.sh"
}

# Sequencer image signature
data "external" "sequencer_signature" {
  count   = var.tee_bootstrap != null ? 1 : 0
  program = ["bash", local.image_info_script]

  query = {
    image = var.sequencer_image
  }
}

# Validator image signature
data "external" "validator_signature" {
  count   = var.tee_bootstrap != null ? 1 : 0
  program = ["bash", local.image_info_script]

  query = {
    image = var.validator_image
  }
}

# Price oracle image signature (only when price oracle is enabled)
data "external" "price_oracle_signature" {
  count   = var.tee_bootstrap != null && var.price_oracle_config != null ? 1 : 0
  program = ["bash", local.image_info_script]

  query = {
    image = var.price_oracle_image
  }
}

# Proof service image info (includes RISC Zero image ID)
data "external" "proof_service_info" {
  count   = var.tee_bootstrap != null ? 1 : 0
  program = ["bash", local.image_info_script]

  query = {
    image = var.proof_service_image
  }
}

# -----------------------------------------------------------------------------
# AttestationVerifier Image Digest Update
# -----------------------------------------------------------------------------
# Automatically updates the on-chain AttestationVerifier contract with the
# expected image digest hash when a new sequencer image is deployed.
#
# The digest is extracted from the sequencer image reference and hashed with
# keccak256 to match what the RISC Zero program commits in the attestation proof.
#
# Requirements:
#   - deployer_private_key must be set in tfvars (not committed to git!)
#   - attestation_verifier_address must be set
#   - foundry (cast) must be installed
#   - tee_bootstrap must be enabled

locals {
  # Path to the update script
  update_digest_script = "${path.module}/../../scripts/update-image-digest.sh"

  # Extract the digest from each image reference (sha256:...)
  # Image format: registry/repo@sha256:abc123... or registry/repo:tag
  sequencer_digest = var.tee_bootstrap != null ? (
    can(regex("@(sha256:[a-f0-9]+)$", var.sequencer_image)) ?
    regex("@(sha256:[a-f0-9]+)$", var.sequencer_image)[0] :
    try(data.external.sequencer_signature[0].result.digest, "")
  ) : ""

  # Price oracle validator digest - only used when price oracle example is enabled
  # This is the application-specific validator (price-oracle-validator), not a generic validator
  price_oracle_validator_digest = var.tee_bootstrap != null && var.price_oracle_config != null ? (
    can(regex("@(sha256:[a-f0-9]+)$", var.validator_image)) ?
    regex("@(sha256:[a-f0-9]+)$", var.validator_image)[0] :
    try(data.external.validator_signature[0].result.digest, "")
  ) : ""

  # Price oracle application digest - only used when price oracle example is enabled
  price_oracle_digest = var.tee_bootstrap != null && var.price_oracle_config != null ? (
    can(regex("@(sha256:[a-f0-9]+)$", var.price_oracle_image)) ?
    regex("@(sha256:[a-f0-9]+)$", var.price_oracle_image)[0] :
    try(data.external.price_oracle_signature[0].result.digest, "")
  ) : ""
}

resource "null_resource" "update_attestation_verifier_sequencer" {
  count = (
    var.tee_bootstrap != null &&
    var.deployer_private_key != "" &&
    var.attestation_verifier_address != "" &&
    local.sequencer_digest != ""
  ) ? 1 : 0

  # Re-run when the sequencer image changes
  triggers = {
    sequencer_digest             = local.sequencer_digest
    attestation_verifier_address = var.attestation_verifier_address
  }

  provisioner "local-exec" {
    command = "${local.update_digest_script} ${local.sequencer_digest}"

    environment = {
      DEPLOYER_PRIVATE_KEY         = var.deployer_private_key
      RPC_URL                      = var.tee_bootstrap.rpc_url
      ATTESTATION_VERIFIER_ADDRESS = var.attestation_verifier_address
    }
  }

  depends_on = [
    data.external.sequencer_signature,
  ]
}

resource "null_resource" "update_attestation_verifier_price_oracle_validator" {
  count = (
    var.tee_bootstrap != null &&
    var.price_oracle_config != null &&
    var.deployer_private_key != "" &&
    var.attestation_verifier_address != "" &&
    local.price_oracle_validator_digest != ""
  ) ? 1 : 0

  # Re-run when the price oracle validator image changes
  triggers = {
    price_oracle_validator_digest = local.price_oracle_validator_digest
    attestation_verifier_address  = var.attestation_verifier_address
  }

  provisioner "local-exec" {
    command = "${local.update_digest_script} ${local.price_oracle_validator_digest}"

    environment = {
      DEPLOYER_PRIVATE_KEY         = var.deployer_private_key
      RPC_URL                      = var.tee_bootstrap.rpc_url
      ATTESTATION_VERIFIER_ADDRESS = var.attestation_verifier_address
    }
  }

  depends_on = [
    data.external.validator_signature,
  ]
}

resource "null_resource" "update_attestation_verifier_price_oracle" {
  count = (
    var.tee_bootstrap != null &&
    var.price_oracle_config != null &&
    var.deployer_private_key != "" &&
    var.attestation_verifier_address != "" &&
    local.price_oracle_digest != ""
  ) ? 1 : 0

  # Re-run when the price oracle image changes
  triggers = {
    price_oracle_digest          = local.price_oracle_digest
    attestation_verifier_address = var.attestation_verifier_address
  }

  provisioner "local-exec" {
    command = "${local.update_digest_script} ${local.price_oracle_digest}"

    environment = {
      DEPLOYER_PRIVATE_KEY         = var.deployer_private_key
      RPC_URL                      = var.tee_bootstrap.rpc_url
      ATTESTATION_VERIFIER_ADDRESS = var.attestation_verifier_address
    }
  }

  depends_on = [
    data.external.price_oracle_signature,
  ]
}

# Output the resolved signatures for debugging
output "resolved_signatures" {
  description = "Image signatures resolved from OCI artifact referrers"
  value = var.tee_bootstrap != null ? {
    sequencer = {
      found     = try(data.external.sequencer_signature[0].result.found, "false")
      signature = try(data.external.sequencer_signature[0].result.signature, "")
    }
    validator = {
      found     = try(data.external.validator_signature[0].result.found, "false")
      signature = try(data.external.validator_signature[0].result.signature, "")
    }
    price_oracle = var.price_oracle_config != null ? {
      found     = try(data.external.price_oracle_signature[0].result.found, "false")
      signature = try(data.external.price_oracle_signature[0].result.signature, "")
    } : null
  } : null
}

# Output the RISC Zero image ID for contract deployment
output "risc0_image_id" {
  description = "RISC Zero program image ID from proof-service (use for RiscZeroAttestationVerifier deployment)"
  value       = var.tee_bootstrap != null ? try(data.external.proof_service_info[0].result.image_id, "") : ""
}

# -----------------------------------------------------------------------------
# RiscZeroAttestationVerifier Image ID Update
# -----------------------------------------------------------------------------
# Automatically deploys a new RiscZeroAttestationVerifier and updates the Bridge
# when the RISC Zero image ID changes. This happens when the guest program is
# modified (rare, but requires contract redeployment since imageId is immutable).
#
# Requirements:
#   - deployer_private_key must be set in tfvars
#   - attestation_verifier_address must be set
#   - bridge_contract_address must be set
#   - trusted_image_signers must be set
#   - foundry (forge, cast) must be installed
#   - oras CLI must be installed

locals {
  # Path to the RISC Zero verifier update script
  update_risc0_script = "${path.module}/../../scripts/update-risc0-verifier.sh"

  # Get the RISC Zero image ID from proof-service (empty string if not available)
  proof_service_risc0_image_id = var.tee_bootstrap != null ? try(
    data.external.proof_service_info[0].result.image_id, ""
  ) : ""

  # Comma-separated list of trusted image signers from the sequencer signature
  trusted_image_signers = var.tee_bootstrap != null ? try(
    # For now use the signer from the signature artifact
    # This could be extended to support multiple signers
    jsondecode(data.external.sequencer_signature[0].result.signature != "" ?
      "{\"signer\": \"${var.trusted_image_signer}\"}" : "{}")["signer"],
    var.trusted_image_signer
  ) : var.trusted_image_signer
}

resource "null_resource" "update_risc0_verifier" {
  count = (
    var.tee_bootstrap != null &&
    var.deployer_private_key != "" &&
    var.attestation_verifier_address != "" &&
    var.bridge_contract_address != "" &&
    var.trusted_image_signer != "" &&
    local.proof_service_risc0_image_id != ""
  ) ? 1 : 0

  # Re-run when the RISC Zero image ID changes
  triggers = {
    risc0_image_id               = local.proof_service_risc0_image_id
    attestation_verifier_address = var.attestation_verifier_address
    bridge_contract_address      = var.bridge_contract_address
  }

  provisioner "local-exec" {
    command = local.update_risc0_script

    environment = {
      DEPLOYER_PRIVATE_KEY                = var.deployer_private_key
      RPC_URL                             = var.tee_bootstrap.rpc_url
      ATTESTATION_VERIFIER_ADDRESS        = var.attestation_verifier_address
      BRIDGE_ADDRESS                      = var.bridge_contract_address
      PROOF_SERVICE_IMAGE                 = var.proof_service_image
      SEQUENCER_IMAGE_DIGEST              = local.sequencer_digest
      PRICE_ORACLE_VALIDATOR_IMAGE_DIGEST = local.price_oracle_validator_digest
      PRICE_ORACLE_IMAGE_DIGEST           = local.price_oracle_digest
      TRUSTED_IMAGE_SIGNERS               = local.trusted_image_signers
      TFVARS_FILE                         = "${path.module}/terraform.tfvars"
      ETHERSCAN_API_KEY                   = var.etherscan_api_key
    }
  }

  depends_on = [
    data.external.proof_service_info,
    data.external.sequencer_signature,
    data.external.validator_signature,
    data.external.price_oracle_signature,
  ]
}
