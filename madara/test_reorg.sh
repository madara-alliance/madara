#!/bin/bash

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Binary path - use the madara binary from cargo build
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
MADARA="$HOME/cache/rust-targets/release/madara"
MADARA_DB="$HOME/cache/madara/db"

# Database directories
SEQ1_DB="$MADARA_DB/seq1_db"
SEQ2_DB="$MADARA_DB/seq2_db"
FULLNODE_DB="$MADARA_DB/fullnode_db"
GENESIS_DB="$MADARA_DB/genesis_db"

# Log files
SEQ1_LOG="$MADARA_DB/seq1.log"
SEQ2_LOG="$MADARA_DB/seq2.log"
FULLNODE_LOG="$MADARA_DB/fullnode.log"
REORG_LOG="$MADARA_DB/reorg.log"

# Start logging to reorg_log file (and also display on console)
exec > >(tee -a "$REORG_LOG")
exec 2>&1

echo "=== Madara Reorg Test ==="
echo "This test will demonstrate proper reorg detection and handling"
echo "Both sequencers will start from the same state and then diverge"
echo "All output is being logged to: $REORG_LOG"
echo

# Initialize PID variables
GENESIS_PID=""
SEQ1_PID=""
SEQ2_PID=""
FULLNODE_PID=""

# Clean up function
cleanup() {
    echo -e "${YELLOW}Cleaning up processes...${NC}"
    # Kill specific PIDs instead of using pkill
    for PID in $GENESIS_PID $SEQ1_PID $SEQ2_PID $FULLNODE_PID; do
        if [ ! -z "$PID" ]; then
            kill $PID 2>/dev/null || true
        fi
    done
    # Also clean up any remaining madara processes, but be more specific
    pkill -f "madara.*--name" 2>/dev/null || true
    sleep 2
}

# Trap to ensure cleanup on exitlet
trap cleanup EXIT

# Initial cleanup
cleanup
echo -e "${YELLOW}Removing old databases and logs...${NC}"
rm -rf "$SEQ1_DB" "$SEQ2_DB" "$FULLNODE_DB" "$GENESIS_DB"
rm -f "$SEQ1_LOG" "$SEQ2_LOG" "$FULLNODE_LOG" "$REORG_LOG.prev"

# Backup previous reorg_log if it exists
if [ -f "$REORG_LOG" ]; then
    mv "$REORG_LOG" "$REORG_LOG.prev"
    echo "Previous reorg_log backed up to $REORG_LOG.prev"
fi

echo -e "${GREEN}Step 1: Creating genesis state${NC}"
echo "Starting a temporary sequencer to create genesis block"
MADARA_DB_MAX_SAVED_TRIE_LOGS=1000 RUST_LOG=info $MADARA \
    --name genesis \
    --devnet \
    --base-path "$GENESIS_DB" \
    --rpc-port 9999 \
    --feeder-gateway-enable \
    --gateway-port 8999 \
    --gateway-enable \
    --gateway-external \
    --chain-config-override chain_id=MADARA_CHAIN \
    --chain-config-override block_time=1s \
    2>&1 &
GENESIS_PID=$!

echo "Waiting for genesis to be created..."
sleep 25

