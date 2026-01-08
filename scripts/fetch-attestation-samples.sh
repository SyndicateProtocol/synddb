#!/usr/bin/env bash
#
# Fetch attestation samples from Cloud Logging and save to proof-service test-data
#
# Usage:
#   ./scripts/fetch-attestation-samples.sh <project-id> [limit]
#
# Example:
#   ./scripts/fetch-attestation-samples.sh synddb-infra 5
#
# Prerequisites:
#   - gcloud CLI authenticated
#   - jq installed
#   - base64 command available

set -euo pipefail

PROJECT_ID="${1:-}"
LIMIT="${2:-5}"
OUTPUT_DIR="crates/proof-service/test-data"

if [[ -z "$PROJECT_ID" ]]; then
    echo "Usage: $0 <project-id> [limit]"
    echo "Example: $0 synddb-infra 5"
    exit 1
fi

# Check prerequisites
command -v gcloud >/dev/null 2>&1 || { echo "gcloud CLI required"; exit 1; }
command -v jq >/dev/null 2>&1 || { echo "jq required"; exit 1; }

echo "Fetching attestation samples from project: $PROJECT_ID (limit: $LIMIT)"

# Query Cloud Logging for attestation samples from proof_service (has JWK data)
LOGS=$(gcloud logging read \
    'jsonPayload.event="attestation_sample" AND jsonPayload.source="proof_service"' \
    --project="$PROJECT_ID" \
    --format=json \
    --limit="$LIMIT" \
    2>/dev/null)

if [[ -z "$LOGS" || "$LOGS" == "[]" ]]; then
    echo "No attestation samples found in logs"
    exit 1
fi

SAMPLE_COUNT=$(echo "$LOGS" | jq 'length')
echo "Found $SAMPLE_COUNT sample(s)"

# Generate output filename with timestamp
TIMESTAMP=$(date +%Y%m%d_%H_%M_%S)
OUTPUT_FILE="$OUTPUT_DIR/samples_${TIMESTAMP}.json"

# Process logs into sample format
# The log entries have: raw_token, jwk_kid, jwk_n, jwk_e, audience
echo "$LOGS" | jq '
# Extract unique JWKs by kid
def extract_jwks:
  [.[] | {
    kid: .jsonPayload.jwk_kid,
    n: .jsonPayload.jwk_n,
    e: .jsonPayload.jwk_e,
    kty: "RSA",
    alg: "RS256",
    use: "sig"
  }] | unique_by(.kid);

# Decode base64url to get header/claims
def decode_jwt_part:
  # Add padding if needed and convert base64url to base64
  (. + "==") | gsub("-"; "+") | gsub("_"; "/") | @base64d | fromjson;

# Process each log entry into a sample
def process_sample:
  .jsonPayload as $payload |
  ($payload.raw_token | split(".")) as $parts |
  {
    raw_token: $payload.raw_token,
    header: ($parts[0] | decode_jwt_part),
    claims: ($parts[1] | decode_jwt_part),
    audience: $payload.audience
  };

{
  samples: [.[] | process_sample],
  jwks: {
    keys: extract_jwks
  },
  metadata: {
    captured_from: "cloud_logging",
    project_id: "'"$PROJECT_ID"'",
    captured_at: now | todate,
    sample_count: length
  }
}
' > "$OUTPUT_FILE"

echo "Saved to: $OUTPUT_FILE"
echo ""
echo "Sample summary:"
jq '.samples | length' "$OUTPUT_FILE" | xargs -I{} echo "  Samples: {}"
jq '.jwks.keys | length' "$OUTPUT_FILE" | xargs -I{} echo "  JWKs: {}"
jq '.samples[0].claims.aud // "N/A"' "$OUTPUT_FILE" | xargs -I{} echo "  Audience: {}"
