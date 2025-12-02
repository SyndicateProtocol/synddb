#!/bin/bash
# Setup GCP infrastructure for Confidential Space attestation testing
#
# This script is idempotent - safe to run multiple times.
#
# Prerequisites:
#   - gcloud CLI installed and authenticated
#   - Sufficient IAM permissions (see docs)
#
# Usage:
#   ./setup-gcp.sh <project-id> [region]
#
# This script creates (if not exists):
#   - Artifact Registry repository for container images
#   - Service account for the workload
#   - GCS bucket for output
#   - Required IAM bindings

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_skip() { echo -e "${YELLOW}[SKIP]${NC} $1 (already exists)"; }
log_create() { echo -e "${GREEN}[CREATE]${NC} $1"; }
log_grant() { echo -e "${GREEN}[GRANT]${NC} $1"; }

if [ $# -lt 1 ]; then
    echo "Usage: $0 <project-id> [region]"
    echo "Example: $0 my-project us-central1"
    exit 1
fi

PROJECT_ID="$1"
REGION="${2:-us-central1}"
ZONE="${REGION}-a"

# Resource names
SERVICE_ACCOUNT_NAME="cs-attestation-workload"
REPO_NAME="synddb-test"
BUCKET_NAME="${PROJECT_ID}-cs-attestation-samples"
IMAGE_NAME="cs-attestation-sample"

echo ""
echo "=== Confidential Space Setup ==="
echo "Project: ${PROJECT_ID}"
echo "Region:  ${REGION}"
echo ""

# Set project
gcloud config set project "${PROJECT_ID}" --quiet

# --- Enable APIs (idempotent by default) ---
log_info "Enabling required APIs..."
gcloud services enable \
    artifactregistry.googleapis.com \
    compute.googleapis.com \
    confidentialcomputing.googleapis.com \
    iamcredentials.googleapis.com \
    storage.googleapis.com \
    --quiet

# --- Artifact Registry ---
log_info "Checking Artifact Registry repository..."
if gcloud artifacts repositories describe "${REPO_NAME}" --location="${REGION}" &>/dev/null; then
    log_skip "Repository ${REPO_NAME}"
else
    gcloud artifacts repositories create "${REPO_NAME}" \
        --repository-format=docker \
        --location="${REGION}" \
        --description="SyndDB Confidential Space test images" \
        --quiet
    log_create "Repository ${REPO_NAME}"
fi

# --- Service Account ---
log_info "Checking service account..."
SA_EMAIL="${SERVICE_ACCOUNT_NAME}@${PROJECT_ID}.iam.gserviceaccount.com"
if gcloud iam service-accounts describe "${SA_EMAIL}" &>/dev/null; then
    log_skip "Service account ${SERVICE_ACCOUNT_NAME}"
else
    gcloud iam service-accounts create "${SERVICE_ACCOUNT_NAME}" \
        --display-name="Confidential Space Attestation Workload" \
        --quiet
    log_create "Service account ${SERVICE_ACCOUNT_NAME}"
fi

# --- IAM Bindings (add-iam-policy-binding is idempotent) ---
log_info "Ensuring IAM bindings..."

# Helper to add binding with status
add_binding() {
    local resource_type="$1"
    local resource="$2"
    local member="$3"
    local role="$4"

    case "$resource_type" in
        project)
            gcloud projects add-iam-policy-binding "${resource}" \
                --member="${member}" \
                --role="${role}" \
                --quiet >/dev/null 2>&1
            ;;
        bucket)
            gcloud storage buckets add-iam-policy-binding "${resource}" \
                --member="${member}" \
                --role="${role}" \
                --quiet >/dev/null 2>&1
            ;;
        repo)
            gcloud artifacts repositories add-iam-policy-binding "${REPO_NAME}" \
                --location="${REGION}" \
                --member="${member}" \
                --role="${role}" \
                --quiet >/dev/null 2>&1
            ;;
        sa)
            gcloud iam service-accounts add-iam-policy-binding "${resource}" \
                --member="${member}" \
                --role="${role}" \
                --quiet >/dev/null 2>&1
            ;;
    esac
    log_grant "${role} on ${resource_type}"
}

add_binding project "${PROJECT_ID}" "serviceAccount:${SA_EMAIL}" "roles/confidentialcomputing.workloadUser"
add_binding project "${PROJECT_ID}" "serviceAccount:${SA_EMAIL}" "roles/logging.logWriter"
add_binding repo "${REPO_NAME}" "serviceAccount:${SA_EMAIL}" "roles/artifactregistry.reader"

# --- GCS Bucket ---
log_info "Checking GCS bucket..."
if gcloud storage buckets describe "gs://${BUCKET_NAME}" &>/dev/null; then
    log_skip "Bucket ${BUCKET_NAME}"
else
    gcloud storage buckets create "gs://${BUCKET_NAME}" \
        --location="${REGION}" \
        --uniform-bucket-level-access \
        --quiet
    log_create "Bucket ${BUCKET_NAME}"
fi

add_binding bucket "gs://${BUCKET_NAME}" "serviceAccount:${SA_EMAIL}" "roles/storage.objectAdmin"

# --- Allow current user to use the service account ---
CURRENT_USER=$(gcloud config get-value account 2>/dev/null)
if [ -n "${CURRENT_USER}" ]; then
    add_binding sa "${SA_EMAIL}" "user:${CURRENT_USER}" "roles/iam.serviceAccountUser"
fi

echo ""
echo "=== Setup Complete ==="
echo ""
echo "Resources:"
echo "  Service Account:    ${SA_EMAIL}"
echo "  Artifact Registry:  ${REGION}-docker.pkg.dev/${PROJECT_ID}/${REPO_NAME}"
echo "  GCS Bucket:         gs://${BUCKET_NAME}"
echo ""
echo "Next step:"
echo "  ./deploy.sh ${PROJECT_ID} ${REGION} --run"
echo ""
