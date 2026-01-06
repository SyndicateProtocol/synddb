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
