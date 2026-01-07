#!/usr/bin/env bash
# Add an allowed image digest hash to the AttestationVerifier contract
#
# Usage: ./update-image-digest.sh <digest>
#
# Arguments:
#   digest: The image digest in format "sha256:abc123..."
#
# Environment variables (required):
#   DEPLOYER_PRIVATE_KEY: Private key of the contract owner
#   RPC_URL: RPC endpoint for the target chain
#   ATTESTATION_VERIFIER_ADDRESS: Address of the AttestationVerifier contract

set -euo pipefail

DIGEST="${1:-}"

if [[ -z "$DIGEST" ]]; then
    echo "Error: digest argument required" >&2
    echo "Usage: $0 <digest>" >&2
    exit 1
fi

if [[ -z "${DEPLOYER_PRIVATE_KEY:-}" ]]; then
    echo "Error: DEPLOYER_PRIVATE_KEY environment variable required" >&2
    exit 1
fi

if [[ -z "${RPC_URL:-}" ]]; then
    echo "Error: RPC_URL environment variable required" >&2
    exit 1
fi

if [[ -z "${ATTESTATION_VERIFIER_ADDRESS:-}" ]]; then
    echo "Error: ATTESTATION_VERIFIER_ADDRESS environment variable required" >&2
    exit 1
fi

# Compute keccak256 hash of the digest string
# The contract expects keccak256(digest) as bytes32
DIGEST_HASH=$(cast keccak "$DIGEST")

echo "Checking if image digest hash is already allowed..."
echo "  Contract: $ATTESTATION_VERIFIER_ADDRESS"
echo "  Digest: $DIGEST"
echo "  Hash: $DIGEST_HASH"

# Check if the digest is already allowed
ALREADY_ALLOWED=$(cast call "$ATTESTATION_VERIFIER_ADDRESS" \
    "isImageDigestHashAllowed(bytes32)(bool)" \
    "$DIGEST_HASH" \
    --rpc-url "$RPC_URL")

if [[ "$ALREADY_ALLOWED" == "true" ]]; then
    echo "Image digest hash is already allowed, skipping..."
    exit 0
fi

echo "Adding allowed image digest hash..."

# Call addAllowedImageDigestHash on the contract
cast send "$ATTESTATION_VERIFIER_ADDRESS" \
    "addAllowedImageDigestHash(bytes32)" \
    "$DIGEST_HASH" \
    --private-key "$DEPLOYER_PRIVATE_KEY" \
    --rpc-url "$RPC_URL"

echo "Successfully added allowed image digest hash"
