#!/bin/bash
# Update AttestationVerifier with new image digest hash
#
# Usage: ./scripts/update-image-digest.sh <image_digest> [--dry-run]
#
# Requires:
#   - DEPLOYER_PRIVATE_KEY env var
#   - RPC_URL env var
#   - ATTESTATION_VERIFIER_ADDRESS env var
#   - foundry (cast)

set -euo pipefail

IMAGE_DIGEST="${1:-}"
DRY_RUN="${2:-}"

if [[ -z "$IMAGE_DIGEST" ]]; then
    echo "Usage: $0 <image_digest> [--dry-run]"
    echo ""
    echo "Example: $0 sha256:b282cb9a839636dfc07ee08a7cad011e2d6c6d51bf09fcff1a15e2083b4be051"
    exit 1
fi

# Validate required environment variables
if [[ -z "${RPC_URL:-}" ]]; then
    echo "Error: RPC_URL environment variable is required"
    exit 1
fi

if [[ -z "${ATTESTATION_VERIFIER_ADDRESS:-}" ]]; then
    echo "Error: ATTESTATION_VERIFIER_ADDRESS environment variable is required"
    exit 1
fi

# Validate private key is set
if [[ -z "${DEPLOYER_PRIVATE_KEY:-}" ]]; then
    echo "Error: DEPLOYER_PRIVATE_KEY environment variable is required"
    exit 1
fi

# Compute keccak256 hash of the image digest string
DIGEST_HASH=$(cast keccak "$IMAGE_DIGEST")

echo "Image digest:      $IMAGE_DIGEST"
echo "Digest hash:       $DIGEST_HASH"
echo "Contract:          $ATTESTATION_VERIFIER_ADDRESS"
echo "RPC:               $RPC_URL"

# Check current value
CURRENT_HASH=$(cast call "$ATTESTATION_VERIFIER_ADDRESS" "expectedImageDigestHash()" --rpc-url "$RPC_URL")
echo "Current hash:      $CURRENT_HASH"

if [[ "$CURRENT_HASH" == "$DIGEST_HASH" ]]; then
    echo "Contract already has the correct image digest hash"
    exit 0
fi

if [[ "$DRY_RUN" == "--dry-run" ]]; then
    echo ""
    echo "[DRY RUN] Would update image digest hash to: $DIGEST_HASH"
    exit 0
fi

echo ""
echo "Updating image digest hash..."

cast send "$ATTESTATION_VERIFIER_ADDRESS" \
    "updateImageDigestHash(bytes32)" \
    "$DIGEST_HASH" \
    --private-key "$DEPLOYER_PRIVATE_KEY" \
    --rpc-url "$RPC_URL"

echo ""
echo "Successfully updated image digest hash"
