#!/bin/bash

# Performance test script to measure database operations at different block counts
# Tests with 1,000, 3,000, and 10,000 blocks

set -e

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Binary and paths
MADARA="/Volumes/itsparser/rust-targets/release/madara"
BASE_DB_PATH="/tmp/madara_perf_test"
RESULTS_FILE="/tmp/madara_performance_results.txt"

# Test configurations (block counts to test)
BLOCK_COUNTS=(1000 3000 10000)

# Cleanup function
cleanup() {
    echo -e "${YELLOW}Cleaning up processes...${NC}"
    pkill -f "madara.*perf_test" 2>/dev/null || true
    sleep 2
}

trap cleanup EXIT

# Function to run a single test
run_test() {
    local target_blocks=$1
    local test_name="test_${target_blocks}_blocks"
    local db_path="${BASE_DB_PATH}_${target_blocks}"
    local log_file="/tmp/madara_perf_${target_blocks}.log"
    
    echo -e "${BLUE}========================================${NC}"
    echo -e "${BLUE}Testing with $target_blocks blocks${NC}"
    echo -e "${BLUE}========================================${NC}"
    
    # Clean up any existing database
    rm -rf "$db_path"
    
    # Phase 1: Initial sync
    echo -e "${GREEN}Phase 1: Starting initial sync to $target_blocks blocks...${NC}"
    local sync_start=$(date +%s)
    
    # Start madara with 1s block time
    timeout 300 $MADARA \
        --name "perf_test_${target_blocks}" \
        --devnet \
        --base-path "$db_path" \
        --rpc-port $((9000 + target_blocks/1000)) \
        --feeder-gateway-enable \
        --gateway-port $((8000 + target_blocks/1000)) \
        --gateway-enable \
        --gateway-external \
        --chain-config-override chain_id=PERF_TEST \
        --chain-config-override block_time=1s \
        2>&1 | tee "$log_file" | while read line; do
            # Monitor for target block reached
            if echo "$line" | grep -q "Block #$target_blocks.*fully imported\|Block #$target_blocks.*head status saved"; then
                echo -e "${GREEN}✓ Reached target block $target_blocks${NC}"
                pkill -f "madara.*perf_test_${target_blocks}" 2>/dev/null || true
                break
            fi
            # Also check if we've exceeded the target
            block_num=$(echo "$line" | grep -o "Block #[0-9]*.*fully imported\|Block #[0-9]*.*head status saved" | grep -o "#[0-9]*" | grep -o "[0-9]*" | tail -1)
            if [ ! -z "$block_num" ] && [ "$block_num" -ge "$target_blocks" ]; then
                echo -e "${GREEN}✓ Reached block $block_num (>= target $target_blocks)${NC}"
                pkill -f "madara.*perf_test_${target_blocks}" 2>/dev/null || true
                break
            fi
        done || true
    
    local sync_end=$(date +%s)
    local sync_time=$((sync_end - sync_start))
    
    # Wait for process to fully terminate
    sleep 3
    
    # Verify blocks were synced
    local last_block=$(grep "fully imported\|head status saved" "$log_file" | tail -1 | grep -o "Block #[0-9]*" | grep -o "[0-9]*" || echo "0")
    
    if [ "$last_block" -lt "$((target_blocks - 10))" ]; then
        echo -e "${RED}Warning: Only synced to block $last_block (target was $target_blocks)${NC}"
    else
        echo -e "${GREEN}Successfully synced to block $last_block${NC}"
    fi
    
    # Phase 2: Restart and measure head status load time
    echo -e "${GREEN}Phase 2: Restarting to measure head status load time...${NC}"
    
    local restart_start=$(date +%s.%N)
    
    # Restart and capture initialization logs
    timeout 5 $MADARA \
        --name "perf_test_${target_blocks}" \
        --base-path "$db_path" \
        --rpc-port $((9000 + target_blocks/1000)) \
        --full \
        --preset devnet \
        --chain-config-override chain_id=PERF_TEST \
        --no-l1-sync \
        --gateway-url http://localhost:8080 \
        2>&1 | tee "${log_file}_restart" | while read line; do
            # Look for database head status loading
            if echo "$line" | grep -q "Database head status:"; then
                local restart_end=$(date +%s.%N)
                echo -e "${GREEN}✓ Head status loaded${NC}"
                pkill -f "madara.*perf_test_${target_blocks}" 2>/dev/null || true
                break
            fi
        done || true
    
    local restart_end=$(date +%s.%N)
    local restart_time=$(echo "$restart_end - $restart_start" | bc)
    
    # Extract head status from restart log
    local loaded_block=$(grep "latest_full_block=" "${log_file}_restart" | grep -o "Some([0-9]*)" | grep -o "[0-9]*" | head -1 || echo "none")
    
    # Phase 3: Measure reorg detection time (simulate switching gateways)
    echo -e "${GREEN}Phase 3: Testing reorg detection performance...${NC}"
    
    # First, start a second sequencer from the same database to create divergence
    local seq2_db="${db_path}_seq2"
    cp -r "$db_path" "$seq2_db"
    
    # Start second sequencer with different block time to create divergence
    timeout 30 $MADARA \
        --name "perf_test_seq2_${target_blocks}" \
        --devnet \
        --base-path "$seq2_db" \
        --rpc-port $((9500 + target_blocks/1000)) \
        --feeder-gateway-enable \
        --gateway-port $((8500 + target_blocks/1000)) \
        --gateway-enable \
        --gateway-external \
        --chain-config-override chain_id=PERF_TEST \
        --chain-config-override block_time=1s \
        2>&1 > "${log_file}_seq2" &
    
    local seq2_pid=$!
    sleep 10  # Let it produce some divergent blocks
    
    # Now restart the full node pointing to the divergent chain
    local reorg_start=$(date +%s.%N)
    
    timeout 15 $MADARA \
        --name "perf_test_${target_blocks}" \
        --base-path "$db_path" \
        --rpc-port $((9000 + target_blocks/1000)) \
        --full \
        --preset devnet \
        --chain-config-override chain_id=PERF_TEST \
        --no-l1-sync \
        --gateway-url http://localhost:$((8500 + target_blocks/1000)) \
        2>&1 | tee "${log_file}_reorg" | while read line; do
            # Look for reorg detection
            if echo "$line" | grep -q "REORG DETECTED\|common ancestor"; then
                local reorg_end=$(date +%s.%N)
                echo -e "${GREEN}✓ Reorg detected${NC}"
                pkill -f "madara.*perf_test" 2>/dev/null || true
                break
            fi
        done || true
    
    local reorg_end=$(date +%s.%N)
    local reorg_time=$(echo "$reorg_end - $reorg_start" | bc)
    
    # Kill the second sequencer
    kill $seq2_pid 2>/dev/null || true
    
    # Clean up processes
    pkill -f "madara.*perf_test" 2>/dev/null || true
    sleep 2
    
    # Calculate and display results
    echo
    echo -e "${BLUE}=== Results for $target_blocks blocks ===${NC}"
    echo "Initial sync time: ${sync_time} seconds"
    echo "Database size: $(du -sh "$db_path" 2>/dev/null | cut -f1 || echo 'N/A')"
    echo "Head status load time: ${restart_time} seconds"
    echo "Loaded block on restart: $loaded_block"
    echo "Reorg detection time: ${reorg_time} seconds"
    
    # Append to results file
    echo "=== Test: $target_blocks blocks ===" >> "$RESULTS_FILE"
    echo "Timestamp: $(date)" >> "$RESULTS_FILE"
    echo "Initial sync time: ${sync_time} seconds" >> "$RESULTS_FILE"
    echo "Database size: $(du -sh "$db_path" 2>/dev/null | cut -f1 || echo 'N/A')" >> "$RESULTS_FILE"
    echo "Head status load time: ${restart_time} seconds" >> "$RESULTS_FILE"
    echo "Loaded block on restart: $loaded_block" >> "$RESULTS_FILE"
    echo "Reorg detection time: ${reorg_time} seconds" >> "$RESULTS_FILE"
    echo "Blocks synced: $last_block" >> "$RESULTS_FILE"
    echo "" >> "$RESULTS_FILE"
    
    # Clean up test database
    rm -rf "$db_path" "$seq2_db"
    
    return 0
}

# Main execution
echo -e "${BLUE}=== Madara Performance Testing ===${NC}"
echo "Testing database operations at different block counts"
echo "All tests use block_time=1s for consistency"
echo

# Clear results file
> "$RESULTS_FILE"
echo "Madara Performance Test Results" > "$RESULTS_FILE"
echo "================================" >> "$RESULTS_FILE"
echo "" >> "$RESULTS_FILE"

# Run tests for each block count
for block_count in "${BLOCK_COUNTS[@]}"; do
    run_test $block_count
    echo
    echo -e "${YELLOW}Waiting 5 seconds before next test...${NC}"
    sleep 5
done

# Display summary
echo
echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}         PERFORMANCE SUMMARY            ${NC}"
echo -e "${BLUE}========================================${NC}"
echo

cat "$RESULTS_FILE"

echo
echo -e "${GREEN}=== All tests complete ===${NC}"
echo "Detailed results saved to: $RESULTS_FILE"
echo
echo "Key metrics measured:"
echo "1. Initial sync time - Time to sync N blocks from genesis"
echo "2. Database size - Storage requirements for N blocks"
echo "3. Head status load time - Time to load and initialize database on restart"
echo "4. Reorg detection time - Time to detect and handle chain reorganization"