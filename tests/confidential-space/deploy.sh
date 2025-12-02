#!/bin/bash
# Build, push, and deploy the attestation sample workload to Confidential Space
#
# This script is idempotent - safe to run multiple times.
#
# Usage:
#   ./deploy.sh <project-id> [region] [--run] [--no-cache]
#
# Options:
#   --run        Also create and start the Confidential VM after pushing
#   --no-cache   Force Docker to rebuild without cache
#
# The script will:
#   1. Configure Docker authentication (if needed)
#   2. Build the Docker image
#   3. Push to Artifact Registry
#   4. (Optional) Create/recreate a Confidential VM to run the workload

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_step() { echo -e "${BLUE}[STEP]${NC} $1"; }
log_skip() { echo -e "${YELLOW}[SKIP]${NC} $1"; }

if [ $# -lt 1 ]; then
    echo "Usage: $0 <project-id> [region] [--run] [--no-cache]"
    echo ""
    echo "Options:"
    echo "  --run        Create and start a Confidential VM after pushing"
    echo "  --no-cache   Force Docker rebuild without cache"
    echo ""
    echo "Examples:"
    echo "  $0 my-project                      # Build and push only"
    echo "  $0 my-project us-central1 --run    # Build, push, and run VM"
    exit 1
fi

PROJECT_ID="$1"
shift

# Parse remaining arguments
REGION="us-central1"
RUN_VM=false
DOCKER_CACHE=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --run)
            RUN_VM=true
            shift
            ;;
        --no-cache)
            DOCKER_CACHE="--no-cache"
            shift
            ;;
        *)
            # Assume it's the region if it doesn't start with --
            if [[ ! "$1" =~ ^-- ]]; then
                REGION="$1"
            fi
            shift
            ;;
    esac
done

ZONE="${REGION}-a"
REPO_NAME="synddb-test"
IMAGE_NAME="cs-attestation-sample"
SERVICE_ACCOUNT_NAME="cs-attestation-workload"
VM_NAME="cs-attestation-vm"
BUCKET_NAME="${PROJECT_ID}-cs-attestation-samples"

IMAGE_URI="${REGION}-docker.pkg.dev/${PROJECT_ID}/${REPO_NAME}/${IMAGE_NAME}"
SA_EMAIL="${SERVICE_ACCOUNT_NAME}@${PROJECT_ID}.iam.gserviceaccount.com"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo ""
echo "=== Confidential Space Deployment ==="
echo "Project: ${PROJECT_ID}"
echo "Region:  ${REGION}"
echo "Image:   ${IMAGE_URI}"
echo "Run VM:  ${RUN_VM}"
echo ""

# --- Configure Docker authentication ---
log_step "Configuring Docker authentication..."
# Check if already configured by looking for the credential helper
if grep -q "${REGION}-docker.pkg.dev" ~/.docker/config.json 2>/dev/null; then
    log_skip "Docker already configured for ${REGION}-docker.pkg.dev"
else
    gcloud auth configure-docker "${REGION}-docker.pkg.dev" --quiet
    log_info "Docker configured for ${REGION}-docker.pkg.dev"
fi

# --- Build the image ---
log_step "Building Docker image..."
docker build ${DOCKER_CACHE} -t "${IMAGE_URI}:latest" "${SCRIPT_DIR}"

# --- Push the image ---
log_step "Pushing to Artifact Registry..."
docker push "${IMAGE_URI}:latest"

# --- Get the image digest ---
log_step "Getting image digest..."
DIGEST=$(docker inspect --format='{{index .RepoDigests 0}}' "${IMAGE_URI}:latest" 2>/dev/null | cut -d'@' -f2 || echo "")

if [ -z "${DIGEST}" ]; then
    # Fallback: get from registry
    DIGEST=$(gcloud artifacts docker images describe "${IMAGE_URI}:latest" --format='value(image_summary.digest)' 2>/dev/null || echo "unknown")
fi

echo "Image digest: ${DIGEST}"

# Save the digest to a file for reference
echo "${DIGEST}" > "${SCRIPT_DIR}/.image-digest"
log_info "Digest saved to ${SCRIPT_DIR}/.image-digest"

if [ "$RUN_VM" = true ]; then
    echo ""
    log_step "Managing Confidential VM..."

    # Check if VM exists
    if gcloud compute instances describe "${VM_NAME}" --zone="${ZONE}" --project="${PROJECT_ID}" &>/dev/null; then
        log_warn "VM ${VM_NAME} already exists, deleting..."
        gcloud compute instances delete "${VM_NAME}" \
            --zone="${ZONE}" \
            --project="${PROJECT_ID}" \
            --quiet
        log_info "Deleted existing VM"
        # Wait a moment for cleanup
        sleep 5
    fi

    log_step "Creating Confidential VM..."
    # Using debug image for easier troubleshooting (has SSH access and verbose logging)
    # n2d-standard-2 is the smallest available for Confidential VMs (AMD SEV)
    # Using spot instance to reduce cost (~60-90% cheaper, may be preempted)
    gcloud compute instances create "${VM_NAME}" \
        --project="${PROJECT_ID}" \
        --zone="${ZONE}" \
        --machine-type="n2d-standard-2" \
        --provisioning-model=SPOT \
        --instance-termination-action=DELETE \
        --confidential-compute-type="SEV" \
        --shielded-secure-boot \
        --scopes="cloud-platform" \
        --image-project="confidential-space-images" \
        --image-family="confidential-space-debug" \
        --service-account="${SA_EMAIL}" \
        --metadata="^~^tee-image-reference=${IMAGE_URI}:latest~tee-container-log-redirect=true~tee-env-ATTESTATION_AUDIENCE=https://synddb-sequencer.example.com~tee-env-OUTPUT_BUCKET=${BUCKET_NAME}"

    log_info "VM created successfully"

    echo ""
    echo "=== Workload Running ==="
    echo ""
    echo "The workload should complete in ~1-2 minutes."
    echo ""
    echo "Tail logs (real-time):"
    echo "  gcloud compute instances tail-serial-port-output ${VM_NAME} \\"
    echo "    --zone=${ZONE} --project=${PROJECT_ID}"
    echo ""
    echo "Download attestation samples:"
    echo "  mkdir -p samples"
    echo "  gcloud storage cp 'gs://${BUCKET_NAME}/attestation-samples/*' ./samples/"
    echo ""
    echo "Delete VM when done:"
    echo "  gcloud compute instances delete ${VM_NAME} --zone=${ZONE} --project=${PROJECT_ID} --quiet"
    echo ""

else
    echo ""
    echo "=== Build Complete ==="
    echo ""
    echo "Image: ${IMAGE_URI}:latest"
    echo "Digest: ${DIGEST}"
    echo ""
    echo "To run the workload:"
    echo "  ./deploy.sh ${PROJECT_ID} ${REGION} --run"
    echo ""
fi
