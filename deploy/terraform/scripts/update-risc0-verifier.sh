#!/usr/bin/env bash
# Update or deploy the RiscZeroAttestationVerifier when the RISC Zero image ID changes
#
# This script checks if the on-chain RISC Zero image ID matches the one from the
# proof-service image. If they differ, it deploys a new RiscZeroAttestationVerifier
# and updates the Bridge to use it.
#
# Usage: ./update-risc0-verifier.sh [--dry-run]
#
# Environment variables (required):
#   DEPLOYER_PRIVATE_KEY: Private key of the contract owner
#   RPC_URL: RPC endpoint for the target chain
#   ATTESTATION_VERIFIER_ADDRESS: Current AttestationVerifier contract address
#   BRIDGE_ADDRESS: Bridge contract address
#   PROOF_SERVICE_IMAGE: Proof service image reference (tag or digest)
#   SEQUENCER_IMAGE_DIGEST: Sequencer container image digest (sha256:...)
#   TRUSTED_IMAGE_SIGNERS: Comma-separated list of trusted image signer addresses
#   TFVARS_FILE: Path to terraform.tfvars file to update (optional)
#
# Environment variables (optional):
#   TRUSTED_JWK_HASHES: Comma-separated list of trusted JWK hashes
#   EXPIRATION_TOLERANCE: Grace period in seconds (default: 3600)
#   ETHERSCAN_API_KEY: For contract verification
#
# Output:
#   Prints the new contract address if deployed, or "NO_CHANGE" if no update needed.
#   Updates TFVARS_FILE with new address if provided.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONTRACTS_DIR="$(cd "$SCRIPT_DIR/../../../contracts" && pwd)"

DRY_RUN=false
if [[ "${1:-}" == "--dry-run" ]]; then
    DRY_RUN=true
fi

# Validate required environment variables
: "${DEPLOYER_PRIVATE_KEY:?DEPLOYER_PRIVATE_KEY is required}"
: "${RPC_URL:?RPC_URL is required}"
: "${ATTESTATION_VERIFIER_ADDRESS:?ATTESTATION_VERIFIER_ADDRESS is required}"
: "${BRIDGE_ADDRESS:?BRIDGE_ADDRESS is required}"
: "${PROOF_SERVICE_IMAGE:?PROOF_SERVICE_IMAGE is required}"
: "${SEQUENCER_IMAGE_DIGEST:?SEQUENCER_IMAGE_DIGEST is required}"
: "${TRUSTED_IMAGE_SIGNERS:?TRUSTED_IMAGE_SIGNERS is required}"

# Optional variables with defaults
EXPIRATION_TOLERANCE="${EXPIRATION_TOLERANCE:-3600}"
TRUSTED_JWK_HASHES="${TRUSTED_JWK_HASHES:-0xc4339aa224c54c5dcad4bf4d0183fd5a7d4eb346b3064b0c3ea938c415b19b5f}"

# Fetch RISC Zero image ID from proof-service OCI artifact
echo "Fetching RISC Zero image ID from proof-service..." >&2
PROOF_SERVICE_INFO=$("$SCRIPT_DIR/get-image-info.sh" <<< "{\"image\": \"$PROOF_SERVICE_IMAGE\"}")
NEW_IMAGE_ID=$(echo "$PROOF_SERVICE_INFO" | jq -r '.image_id // empty')

if [[ -z "$NEW_IMAGE_ID" ]]; then
    echo "Error: Could not fetch RISC Zero image ID from proof-service" >&2
    echo "Make sure the proof-service image has the risc0-image-id artifact attached" >&2
    exit 1
fi

echo "New RISC Zero image ID: $NEW_IMAGE_ID" >&2

# Get current on-chain image ID
echo "Fetching current on-chain image ID..." >&2
CURRENT_IMAGE_ID=$(cast call "$ATTESTATION_VERIFIER_ADDRESS" "imageId()(bytes32)" --rpc-url "$RPC_URL" 2>/dev/null || echo "")

if [[ -z "$CURRENT_IMAGE_ID" ]]; then
    echo "Warning: Could not fetch current image ID (contract may not exist or have imageId())" >&2
    CURRENT_IMAGE_ID="0x0000000000000000000000000000000000000000000000000000000000000000"
fi

echo "Current on-chain image ID: $CURRENT_IMAGE_ID" >&2

# Compare image IDs (normalize to lowercase)
NEW_IMAGE_ID_LOWER=$(echo "$NEW_IMAGE_ID" | tr '[:upper:]' '[:lower:]')
CURRENT_IMAGE_ID_LOWER=$(echo "$CURRENT_IMAGE_ID" | tr '[:upper:]' '[:lower:]')

if [[ "$NEW_IMAGE_ID_LOWER" == "$CURRENT_IMAGE_ID_LOWER" ]]; then
    echo "RISC Zero image IDs match - no update needed" >&2
    echo "NO_CHANGE"
    exit 0
fi

echo "" >&2
echo "========================================" >&2
echo "RISC Zero image ID CHANGED" >&2
echo "========================================" >&2
echo "Old: $CURRENT_IMAGE_ID" >&2
echo "New: $NEW_IMAGE_ID" >&2
echo "========================================" >&2
echo "" >&2

if [[ "$DRY_RUN" == "true" ]]; then
    echo "DRY RUN: Would deploy new RiscZeroAttestationVerifier and update Bridge" >&2
    echo "DRY_RUN"
    exit 0
