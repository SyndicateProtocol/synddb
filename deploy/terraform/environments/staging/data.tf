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

  # Extract the digest from the sequencer image reference (sha256:...)
  # Image format: registry/repo@sha256:abc123... or registry/repo:tag
  sequencer_digest = var.tee_bootstrap != null ? (
    can(regex("@(sha256:[a-f0-9]+)$", var.sequencer_image)) ?
    regex("@(sha256:[a-f0-9]+)$", var.sequencer_image)[0] :
    try(data.external.sequencer_signature[0].result.digest, "")
  ) : ""
}

resource "null_resource" "update_attestation_verifier" {
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
