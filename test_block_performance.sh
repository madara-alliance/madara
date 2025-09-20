#!/bin/bash

# Performance test for measuring sync and restart times at different block counts
# Tests with 1000, 3000, and 10000 blocks using block_time=1s

set -e

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Configuration
MADARA="/Volumes/itsparser/rust-targets/release/madara"
MADARA_DB="/tmp/madara_perf"
RESULTS="/tmp/performance_results_$(date +%Y%m%d_%H%M%S).txt"

# Block counts to test
TARGETS=(1000 3000 10000)

# Initialize results file
echo "Madara Block Performance Test Results" > "$RESULTS"
echo "Test Date: $(date)" >> "$RESULTS"
echo "Block Time: 1s (consistent across all tests)" >> "$RESULTS"
echo "======================================" >> "$RESULTS"
echo >> "$RESULTS"

cleanup() {
    echo -e "${YELLOW}Cleaning up...${NC}"
    pkill -f "madara.*perf_" 2>/dev/null || true
    sleep 2
}

trap cleanup EXIT

# Function to wait for N blocks to be synced
wait_for_blocks() {
    local target=$1
    local log_file=$2
    local timeout=${3:-600}  # Default 10 minutes
    local start_time=$(date +%s)
    
    echo -e "${YELLOW}Waiting for $target blocks to sync (timeout: ${timeout}s)...${NC}"
    
    while true; do
        # Check latest synced block from log
        local current=$(grep -o "Block #[0-9]*.*fully imported\|Block #[0-9]*.*head status saved" "$log_file" 2>/dev/null | tail -1 | grep -o "#[0-9]*" | grep -o "[0-9]*" || echo "0")
        
        if [ "$current" -ge "$target" ]; then
            echo -e "${GREEN}✓ Reached block $current (target: $target)${NC}"
            return 0
        fi
        
        # Check timeout
        local elapsed=$(($(date +%s) - start_time))
        if [ "$elapsed" -gt "$timeout" ]; then
            echo -e "${RED}✗ Timeout after ${elapsed}s (reached block $current)${NC}"
            return 1
        fi
        
        # Show progress every 10 seconds
        if [ $((elapsed % 10)) -eq 0 ]; then
            echo "Progress: Block $current / $target (${elapsed}s elapsed)"
        fi
        
        sleep 1
    done
}

