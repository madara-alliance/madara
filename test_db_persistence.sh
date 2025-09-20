#!/bin/bash

# Test script to check if database properly persists head status

set -e

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "=== Testing Database Head Status Persistence ==="

# Database path
TEST_DB="/tmp/madara_test_db_$(date +%s)"
MADARA="/Volumes/itsparser/rust-targets/release/madara"

echo -e "${YELLOW}Using test database at: $TEST_DB${NC}"

# Cleanup function
cleanup() {
    echo -e "${YELLOW}Cleaning up...${NC}"
    pkill -f "madara.*test_persistence" 2>/dev/null || true
    sleep 2
    rm -rf "$TEST_DB"
}

trap cleanup EXIT

# Start madara and let it sync a few blocks
echo -e "${GREEN}Step 1: Starting Madara to sync blocks...${NC}"
timeout 30 $MADARA \
    --name test_persistence \
    --devnet \
    --base-path "$TEST_DB" \
    --rpc-port 9876 \
    --feeder-gateway-enable \
    --gateway-port 8765 \
    --gateway-enable \
    --gateway-external \
    --chain-config-override chain_id=TEST_CHAIN \
    --chain-config-override block_time=1s \
    2>&1 | tee /tmp/madara_test_first_run.log || true

echo -e "${GREEN}Step 2: Checking what blocks were synced...${NC}"
LAST_BLOCK=$(grep "fully imported" /tmp/madara_test_first_run.log | tail -1 | grep -o "Block #[0-9]*" | grep -o "[0-9]*" || echo "0")
echo "Last fully imported block: $LAST_BLOCK"

if [ "$LAST_BLOCK" == "0" ] || [ -z "$LAST_BLOCK" ]; then
    echo -e "${RED}No blocks were synced in first run!${NC}"
    exit 1
fi

echo -e "${GREEN}Step 3: Restarting Madara to check if it remembers the last block...${NC}"
timeout 5 $MADARA \
    --name test_persistence \
    --base-path "$TEST_DB" \
    --rpc-port 9876 \
    --full \
    --preset devnet \
    --chain-config-override chain_id=TEST_CHAIN \
    --no-l1-sync \
    --gateway-url http://localhost:8080 \
    2>&1 | tee /tmp/madara_test_second_run.log || true

echo -e "${GREEN}Step 4: Checking if database loaded the head status...${NC}"
if grep "latest_full_block=None" /tmp/madara_test_second_run.log; then
    echo -e "${RED}❌ FAILED: Database did not persist head status!${NC}"
    echo "Expected latest_full_block=Some($LAST_BLOCK), got None"
    exit 1
elif grep "latest_full_block=Some($LAST_BLOCK)" /tmp/madara_test_second_run.log; then
    echo -e "${GREEN}✓ SUCCESS: Database correctly loaded head status!${NC}"
    echo "Database remembered it had synced up to block $LAST_BLOCK"
    exit 0
else
    echo -e "${YELLOW}Could not determine head status from logs${NC}"
    echo "Check /tmp/madara_test_second_run.log for details"
    grep "latest_full_block" /tmp/madara_test_second_run.log || echo "No head status log found"
    exit 1
fi