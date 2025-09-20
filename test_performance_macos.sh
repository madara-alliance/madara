#!/bin/bash

# Performance test script to measure database operations at different block counts
# Tests with 1,000, 3,000, and 10,000 blocks
# Modified for macOS compatibility

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
BLOCK_COUNTS=(100 500 1000)  # Reduced for testing

# Cleanup function
cleanup() {
    echo -e "${YELLOW}Cleaning up processes...${NC}"
    pkill -f "madara.*perf_test" 2>/dev/null || true
    sleep 2
}

trap cleanup EXIT

# Function to run command with timeout (macOS compatible)
run_with_timeout() {
    local timeout=$1
    shift
    local command="$@"
    
    # Run command in background
    eval "$command" &
    local pid=$!
    
    # Wait for specified timeout
    local count=0
    while [ $count -lt $timeout ]; do
        if ! kill -0 $pid 2>/dev/null; then
            # Process has finished
            wait $pid
            return $?
        fi
        sleep 1
        count=$((count + 1))
    done
    
    # Timeout reached, kill process
    kill -TERM $pid 2>/dev/null || true
    sleep 2
    kill -KILL $pid 2>/dev/null || true
    return 124  # timeout exit code
}

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
    $MADARA \
        --name "perf_test_${target_blocks}" \
        --devnet \
        --base-path "$db_path" \
        --rpc-port $((9000 + target_blocks/100)) \
        --feeder-gateway-enable \
        --gateway-port $((8000 + target_blocks/100)) \
        --gateway-enable \
        --gateway-external \
        --chain-config-override chain_id=PERF_TEST \
        --chain-config-override block_time=1s \
        2>&1 | tee "$log_file" &
    
    local madara_pid=$!
    
    # Monitor for target block reached
    local reached_target=false
    local count=0
    local max_wait=$((target_blocks * 2))  # Wait up to 2 seconds per block
    
    while [ $count -lt $max_wait ]; do
        if ! kill -0 $madara_pid 2>/dev/null; then
            echo -e "${RED}Madara process died unexpectedly${NC}"
            break
        fi
        
        # Check latest block in log
        local last_block=$(grep "fully imported\|head status saved" "$log_file" 2>/dev/null | tail -1 | grep -o "Block #[0-9]*" | grep -o "[0-9]*" || echo "0")
        
        if [ ! -z "$last_block" ] && [ "$last_block" -ge "$target_blocks" ]; then
            echo -e "${GREEN}✓ Reached block $last_block (>= target $target_blocks)${NC}"
            reached_target=true
            break
        fi
        
        if [ $((count % 10)) -eq 0 ]; then
            echo "Waiting... Current block: $last_block / $target_blocks"
        fi
        
        sleep 1
        count=$((count + 1))
    done
    
    # Kill madara process
    kill -TERM $madara_pid 2>/dev/null || true
    sleep 2
    kill -KILL $madara_pid 2>/dev/null || true
    
    local sync_end=$(date +%s)
    local sync_time=$((sync_end - sync_start))
    
    # Wait for process to fully terminate
    sleep 3
    
    # Verify blocks were synced
    local last_block=$(grep "fully imported\|head status saved" "$log_file" 2>/dev/null | tail -1 | grep -o "Block #[0-9]*" | grep -o "[0-9]*" || echo "0")
    
    if [ "$last_block" -lt "$((target_blocks - 10))" ]; then
        echo -e "${RED}Warning: Only synced to block $last_block (target was $target_blocks)${NC}"
    else
        echo -e "${GREEN}Successfully synced to block $last_block${NC}"
    fi
    
    # Phase 2: Restart and measure head status load time
    echo -e "${GREEN}Phase 2: Restarting to measure head status load time...${NC}"
    
    local restart_start=$(date +%s)
    
    # Restart and capture initialization logs
    $MADARA \
        --name "perf_test_${target_blocks}" \
        --base-path "$db_path" \
        --rpc-port $((9000 + target_blocks/100)) \
        --full \
        --preset devnet \
        --chain-config-override chain_id=PERF_TEST \
        --no-l1-sync \
        --gateway-url http://localhost:8080 \
        2>&1 | tee "${log_file}_restart" &
    
    local restart_pid=$!
    
    # Wait for head status to load
    local head_loaded=false
    count=0
    while [ $count -lt 10 ]; do
        if grep -q "Database head status:" "${log_file}_restart" 2>/dev/null; then
            echo -e "${GREEN}✓ Head status loaded${NC}"
            head_loaded=true
            break
        fi
        sleep 1
        count=$((count + 1))
    done
    
    kill -TERM $restart_pid 2>/dev/null || true
    sleep 1
    kill -KILL $restart_pid 2>/dev/null || true
    
    local restart_end=$(date +%s)
    local restart_time=$((restart_end - restart_start))
    
    # Extract head status from restart log
    local loaded_block=$(grep "latest_full_block=" "${log_file}_restart" 2>/dev/null | grep -o "Some([0-9]*)" | grep -o "[0-9]*" | head -1 || echo "none")
    
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
    echo "Last synced block: $last_block"
    
    # Append to results file
    echo "=== Test: $target_blocks blocks ===" >> "$RESULTS_FILE"
    echo "Timestamp: $(date)" >> "$RESULTS_FILE"
    echo "Initial sync time: ${sync_time} seconds" >> "$RESULTS_FILE"
    echo "Database size: $(du -sh "$db_path" 2>/dev/null | cut -f1 || echo 'N/A')" >> "$RESULTS_FILE"
    echo "Head status load time: ${restart_time} seconds" >> "$RESULTS_FILE"
    echo "Loaded block on restart: $loaded_block" >> "$RESULTS_FILE"
    echo "Blocks synced: $last_block" >> "$RESULTS_FILE"
    echo "" >> "$RESULTS_FILE"
    
    # Clean up test database
    rm -rf "$db_path"
    
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