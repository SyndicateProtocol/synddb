#!/bin/bash
# End-to-end test for the price oracle example
# This script tests the full workflow with SyndDB replication

set -e

CONTAINER="price-oracle-price-oracle-1"

echo "=== Price Oracle E2E Test ==="

# Helper function to run commands in the container
run() {
    docker exec "$CONTAINER" python -m app.main --db /data/price-oracle.db --sequencer-url http://sequencer:8433 "$@"
}

# Step 1: Initialize the database
echo ""
echo "Step 1: Initialize database"
run init
echo "  Database initialized"

# Step 2: List tracked assets
echo ""
echo "Step 2: List tracked assets"
run assets

# Step 3: Add an asset
echo ""
echo "Step 3: Add tracked asset"
run add-asset solana SOL
echo "  Added asset: solana (SOL)"

# Step 4: Fetch prices using mock APIs
echo ""
echo "Step 4: Fetch prices (mock APIs)"
run fetch --mock
echo "  Fetched prices from mock APIs"

# Step 5: Check recent prices
echo ""
echo "Step 5: Check recent prices"
run prices --limit 10
echo "  Retrieved recent prices"

# Step 6: Fetch again to test updates
echo ""
echo "Step 6: Fetch prices again"
run fetch --mock
echo "  Second price fetch completed"

# Step 7: Compare prices for bitcoin
echo ""
echo "Step 7: Compare prices for bitcoin"
run compare bitcoin
echo "  Retrieved price comparison for bitcoin"

# Step 8: List assets to verify solana was added
echo ""
echo "Step 8: Verify assets list"
run assets

# Step 9: Check sequencer received messages
echo ""
echo "Step 9: Verify sequencer received changesets"
SEQUENCE=$(curl -sf http://localhost:8433/sequence)
echo "  Current sequencer sequence: $SEQUENCE"

if [ "$SEQUENCE" -lt 1 ]; then
    echo "  ERROR: No changesets were published to sequencer"
    exit 1
fi

echo ""
echo "=== All E2E tests passed ==="
