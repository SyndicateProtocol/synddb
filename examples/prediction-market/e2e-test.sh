#!/bin/bash
# End-to-end test for the prediction market example
# This script tests the full workflow with SyndDB replication

set -e

CONTAINER="prediction-market-prediction-market-1"

echo "=== Prediction Market E2E Test ==="

# Helper function to run commands in the container
run() {
    docker exec "$CONTAINER" prediction-market --db /data/market.db --sequencer http://sequencer:8433 "$@"
}

# Step 1: Initialize the database
echo ""
echo "Step 1: Initialize database"
run init
echo "  Database initialized"

# Step 2: Create user accounts
echo ""
echo "Step 2: Create user accounts"
run create-account alice
run create-account bob
echo "  Created accounts: alice, bob"

# Step 3: Create a prediction market
echo ""
echo "Step 3: Create prediction market"
RESOLUTION_TIME=$(($(date +%s) + 86400))  # 24 hours from now
run create-market "Will it rain tomorrow?" --resolution-time "$RESOLUTION_TIME" --description "Test market"
echo "  Created market: 'Will it rain tomorrow?'"

# Step 4: Fund accounts via simulated deposits
echo ""
echo "Step 4: Fund user accounts via simulated deposits"
run simulate-deposit --tx-hash 0x1234 --from 0xABCD --to alice --amount 100000
run simulate-deposit --tx-hash 0x5678 --from 0xEFGH --to bob --amount 50000
echo "  Simulated deposits: alice 1000.00, bob 500.00"

# Step 5: Process the deposits
echo ""
echo "Step 5: Process pending deposits"
run process-deposits
echo "  Deposits processed"

# Step 6: Buy shares
echo ""
echo "Step 6: Buy shares"
run buy --account 1 --market 1 --outcome yes --shares 100
run buy --account 2 --market 1 --outcome no --shares 50
echo "  alice bought 100 YES shares, bob bought 50 NO shares"

# Step 7: Check status
echo ""
echo "Step 7: Check status"
run status

# Step 8: Sell shares
echo ""
echo "Step 8: Sell shares"
run sell --account 1 --market 1 --outcome yes --shares 50
echo "  alice sold 50 YES shares"

# Step 9: Final status check
echo ""
echo "Step 9: Final status"
run status

# Step 10: Check sequencer received messages
echo ""
echo "Step 10: Verify sequencer received changesets"
SEQUENCE=$(curl -sf http://localhost:8433/sequence)
echo "  Current sequencer sequence: $SEQUENCE"

if [ "$SEQUENCE" -lt 1 ]; then
    echo "  ERROR: No changesets were published to sequencer"
    exit 1
fi

echo ""
echo "=== All E2E tests passed ==="
