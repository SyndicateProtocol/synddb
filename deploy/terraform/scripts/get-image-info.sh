#!/usr/bin/env bash
#
# Fetches the digest and secp256k1 image signature from OCI artifact referrers.
#
# This script is designed to be used as a Terraform external data source.
# It reads JSON input from stdin and outputs JSON to stdout.
#
# Input:  {"image": "registry/repo:tag"} or {"image": "registry/repo@sha256:..."}
# Output: {"digest": "sha256:...", "signature": "0x...", "found": "true"}
#         or {"digest": "", "signature": "", "found": "false"}
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
  echo '{"digest": "", "signature": "", "found": "false", "error": "image parameter is required"}' >&2
  exit 1
fi

# If the image is a tag reference, resolve it to a digest
if [[ "$IMAGE" != *"@sha256:"* ]]; then
  # Extract the base image (without tag) and resolve the digest
  DIGEST=$(oras manifest fetch "$IMAGE" --descriptor 2>/dev/null | jq -r '.digest // empty')
  if [[ -z "$DIGEST" ]]; then
    echo '{"digest": "", "signature": "", "found": "false", "error": "failed to resolve tag to digest"}'
    exit 0
  fi
  # Construct the digest reference
  BASE_IMAGE="${IMAGE%:*}"
  IMAGE="$BASE_IMAGE@$DIGEST"
else
  # Extract digest from the image reference
  DIGEST="${IMAGE##*@}"
fi

# Discover signature artifacts attached to this image
# The artifact type matches what we attach in CI
ARTIFACT_TYPE="application/vnd.syndicate.image-signature.v1+json"

REFERRERS=$(oras discover "$IMAGE" \
  --artifact-type "$ARTIFACT_TYPE" \
  --format json 2>/dev/null || echo '{"referrers":[]}')

# Check if we found any signatures
MANIFEST_DIGEST=$(echo "$REFERRERS" | jq -r '.referrers[0].digest // empty')

if [[ -z "$MANIFEST_DIGEST" ]]; then
  # No signature found, but we have the digest
  jq -n --arg digest "$DIGEST" '{"digest": $digest, "signature": "", "found": "false"}'
  exit 0
fi

# Extract base image (without digest) to form the referrer reference
BASE_IMAGE="${IMAGE%@*}"
REFERRER_REF="$BASE_IMAGE@$MANIFEST_DIGEST"

# Create temp directory for pulling artifact
TMPDIR=$(mktemp -d)
trap "rm -rf $TMPDIR" EXIT

# Pull the signature artifact (suppress output)
if ! oras pull "$REFERRER_REF" -o "$TMPDIR" --no-tty >/dev/null 2>&1; then
  jq -n --arg digest "$DIGEST" '{"digest": $digest, "signature": "", "found": "false", "error": "failed to pull signature artifact"}'
  exit 0
fi

# Read signature from the pulled artifact
if [[ -f "$TMPDIR/signature.json" ]]; then
  SIGNATURE=$(jq -r '.signature // empty' "$TMPDIR/signature.json")
  if [[ -n "$SIGNATURE" ]]; then
    # Output valid JSON with the digest and signature
    jq -n --arg digest "$DIGEST" --arg sig "$SIGNATURE" '{"digest": $digest, "signature": $sig, "found": "true"}'
  else
    jq -n --arg digest "$DIGEST" '{"digest": $digest, "signature": "", "found": "false", "error": "signature field missing in artifact"}'
  fi
else
  jq -n --arg digest "$DIGEST" '{"digest": $digest, "signature": "", "found": "false", "error": "signature.json not found in artifact"}'
fi
