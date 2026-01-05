#!/usr/bin/env bash
#
# Fetches the secp256k1 image signature from OCI artifact referrers.
#
# This script is designed to be used as a Terraform external data source.
# It reads JSON input from stdin and outputs JSON to stdout.
#
# Input:  {"image": "registry/repo@sha256:..."}
# Output: {"signature": "0x...", "found": "true"} or {"signature": "", "found": "false"}
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
  echo '{"signature": "", "found": "false", "error": "image parameter is required"}' >&2
  exit 1
fi

# Image must be a digest reference
if [[ "$IMAGE" != *"@sha256:"* ]]; then
  echo '{"signature": "", "found": "false", "error": "image must be a digest reference"}' >&2
  exit 1
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
  # No signature found
  echo '{"signature": "", "found": "false"}'
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
  echo '{"signature": "", "found": "false", "error": "failed to pull signature artifact"}'
  exit 0
fi

# Read signature from the pulled artifact
if [[ -f "$TMPDIR/signature.json" ]]; then
  SIGNATURE=$(jq -r '.signature // empty' "$TMPDIR/signature.json")
  if [[ -n "$SIGNATURE" ]]; then
    # Output valid JSON with the signature
    jq -n --arg sig "$SIGNATURE" '{"signature": $sig, "found": "true"}'
  else
    echo '{"signature": "", "found": "false", "error": "signature field missing in artifact"}'
  fi
else
  echo '{"signature": "", "found": "false", "error": "signature.json not found in artifact"}'
fi
