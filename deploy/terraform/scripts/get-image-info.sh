#!/usr/bin/env bash
#
# Fetches the digest, secp256k1 image signature, and RISC Zero image ID
# from OCI artifact referrers.
#
# This script is designed to be used as a Terraform external data source.
# It reads JSON input from stdin and outputs JSON to stdout.
#
# Input:  {"image": "registry/repo:tag"} or {"image": "registry/repo@sha256:..."}
# Output: {"digest": "sha256:...", "signature": "0x...", "image_id": "0x...", "found": "true"}
#         or {"digest": "", "signature": "", "image_id": "", "found": "false"}
#
# If a tag reference is provided, it will be resolved to a digest first.
#
# Requirements:
#   - oras CLI installed (https://oras.land)
#
# Authentication:
#   The synddb registry (us-central1-docker.pkg.dev/synddb-infra/synddb) is
#   public, so no authentication is needed.
#
set -euo pipefail

# Read JSON input from Terraform external data source
INPUT=$(cat)
IMAGE=$(echo "$INPUT" | jq -r '.image // empty')

# Validate input
if [[ -z "$IMAGE" ]]; then
  echo '{"digest": "", "signature": "", "image_id": "", "found": "false", "error": "image parameter is required"}' >&2
  exit 1
fi

# If the image is a tag reference, resolve it to a digest
if [[ "$IMAGE" != *"@sha256:"* ]]; then
  # Extract the base image (without tag) and resolve the digest
  DIGEST=$(oras manifest fetch "$IMAGE" --descriptor 2>/dev/null | jq -r '.digest // empty')
  if [[ -z "$DIGEST" ]]; then
    echo '{"digest": "", "signature": "", "image_id": "", "found": "false", "error": "failed to resolve tag to digest"}'
    exit 0
  fi
  # Construct the digest reference
  BASE_IMAGE="${IMAGE%:*}"
  IMAGE="$BASE_IMAGE@$DIGEST"
else
  # Extract digest from the image reference
  DIGEST="${IMAGE##*@}"
fi

# Extract base image (without digest) for referrer references
BASE_IMAGE="${IMAGE%@*}"

# Create temp directory for pulling artifacts
TMPDIR=$(mktemp -d)
trap "rm -rf $TMPDIR" EXIT

# Initialize output values
SIGNATURE=""
IMAGE_ID=""
FOUND="false"

# --- Fetch signature artifact ---
SIG_ARTIFACT_TYPE="application/vnd.syndicate.image-signature.v1+json"
SIG_REFERRERS=$(oras discover "$IMAGE" \
  --artifact-type "$SIG_ARTIFACT_TYPE" \
  --format json 2>/dev/null || echo '{"referrers":[]}')

SIG_MANIFEST_DIGEST=$(echo "$SIG_REFERRERS" | jq -r '.referrers[0].digest // empty')

if [[ -n "$SIG_MANIFEST_DIGEST" ]]; then
  SIG_REF="$BASE_IMAGE@$SIG_MANIFEST_DIGEST"
  mkdir -p "$TMPDIR/sig"
  if oras pull "$SIG_REF" -o "$TMPDIR/sig" --no-tty >/dev/null 2>&1; then
    if [[ -f "$TMPDIR/sig/signature.json" ]]; then
      SIGNATURE=$(jq -r '.signature // empty' "$TMPDIR/sig/signature.json")
      if [[ -n "$SIGNATURE" ]]; then
        FOUND="true"
      fi
    fi
  fi
fi

# --- Fetch RISC Zero image ID artifact (only attached to proof-service) ---
RISC0_ARTIFACT_TYPE="application/vnd.syndicate.risc0-image-id.v1+json"
RISC0_REFERRERS=$(oras discover "$IMAGE" \
  --artifact-type "$RISC0_ARTIFACT_TYPE" \
  --format json 2>/dev/null || echo '{"referrers":[]}')

RISC0_MANIFEST_DIGEST=$(echo "$RISC0_REFERRERS" | jq -r '.referrers[0].digest // empty')

if [[ -n "$RISC0_MANIFEST_DIGEST" ]]; then
  RISC0_REF="$BASE_IMAGE@$RISC0_MANIFEST_DIGEST"
  mkdir -p "$TMPDIR/risc0"
  if oras pull "$RISC0_REF" -o "$TMPDIR/risc0" --no-tty >/dev/null 2>&1; then
    if [[ -f "$TMPDIR/risc0/image_id.json" ]]; then
      IMAGE_ID=$(jq -r '.imageId // empty' "$TMPDIR/risc0/image_id.json")
      if [[ -n "$IMAGE_ID" ]]; then
        FOUND="true"
      fi
    fi
  fi
fi

# Output JSON with all collected info
jq -n \
  --arg digest "$DIGEST" \
  --arg signature "$SIGNATURE" \
  --arg image_id "$IMAGE_ID" \
  --arg found "$FOUND" \
  '{"digest": $digest, "signature": $signature, "image_id": $image_id, "found": $found}'
