#!/bin/bash

set -e

echo "=== Madara Reorg Test ==="
echo "This test will demonstrate reorg detection and handling"
echo "Enhanced logging enabled for common ancestor finding"
echo

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Binary path
MADARA="/Volumes/itsparser/rust-targets/release/madara"

MADARA_DB="/Volumes/itsparser/cache/madara/db"

# Database directories
SEQ1_DB="$MADARA_DB/seq1_db"
SEQ2_DB="$MADARA_DB/seq2_db"
FULLNODE_DB="$MADARA_DB/fullnode_db"

# Log files
SEQ1_LOG="$MADARA_DB/seq1.log"
SEQ2_LOG="$MADARA_DB/seq2.log"
FULLNODE_LOG="$MADARA_DB/fullnode.log"

# Clean up function
cleanup() {
    echo -e "${YELLOW}Cleaning up processes...${NC}"
    pkill -f madara 2>/dev/null || true
    sleep 2
}

# Trap to ensure cleanup on exit
trap cleanup EXIT

# Initial cleanup
cleanup
echo -e "${YELLOW}Removing old databases and logs...${NC}"
rm -rf "$SEQ1_DB" "$SEQ2_DB" "$FULLNODE_DB"
rm -f "$SEQ1_LOG" "$SEQ2_LOG" "$FULLNODE_LOG"

echo -e "${GREEN}Step 1: Starting Sequencer 1 (Primary chain)${NC}"
echo "This will create the 'canonical' chain"
RUST_LOG=info,mc_sync=debug,mc_import=debug $MADARA \
    --name sequencer1 \
    --devnet \
    --base-path "$SEQ1_DB" \
    --rpc-port 9944 \
    --feeder-gateway-enable \
    --gateway-port 8080 \
    --gateway-enable \
    --gateway-external \
    --chain-config-override chain_id=MADARA_CHAIN \
    --chain-config-override block_time=2s \
    2>&1 | tee "$SEQ1_LOG" &
SEQ1_PID=$!

echo "Waiting for Sequencer 1 to start..."
sleep 5

# Check if gateway is working
echo "Verifying Sequencer 1 gateway..."
curl -s "http://localhost:8080/feeder_gateway/get_block?blockNumber=latest" | head -1
echo

echo -e "${GREEN}Step 2: Starting Sequencer 2 (Divergent chain)${NC}"
echo "This will create a different chain that diverges from Sequencer 1"
RUST_LOG=info,mc_sync=debug,mc_import=debug $MADARA \
    --name sequencer2 \
    --devnet \
    --base-path "$SEQ2_DB" \
    --rpc-port 9945 \
    --feeder-gateway-enable \
    --gateway-port 8081 \
    --gateway-enable \
    --gateway-external \
    --chain-config-override chain_id=MADARA_CHAIN \
    --chain-config-override block_time=3s \
    2>&1 | tee "$SEQ2_LOG" &
SEQ2_PID=$!

echo "Waiting for Sequencer 2 to start..."
sleep 5


# Make sure the process is really dead
if ps -p $FULLNODE_PID > /dev/null 2>&1; then
    echo "Process still running, force killing..."
    kill -9 $FULLNODE_PID 2>/dev/null || true
fi

echo "Waiting for full node to fully shutdown and release ports..."
sleep 20

# Ensure the database lock is released
if [ -f "$FULLNODE_DB/db/LOCK" ]; then
    echo "Removing stale database lock file..."
    rm -f "$FULLNODE_DB/db/LOCK"
fi

# Check if port 4444 is still in use and kill the process using it
if lsof -i:4444 > /dev/null 2>&1; then
    echo "Port 4444 still in use, finding and killing the process..."
    # Find the PID using port 4444 and kill it
    PORT_PID=$(lsof -ti:4444)
    if [ ! -z "$PORT_PID" ]; then
        echo "Killing process $PORT_PID using port 4444..."
        kill -9 $PORT_PID 2>/dev/null || true
        sleep 2
    fi
fi

# Double-check the port is free
if lsof -i:4444 > /dev/null 2>&1; then
    echo "ERROR: Port 4444 is still in use after cleanup!"
    echo "Processes using port 4444:"
    lsof -i:4444
    exit 1
fi

