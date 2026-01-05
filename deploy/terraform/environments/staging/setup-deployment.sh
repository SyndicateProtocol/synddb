#!/usr/bin/env bash
# Setup script for SyndDB staging deployment
# Fetches image digests from Artifact Registry and prepares deployment variables

set -euo pipefail

# Configuration
REGISTRY="us-central1-docker.pkg.dev/synddb-infra/synddb"
TAG="${IMAGE_TAG:-edge}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

info() { echo -e "${GREEN}[INFO]${NC} $*"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $*"; }
error() { echo -e "${RED}[ERROR]${NC} $*" >&2; }

# Check prerequisites
check_prerequisites() {
    if ! command -v gcloud &> /dev/null; then
        error "gcloud CLI not found. Install from: https://cloud.google.com/sdk/docs/install"
        exit 1
    fi

    if ! command -v cast &> /dev/null; then
        error "cast (Foundry) not found. Install from: https://book.getfoundry.sh"
        exit 1
    fi

    # Check gcloud auth
    if ! gcloud auth print-access-token &> /dev/null; then
        error "Not authenticated with gcloud. Run: gcloud auth login"
        exit 1
    fi
}

# Get image digest from Artifact Registry
get_image_digest() {
    local image=$1
    local full_image="${REGISTRY}/${image}:${TAG}"

    # Get the digest using gcloud (stderr to /dev/null, only output digest)
    local digest
    digest=$(gcloud artifacts docker images describe "${full_image}" \
        --format='get(image_summary.digest)' 2>/dev/null) || {
        echo ""
        return 1
    }

    echo "${digest}"
}

# Compute keccak256 hash of concatenated digests
compute_image_digest_hash() {
    local digests=("$@")
    local combined=""

    for digest in "${digests[@]}"; do
        if [[ -n "$digest" ]]; then
            combined="${combined}${digest}"
        fi
    done

    if [[ -z "$combined" ]]; then
        error "No valid digests found"
        return 1
    fi

    # Compute keccak256 hash
    cast keccak "${combined}"
}

main() {
    info "SyndDB Staging Deployment Setup"
    info "================================"
    echo ""

    check_prerequisites

    # Fetch image digests
    info "Fetching image digests from Artifact Registry..."
    echo ""

    info "  Fetching synddb-sequencer:${TAG}..."
    SEQUENCER_DIGEST=$(get_image_digest "synddb-sequencer")
    [[ -z "$SEQUENCER_DIGEST" ]] && warn "    Image not found"

    info "  Fetching price-oracle-validator:${TAG}..."
    VALIDATOR_DIGEST=$(get_image_digest "price-oracle-validator")
    [[ -z "$VALIDATOR_DIGEST" ]] && warn "    Image not found"

    info "  Fetching price-oracle:${TAG}..."
    PRICE_ORACLE_DIGEST=$(get_image_digest "price-oracle")
    [[ -z "$PRICE_ORACLE_DIGEST" ]] && warn "    Image not found"

    info "  Fetching proof-service:${TAG}..."
    PROOF_SERVICE_DIGEST=$(get_image_digest "proof-service")
    [[ -z "$PROOF_SERVICE_DIGEST" ]] && warn "    Image not found"

    echo ""
    info "Image Digests:"
    echo "  synddb-sequencer:       ${SEQUENCER_DIGEST:-NOT FOUND}"
    echo "  price-oracle-validator: ${VALIDATOR_DIGEST:-NOT FOUND}"
    echo "  price-oracle:           ${PRICE_ORACLE_DIGEST:-NOT FOUND}"
    echo "  proof-service:          ${PROOF_SERVICE_DIGEST:-NOT FOUND}"
    echo ""

    # Compute combined hash for TEE images (sequencer + validator)
    if [[ -n "$SEQUENCER_DIGEST" && -n "$VALIDATOR_DIGEST" ]]; then
        info "Computing EXPECTED_IMAGE_DIGEST_HASH..."
        # For TEE verification, we hash the allowed image digests
        # Each digest is already "sha256:..." format
        IMAGE_HASH=$(compute_image_digest_hash "$SEQUENCER_DIGEST" "$VALIDATOR_DIGEST")
        echo ""
        info "EXPECTED_IMAGE_DIGEST_HASH: ${IMAGE_HASH}"
    else
        warn "Cannot compute image hash - missing digests"
        IMAGE_HASH=""
    fi

    echo ""
    info "=========================================="
    info "Deployment Variables"
    info "=========================================="
    echo ""
    echo "# SP1 Verification Key (from CI - stable unless SP1 program changes)"
    echo "export ATTESTATION_VERIFIER_VKEY=\"0x005d59c275cbbb6fb41f5ba96c0d6505bd09cf154ac890a0e001673c71a05fc7\""
    echo ""
    echo "# Image digest hash (changes when images are rebuilt)"
    echo "export EXPECTED_IMAGE_DIGEST_HASH=\"${IMAGE_HASH}\""
    echo ""
    echo "# Expiration tolerance (24 hours in seconds)"
    echo "export EXPIRATION_TOLERANCE=\"86400\""
    echo ""
    echo "# Individual image digests (for reference)"
    echo "export SEQUENCER_DIGEST=\"${SEQUENCER_DIGEST}\""
    echo "export VALIDATOR_DIGEST=\"${VALIDATOR_DIGEST}\""
    echo "export PRICE_ORACLE_DIGEST=\"${PRICE_ORACLE_DIGEST}\""
    echo ""
    info "=========================================="
    echo ""
    info "Next steps:"
    echo "  1. Export the variables above"
    echo "  2. Deploy contracts: see README.md Step 2"
    echo "  3. Fill in terraform.tfvars with contract addresses"
    echo "  4. Run: terraform init && terraform apply"
}

main "$@"