# Get and log the genesis block details before shutdown
echo -e "${YELLOW}Capturing genesis block details:${NC}"
GENESIS_BLOCK=$(curl -s "http://localhost:8999/feeder_gateway/get_block?blockNumber=0" 2>/dev/null)
if [ ! -z "$GENESIS_BLOCK" ]; then
    echo "Genesis block hash: $(echo "$GENESIS_BLOCK" | grep -o '"block_hash":"[^"]*' | cut -d'"' -f4 | head -1)"
    GENESIS_HASH=$(echo "$GENESIS_BLOCK" | grep -o '"block_hash":"[^"]*' | cut -d'"' -f4 | head -1)
else
    echo "Warning: Could not fetch genesis block details"
fi

# Get the latest block created
LATEST_BLOCK=$(curl -s "http://localhost:8999/feeder_gateway/get_block?blockNumber=latest" 2>/dev/null)
if [ ! -z "$LATEST_BLOCK" ]; then
    LATEST_BLOCK_NUM=$(echo "$LATEST_BLOCK" | grep -o '"block_number":[0-9]*' | cut -d':' -f2 | head -1)
    LATEST_BLOCK_HASH=$(echo "$LATEST_BLOCK" | grep -o '"block_hash":"[^"]*' | cut -d'"' -f4 | head -1)
    echo "Latest block created: $LATEST_BLOCK_NUM"
    echo "Latest block hash: $LATEST_BLOCK_HASH"
else
    echo "Latest block created: 0 (genesis only)"
    LATEST_BLOCK_NUM=0
fi

# Kill the genesis sequencer
kill $GENESIS_PID 2>/dev/null || true
sleep 2

echo -e "${GREEN}Step 2: Copying genesis state to both sequencers${NC}"
echo "This ensures both chains start from the same point"
cp -r "$GENESIS_DB" "$SEQ1_DB"
cp -r "$GENESIS_DB" "$SEQ2_DB"

echo -e "${GREEN}Step 3: Starting Sequencer 1 (Primary chain)${NC}"
echo "This will create the first fork of the chain"
MADARA_DB_MAX_SAVED_TRIE_LOGS=1000 RUST_LOG=info,mc_sync=debug,mc_import=debug $MADARA \
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

echo "Waiting for Sequencer 1 to start producing blocks..."
sleep 10

# Verify Sequencer 1 is running
echo "Verifying Sequencer 1 gateway..."
curl -s "http://localhost:8080/feeder_gateway/get_block?blockNumber=latest" | head -1
echo

echo -e "${GREEN}Step 4: Starting Sequencer 2 (Divergent chain)${NC}"
echo "This will create a different fork from the same genesis"
MADARA_DB_MAX_SAVED_TRIE_LOGS=1000 RUST_LOG=info,mc_sync=debug,mc_import=debug $MADARA \
    --name sequencer2 \
    --devnet \
    --base-path "$SEQ2_DB" \
    --rpc-port 9945 \
    --feeder-gateway-enable \
    --gateway-port 8081 \
    --gateway-enable \
    --gateway-external \
    --chain-config-override chain_id=MADARA_CHAIN \
    --chain-config-override block_time=1s \
    2>&1 | tee "$SEQ2_LOG" &
SEQ2_PID=$!

echo "Waiting for Sequencer 2 to start producing blocks..."
sleep 10

# Verify both sequencers have the same genesis block
echo -e "${YELLOW}Verifying common genesis block between sequencers:${NC}"
SEQ1_GENESIS=$(curl -s "http://localhost:8080/feeder_gateway/get_block?blockNumber=0" 2>/dev/null)
SEQ2_GENESIS=$(curl -s "http://localhost:8081/feeder_gateway/get_block?blockNumber=0" 2>/dev/null)

if [ ! -z "$SEQ1_GENESIS" ] && [ ! -z "$SEQ2_GENESIS" ]; then
    SEQ1_GENESIS_HASH=$(echo "$SEQ1_GENESIS" | grep -o '"block_hash":"[^"]*' | cut -d'"' -f4 | head -1)
    SEQ2_GENESIS_HASH=$(echo "$SEQ2_GENESIS" | grep -o '"block_hash":"[^"]*' | cut -d'"' -f4 | head -1)

    echo "Sequencer 1 genesis hash: $SEQ1_GENESIS_HASH"
    echo "Sequencer 2 genesis hash: $SEQ2_GENESIS_HASH"

    if [ "$SEQ1_GENESIS_HASH" = "$SEQ2_GENESIS_HASH" ]; then
        echo -e "${GREEN}✓ Both sequencers have the same genesis block${NC}"
    else
        echo -e "${RED}✗ Genesis blocks do not match! Test cannot proceed properly${NC}"
        echo "This means the sequencers don't share a common ancestor"
        exit 1
    fi

    # Also verify it matches the original genesis if we captured it
    if [ ! -z "$GENESIS_HASH" ]; then
        if [ "$SEQ1_GENESIS_HASH" = "$GENESIS_HASH" ]; then
            echo -e "${GREEN}✓ Genesis matches the original genesis block${NC}"
        else
            echo -e "${YELLOW}⚠ Genesis differs from original (may be expected if blocks were produced)${NC}"
        fi
    fi
else
    echo -e "${YELLOW}Warning: Could not fetch genesis blocks from sequencers${NC}"
fi

# Verify both sequencers have the same hash at the genesis latest block number
if [ "$LATEST_BLOCK_NUM" -gt 0 ]; then
    echo -e "${YELLOW}Verifying block $LATEST_BLOCK_NUM hash matches between sequencers:${NC}"

    SEQ1_BLOCK=$(curl -s "http://localhost:8080/feeder_gateway/get_block?blockNumber=$LATEST_BLOCK_NUM" 2>/dev/null)
    SEQ2_BLOCK=$(curl -s "http://localhost:8081/feeder_gateway/get_block?blockNumber=$LATEST_BLOCK_NUM" 2>/dev/null)

    if [ ! -z "$SEQ1_BLOCK" ] && [ ! -z "$SEQ2_BLOCK" ]; then
        SEQ1_BLOCK_HASH=$(echo "$SEQ1_BLOCK" | grep -o '"block_hash":"[^"]*' | cut -d'"' -f4 | head -1)
        SEQ2_BLOCK_HASH=$(echo "$SEQ2_BLOCK" | grep -o '"block_hash":"[^"]*' | cut -d'"' -f4 | head -1)

        echo "Sequencer 1 block $LATEST_BLOCK_NUM hash: $SEQ1_BLOCK_HASH"
        echo "Sequencer 2 block $LATEST_BLOCK_NUM hash: $SEQ2_BLOCK_HASH"

        if [ "$SEQ1_BLOCK_HASH" = "$SEQ2_BLOCK_HASH" ] && [ "$SEQ1_BLOCK_HASH" = "$LATEST_BLOCK_HASH" ]; then
            echo -e "${GREEN}✓ Block $LATEST_BLOCK_NUM has the same hash on both sequencers${NC}"
            echo -e "${GREEN}✓ This confirms both chains start from identical state up to block $LATEST_BLOCK_NUM${NC}"
        else
            echo -e "${RED}✗ Block $LATEST_BLOCK_NUM hashes do not match!${NC}"
            echo "Expected (from genesis): $LATEST_BLOCK_HASH"
            echo "Sequencer 1: $SEQ1_BLOCK_HASH"
            echo "Sequencer 2: $SEQ2_BLOCK_HASH"
            echo "This means the chains diverged before expected!"
            exit 1
        fi
    else
        echo -e "${YELLOW}Warning: Could not fetch block $LATEST_BLOCK_NUM from sequencers${NC}"
    fi
    echo
fi

# Show current block heights
echo -e "${YELLOW}Current block heights:${NC}"
SEQ1_LATEST=$(curl -s "http://localhost:8080/feeder_gateway/get_block?blockNumber=latest" | grep -o '"block_number":[0-9]*' | cut -d':' -f2 | head -1)
SEQ2_LATEST=$(curl -s "http://localhost:8081/feeder_gateway/get_block?blockNumber=latest" | grep -o '"block_number":[0-9]*' | cut -d':' -f2 | head -1)
echo "Sequencer 1 at block: ${SEQ1_LATEST:-0}"
echo "Sequencer 2 at block: ${SEQ2_LATEST:-0}"
echo

echo -e "${GREEN}Step 5: Starting Full Node syncing from Sequencer 1${NC}"
echo "Full node will initially sync from Sequencer 1's gateway"
MADARA_DB_MAX_SAVED_TRIE_LOGS=1000 RUST_LOG=info,mc_sync=debug,mc_import=debug $MADARA \
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

echo "Letting full node sync from Sequencer 1 for 30 seconds to ensure blocks are fully processed..."
echo "This gives time for all 3 pipelines (blocks, classes, state) to complete"
sleep 30

# Check sync status and look for fully imported blocks
echo -e "${YELLOW}Checking full node sync status from Sequencer 1:${NC}"
echo "Looking for 'fully imported' messages:"
grep "fully imported\|head status saved" "$FULLNODE_LOG" | tail -5 || echo "No fully imported messages found yet"
echo
echo "Current sync status:"
tail -5 "$FULLNODE_LOG" | grep -E "(Blocks:|Sync is at)" || true
echo

echo -e "${GREEN}Step 6: Switching full node to Sequencer 2${NC}"
echo "This should trigger reorg detection as chains have diverged from the same genesis"

# Stop the fullnode
echo "Stopping full node..."
kill $FULLNODE_PID 2>/dev/null || true
sleep 2

# Make sure the process is really dead
if ps -p $FULLNODE_PID > /dev/null 2>&1; then
    echo "Process still running, force killing..."
    kill -9 $FULLNODE_PID 2>/dev/null || true
fi

echo "Waiting for full node to fully shutdown..."
sleep 5

# Ensure the database lock is released
if [ -f "$FULLNODE_DB/db/LOCK" ]; then
    echo "Removing stale database lock file..."
    rm -f "$FULLNODE_DB/db/LOCK"
fi

# Kill any process using port 4444
if lsof -i:4444 > /dev/null 2>&1; then
    echo "Port 4444 still in use, finding and killing the process..."
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

echo -e "${GREEN}Step 7: Restarting full node to sync from Sequencer 2${NC}"
echo "This should trigger reorg detection as chains have diverged"
MADARA_DB_MAX_SAVED_TRIE_LOGS=1000 RUST_LOG=info,mc_sync=debug,mc_import=debug $MADARA \
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

echo "Waiting for reorg detection and sync (25 seconds)..."
echo "This gives time for reorg detection, rollback, and resync from the new chain"
sleep 45

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
echo "1. Both sequencers started from the same genesis"
echo "2. Sequencer 1 produced blocks at 2s intervals"
echo "3. Sequencer 2 produced blocks at 3s intervals (creating divergence)"
echo "4. Full node synced from Sequencer 1"
echo "5. Full node switched to Sequencer 2"
echo "6. Reorg should be detected with proper common ancestor finding"
echo
echo "Check the logs for detailed reorg handling:"
echo "  - $REORG_LOG: Complete test execution log (this file)"
echo "  - $FULLNODE_LOG: Full node sync and reorg detection"
echo "  - $SEQ1_LOG: Sequencer 1 blocks"
echo "  - $SEQ2_LOG: Sequencer 2 blocks"

echo
echo -e "${YELLOW}Test complete. All output has been saved to $REORG_LOG${NC}"
echo -e "${YELLOW}Press Ctrl+C to stop all processes.${NC}"

# Keep running to observe
# wait
