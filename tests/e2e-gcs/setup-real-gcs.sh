#!/usr/bin/env bash
set -euo pipefail

# Real GCS Setup Script for E2E Tests
# This script is idempotent - safe to run multiple times

PROJECT_ID="${1:-synd-db-testing}"
BUCKET_NAME="synddb-e2e-test"
SERVICE_ACCOUNT_NAME="synddb-e2e-test"
SERVICE_ACCOUNT_EMAIL="${SERVICE_ACCOUNT_NAME}@${PROJECT_ID}.iam.gserviceaccount.com"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CREDENTIALS_DIR="${SCRIPT_DIR}/.credentials"
KEY_FILE="${CREDENTIALS_DIR}/service-account.json"

echo "=== Real GCS Setup for E2E Tests ==="
echo ""
echo "Project:         ${PROJECT_ID}"
echo "Bucket:          ${BUCKET_NAME}"
echo "Service Account: ${SERVICE_ACCOUNT_EMAIL}"
echo ""

# Check gcloud is installed and authenticated
if ! command -v gcloud &> /dev/null; then
    echo "Error: gcloud CLI not found. Install from https://cloud.google.com/sdk/docs/install"
    exit 1
fi

# Set project
echo "Setting project to ${PROJECT_ID}..."
gcloud config set project "${PROJECT_ID}" --quiet

# Create bucket if it doesn't exist
echo ""
echo "Checking bucket..."
if gcloud storage buckets describe "gs://${BUCKET_NAME}" &> /dev/null; then
    echo "  Bucket gs://${BUCKET_NAME} already exists"
else
    echo "  Creating bucket gs://${BUCKET_NAME}..."
    gcloud storage buckets create "gs://${BUCKET_NAME}" \
        --location=us-central1 \
        --uniform-bucket-level-access
    echo "  Bucket created"
fi

# Set lifecycle policy to auto-delete old test data (7 days)
echo ""
echo "Setting lifecycle policy (auto-delete after 7 days)..."
cat > /tmp/lifecycle.json << 'EOF'
{
  "rule": [
    {
      "action": {"type": "Delete"},
      "condition": {"age": 7}
    }
  ]
}
EOF
gcloud storage buckets update "gs://${BUCKET_NAME}" --lifecycle-file=/tmp/lifecycle.json --quiet
rm /tmp/lifecycle.json
echo "  Lifecycle policy set"

# Create service account if it doesn't exist
echo ""
echo "Checking service account..."
if gcloud iam service-accounts describe "${SERVICE_ACCOUNT_EMAIL}" &> /dev/null; then
    echo "  Service account ${SERVICE_ACCOUNT_NAME} already exists"
else
    echo "  Creating service account ${SERVICE_ACCOUNT_NAME}..."
    gcloud iam service-accounts create "${SERVICE_ACCOUNT_NAME}" \
        --display-name="SyndDB E2E Test"
    echo "  Service account created"
    echo "  Waiting for propagation..."
    sleep 10
fi

# Grant storage permissions on the bucket
echo ""
echo "Granting storage permissions..."
gcloud storage buckets add-iam-policy-binding "gs://${BUCKET_NAME}" \
    --member="serviceAccount:${SERVICE_ACCOUNT_EMAIL}" \
    --role="roles/storage.objectAdmin" \
    --quiet 2>/dev/null || true
echo "  Permissions granted"

# Create credentials directory
mkdir -p "${CREDENTIALS_DIR}"

# Create service account key if it doesn't exist
echo ""
echo "Checking credentials..."
if [[ -f "${KEY_FILE}" ]]; then
    echo "  Credentials file already exists at ${KEY_FILE}"
    echo "  To regenerate, delete the file and run this script again"
else
    echo "  Creating service account key..."
    gcloud iam service-accounts keys create "${KEY_FILE}" \
        --iam-account="${SERVICE_ACCOUNT_EMAIL}"
    echo "  Key saved to ${KEY_FILE}"
fi

echo ""
echo "=== Setup Complete ==="
echo ""
echo "To run the real GCS E2E test:"
echo ""
echo "  REAL_GCS=true \\"
echo "  GCS_BUCKET=${BUCKET_NAME} \\"
echo "  GOOGLE_APPLICATION_CREDENTIALS=${KEY_FILE} \\"
echo "    cargo test -p synddb-e2e-gcs"
echo ""