fi

# Compute keccak256 hash of the sequencer image digest
# The contract expects keccak256(digest) as bytes32
echo "Computing expected image digest hash..." >&2
EXPECTED_IMAGE_DIGEST_HASH=$(cast keccak "$SEQUENCER_IMAGE_DIGEST")
echo "Expected image digest hash: $EXPECTED_IMAGE_DIGEST_HASH" >&2

# Deploy new RiscZeroAttestationVerifier
echo "Deploying new RiscZeroAttestationVerifier..." >&2

cd "$CONTRACTS_DIR"

VERIFY_ARGS=""
if [[ -n "${ETHERSCAN_API_KEY:-}" ]]; then
    VERIFY_ARGS="--verify --etherscan-api-key $ETHERSCAN_API_KEY"
fi

# Run forge script and capture output
DEPLOY_OUTPUT=$(
    RISC_ZERO_IMAGE_ID="$NEW_IMAGE_ID" \
    EXPECTED_IMAGE_DIGEST_HASH="$EXPECTED_IMAGE_DIGEST_HASH" \
    EXPIRATION_TOLERANCE="$EXPIRATION_TOLERANCE" \
    TRUSTED_JWK_HASHES="$TRUSTED_JWK_HASHES" \
    TRUSTED_IMAGE_SIGNERS="$TRUSTED_IMAGE_SIGNERS" \
    forge script script/DeployRiscZeroAttestationVerifier.s.sol \
        --rpc-url "$RPC_URL" \
        --private-key "$DEPLOYER_PRIVATE_KEY" \
        --broadcast \
        $VERIFY_ARGS \
        -vvv 2>&1
) || {
    echo "Error: Deployment failed" >&2
    echo "$DEPLOY_OUTPUT" >&2
    exit 1
}

# Extract the new contract address from the output
# Look for "RiscZeroAttestationVerifier: 0x..." in the logs
NEW_VERIFIER_ADDRESS=$(echo "$DEPLOY_OUTPUT" | grep -oE "RiscZeroAttestationVerifier: 0x[a-fA-F0-9]{40}" | head -1 | cut -d' ' -f2)

if [[ -z "$NEW_VERIFIER_ADDRESS" ]]; then
    echo "Error: Could not extract new verifier address from deployment output" >&2
    echo "$DEPLOY_OUTPUT" >&2
    exit 1
fi

echo "New RiscZeroAttestationVerifier deployed at: $NEW_VERIFIER_ADDRESS" >&2

# Update the Bridge to use the new verifier
echo "Updating Bridge to use new AttestationVerifier..." >&2
cast send "$BRIDGE_ADDRESS" \
    "updateAttestationVerifier(address)" \
    "$NEW_VERIFIER_ADDRESS" \
    --private-key "$DEPLOYER_PRIVATE_KEY" \
    --rpc-url "$RPC_URL" >&2

echo "Bridge updated successfully" >&2

# Update tfvars file if provided
if [[ -n "${TFVARS_FILE:-}" && -f "$TFVARS_FILE" ]]; then
    echo "Updating $TFVARS_FILE with new address..." >&2

    # Use sed to update the attestation_verifier_address
    if grep -q "attestation_verifier_address" "$TFVARS_FILE"; then
        sed -i.bak "s|attestation_verifier_address = \"0x[a-fA-F0-9]*\"|attestation_verifier_address = \"$NEW_VERIFIER_ADDRESS\"|g" "$TFVARS_FILE"
        rm -f "${TFVARS_FILE}.bak"
        echo "Updated attestation_verifier_address in $TFVARS_FILE" >&2
    fi

    # Update the comment with new RISC Zero image ID
    if grep -q "RISC Zero image ID:" "$TFVARS_FILE"; then
        sed -i.bak "s|RISC Zero image ID: 0x[a-fA-F0-9]*|RISC Zero image ID: $NEW_IMAGE_ID|g" "$TFVARS_FILE"
        rm -f "${TFVARS_FILE}.bak"
    fi

    # Update the verifier address in comments
    if grep -q "AttestationVerifier:" "$TFVARS_FILE"; then
        TODAY=$(date +%Y-%m-%d)
        # Update the address line
        sed -i.bak "s|# AttestationVerifier: 0x[a-fA-F0-9]*|# AttestationVerifier: $NEW_VERIFIER_ADDRESS|g" "$TFVARS_FILE"
        rm -f "${TFVARS_FILE}.bak"
        # Update the basescan link
        sed -i.bak "s|sepolia.basescan.org/address/0x[a-fA-F0-9]*|sepolia.basescan.org/address/$NEW_VERIFIER_ADDRESS|g" "$TFVARS_FILE"
        rm -f "${TFVARS_FILE}.bak"
        # Update the date
        sed -i.bak "s|(Updated [0-9-]* for RISC Zero|(Updated $TODAY for RISC Zero|g" "$TFVARS_FILE"
        rm -f "${TFVARS_FILE}.bak"
    fi
fi

echo "" >&2
echo "========================================" >&2
echo "RISC Zero Verifier Update Complete" >&2
echo "========================================" >&2
echo "New AttestationVerifier: $NEW_VERIFIER_ADDRESS" >&2
echo "RISC Zero Image ID: $NEW_IMAGE_ID" >&2
echo "========================================" >&2

# Output the new address for consumption by terraform or other tools
echo "$NEW_VERIFIER_ADDRESS"