echo -e "${GREEN}Step 3: Starting Full Node syncing from Sequencer 1${NC}"
echo "Full node will initially sync from Sequencer 1's gateway"
RUST_LOG=info,mc_sync=debug,mc_import=debug $MADARA \
    --name fullnode \
    --base-path "$FULLNODE_DB" \
    --rpc-port 4444 \
    --full \
    --preset devnet \
    --chain-config-override chain_id=MADARA_CHAIN \
    --no-l1-sync \
    --gateway-url http://localhost:8080 \
    2>&1 | tee "$FULLNODE_LOG" &
FULLNODE_PID=$!

echo "Letting full node sync from Sequencer 1 for 15 seconds..."
sleep 15

# Check sync status
echo -e "${YELLOW}Checking full node sync status from Sequencer 1:${NC}"
tail -5 "$FULLNODE_LOG" | grep -E "(Blocks:|Sync is at)" || true
echo

echo -e "${GREEN}Step 4: Stopping full node to switch gateways${NC}"
echo "Killing fullnode process (PID: $FULLNODE_PID)..."
kill $FULLNODE_PID 2>/dev/null || true
sleep 2

# Make sure the process is really dead
if ps -p $FULLNODE_PID > /dev/null 2>&1; then
    echo "Process still running, force killing..."
    kill -9 $FULLNODE_PID 2>/dev/null || true
fi

echo "Waiting for full node to fully shutdown and release ports..."
sleep 20

# Ensure the database lock is released
if [ -f "$FULLNODE_DB/db/LOCK" ]; then
    echo "Removing stale database lock file..."
    rm -f "$FULLNODE_DB/db/LOCK"
fi

# Check if port 4444 is still in use and kill the process using it
if lsof -i:4444 > /dev/null 2>&1; then
    echo "Port 4444 still in use, finding and killing the process..."
    # Find the PID using port 4444 and kill it
    PORT_PID=$(lsof -ti:4444)
    if [ ! -z "$PORT_PID" ]; then
        echo "Killing process $PORT_PID using port 4444..."
        kill -9 $PORT_PID 2>/dev/null || true
        sleep 2
    fi
fi

# Double-check the port is free
if lsof -i:4444 > /dev/null 2>&1; then
    echo "ERROR: Port 4444 is still in use after cleanup!"
    echo "Processes using port 4444:"
    lsof -i:4444
    exit 1
fi

echo -e "${GREEN}Step 5: Restarting full node to sync from Sequencer 2${NC}"
echo "This should trigger reorg detection as chains have diverged"
RUST_LOG=info,mc_sync=debug,mc_import=debug $MADARA \
    --name fullnode \
    --base-path "$FULLNODE_DB" \
    --rpc-port 4444 \
    --full \
    --preset devnet \
    --chain-config-override chain_id=MADARA_CHAIN \
    --no-l1-sync \
    --gateway-url http://localhost:8081 \
    2>&1 | tee -a "$FULLNODE_LOG" &
FULLNODE_PID=$!

echo "Waiting for reorg detection (10 seconds)..."
sleep 10

echo -e "${YELLOW}=== Checking for Reorg Detection ===${NC}"
echo "Looking for reorg-related messages in the log:"
grep -i "reorg\|parent\|ancestor\|probe\|Expected parent_hash" "$FULLNODE_LOG" | tail -20 || echo "No reorg messages found yet"

echo
echo -e "${YELLOW}=== Enhanced Common Ancestor Finding Logs ===${NC}"
echo "Looking for detailed common ancestor search:"
grep -E "🔍|📖|🌐|Finding common ancestor|hash mismatch" "$FULLNODE_LOG" | tail -30 || echo "No common ancestor logs found"

echo
echo -e "${YELLOW}=== Incremental Sync After Reorg ===${NC}"
echo "Looking for incremental sync behavior:"
grep -E "📊 After reorg|capped_target|incremental" "$FULLNODE_LOG" | tail -20 || echo "No incremental sync logs found"

echo
echo -e "${GREEN}=== Test Summary ===${NC}"
echo "1. Sequencer 1 created a chain (2s blocks)"
echo "2. Sequencer 2 created a different chain (3s blocks)"
echo "3. Full node synced from Sequencer 1"
echo "4. Full node switched to Sequencer 2"
echo "5. Reorg detection should occur due to chain divergence"
echo
echo "Check the logs for detailed reorg handling:"
echo "  - $FULLNODE_LOG: Full node sync and reorg detection"
echo "  - $SEQ1_LOG: Sequencer 1 blocks"
echo "  - $SEQ2_LOG: Sequencer 2 blocks"

echo
echo -e "${YELLOW}Keeping processes running for observation...${NC}"
echo "Press Ctrl+C to stop the test"

# Keep running to observe
wait