# Run test for each block count
for TARGET in "${TARGETS[@]}"; do
    echo
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${BLUE}   Testing with $TARGET blocks${NC}"
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    
    DB_PATH="${MADARA_DB}_${TARGET}"
    LOG_SYNC="${MADARA_DB}_${TARGET}_sync.log"
    LOG_RESTART="${MADARA_DB}_${TARGET}_restart.log"
    
    # Clean previous run
    rm -rf "$DB_PATH"
    rm -f "$LOG_SYNC" "$LOG_RESTART"
    
    # Calculate reasonable timeout (2 seconds per block + buffer)
    SYNC_TIMEOUT=$((TARGET * 2 + 60))
    
    echo -e "${GREEN}▶ Phase 1: Initial Sync${NC}"
    echo "Starting Madara to sync $TARGET blocks..."
    
    # Record start time
    SYNC_START=$(date +%s)
    
    # Start Madara in background
    RUST_LOG=info $MADARA \
        --name "perf_${TARGET}" \
        --devnet \
        --base-path "$DB_PATH" \
        --rpc-port $((9000 + TARGET/1000)) \
        --feeder-gateway-enable \
        --gateway-port $((8000 + TARGET/1000)) \
        --gateway-enable \
        --gateway-external \
        --chain-config-override chain_id=PERF_TEST \
        --chain-config-override block_time=1s \
        > "$LOG_SYNC" 2>&1 &
    
    MADARA_PID=$!
    
    # Wait for target blocks
    if wait_for_blocks "$TARGET" "$LOG_SYNC" "$SYNC_TIMEOUT"; then
        SYNC_END=$(date +%s)
        SYNC_TIME=$((SYNC_END - SYNC_START))
        echo -e "${GREEN}✓ Sync completed in ${SYNC_TIME} seconds${NC}"
        
        # Get exact last block
        LAST_BLOCK=$(grep -o "Block #[0-9]*.*fully imported\|Block #[0-9]*.*head status saved" "$LOG_SYNC" | tail -1 | grep -o "#[0-9]*" | grep -o "[0-9]*" || echo "0")
    else
        SYNC_TIME="TIMEOUT"
        LAST_BLOCK=$(grep -o "Block #[0-9]*.*fully imported\|Block #[0-9]*.*head status saved" "$LOG_SYNC" | tail -1 | grep -o "#[0-9]*" | grep -o "[0-9]*" || echo "0")
    fi
    
    # Stop Madara
    kill $MADARA_PID 2>/dev/null || true
    sleep 3
    
    # Get database size
    DB_SIZE=$(du -sh "$DB_PATH" 2>/dev/null | cut -f1 || echo "N/A")
    
    echo -e "${GREEN}▶ Phase 2: Restart Performance${NC}"
    echo "Restarting to measure head status load time..."
    
    # Record restart time with nanosecond precision
    RESTART_START=$(date +%s.%N)
    
    # Start and immediately check for head status load
    timeout 10 $MADARA \
        --name "perf_${TARGET}" \
        --base-path "$DB_PATH" \
        --rpc-port $((9000 + TARGET/1000)) \
        --full \
        --preset devnet \
        --chain-config-override chain_id=PERF_TEST \
        --no-l1-sync \
        --gateway-url http://localhost:8080 \
        2>&1 | tee "$LOG_RESTART" | while read line; do
            if echo "$line" | grep -q "Database head status:"; then
                pkill -f "madara.*perf_${TARGET}" 2>/dev/null || true
                break
            fi
        done || true
    
    RESTART_END=$(date +%s.%N)
    RESTART_TIME=$(echo "scale=3; $RESTART_END - $RESTART_START" | bc)
    
    # Extract loaded block number
    LOADED_BLOCK=$(grep "latest_full_block=" "$LOG_RESTART" | grep -o "Some([0-9]*)" | grep -o "[0-9]*" | head -1 || echo "none")
    
    # Check if persistence worked
    if [ "$LOADED_BLOCK" = "none" ] || [ "$LOADED_BLOCK" = "" ]; then
        PERSISTENCE="FAILED"
        echo -e "${RED}✗ Database did not persist head status${NC}"
    elif [ "$LOADED_BLOCK" -eq "$LAST_BLOCK" ] 2>/dev/null; then
        PERSISTENCE="SUCCESS"
        echo -e "${GREEN}✓ Database correctly persisted block $LOADED_BLOCK${NC}"
    else
        PERSISTENCE="PARTIAL"
        echo -e "${YELLOW}⚠ Database loaded block $LOADED_BLOCK (expected $LAST_BLOCK)${NC}"
    fi
    
    # Display results
    echo
    echo -e "${BLUE}Results for $TARGET blocks:${NC}"
    echo "├─ Blocks synced: $LAST_BLOCK"
    echo "├─ Sync time: $SYNC_TIME seconds"
    echo "├─ Database size: $DB_SIZE"
    echo "├─ Restart time: $RESTART_TIME seconds"
    echo "├─ Loaded block: $LOADED_BLOCK"
    echo "└─ Persistence: $PERSISTENCE"
    
    # Write to results file
    echo "Test: $TARGET blocks" >> "$RESULTS"
    echo "-------------------" >> "$RESULTS"
    echo "Blocks synced: $LAST_BLOCK" >> "$RESULTS"
    echo "Sync time: $SYNC_TIME seconds" >> "$RESULTS"
    echo "Database size: $DB_SIZE" >> "$RESULTS"
    echo "Restart time: $RESTART_TIME seconds" >> "$RESULTS"
    echo "Loaded block: $LOADED_BLOCK" >> "$RESULTS"
    echo "Persistence: $PERSISTENCE" >> "$RESULTS"
    if [ "$SYNC_TIME" != "TIMEOUT" ] && [ "$LAST_BLOCK" -gt 0 ]; then
        BLOCKS_PER_SEC=$(echo "scale=2; $LAST_BLOCK / $SYNC_TIME" | bc)
        echo "Sync rate: $BLOCKS_PER_SEC blocks/second" >> "$RESULTS"
    fi
    echo >> "$RESULTS"
    
    # Clean up for next test
    cleanup
    rm -rf "$DB_PATH"
    
    echo -e "${YELLOW}Waiting 5 seconds before next test...${NC}"
    sleep 5
done

# Final summary
echo
echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${GREEN}   PERFORMANCE TEST COMPLETE${NC}"
echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo
cat "$RESULTS"
echo
echo "Results saved to: $RESULTS"