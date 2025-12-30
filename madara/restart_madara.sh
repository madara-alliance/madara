#!/bin/bash

# ============================================================================
# MADARA AUTO-RESTART SCRIPT
# ============================================================================
# 
# WHERE TO SAVE THIS SCRIPT:
#   - Save this script anywhere convenient (e.g., ~/scripts/ or your Madara project root)
#   - Make it executable: chmod +x restart_madara.sh
#
# WHERE TO RUN THIS SCRIPT FROM:
#   - MUST be run from your Madara project ROOT directory
#   - This is the directory containing Cargo.toml (the main workspace Cargo.toml)
#   - The script uses 'cargo run' which expects to be in the project root
#   - The relative path '../configs/presets/paradex_mock.yaml' also depends on this
#
# Example directory structure:
#   /path/to/madara/
#   ├── Cargo.toml          <- Run script from here
#   ├── configs/
#   │   └── presets/
#   │       └── paradex_mock.yaml
#   └── restart_madara.sh   <- Script can be saved here OR elsewhere
#
# ============================================================================

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Log file for the restart script itself
# Adjust this path to your preferred log directory
RESTART_LOG="/home/ubuntu/madara_paradex_mock_db_logs/restart_monitor.log"

# Main log file for Madara output
MADARA_LOG="/home/ubuntu/madara_paradex_mock_db_logs/logs.log"

# Create log directory if it doesn't exist
mkdir -p "$(dirname "$RESTART_LOG")"
mkdir -p "$(dirname "$MADARA_LOG")"

# Check if we're in a directory with Cargo.toml
if [ ! -f "Cargo.toml" ]; then
    echo -e "${RED}ERROR:${NC} Cargo.toml not found in current directory!"
    echo -e "${YELLOW}Please run this script from your Madara project root directory.${NC}"
    echo -e "Current directory: $(pwd)"
    exit 1
fi

# Check if the config file path exists (relative to project root)
if [ ! -f "../configs/presets/paradex_mock.yaml" ]; then
    echo -e "${YELLOW}WARNING:${NC} Config file not found at '../configs/presets/paradex_mock.yaml'"
    echo -e "Please verify the --chain-config-path is correct for your setup."
fi

# Function to log messages
log_message() {
    echo -e "${GREEN}[$(date '+%Y-%m-%d %H:%M:%S')]${NC} $1" | tee -a "$RESTART_LOG"
}

log_error() {
    echo -e "${RED}[$(date '+%Y-%m-%d %H:%M:%S')] ERROR:${NC} $1" | tee -a "$RESTART_LOG"
}

log_warning() {
    echo -e "${YELLOW}[$(date '+%Y-%m-%d %H:%M:%S')] WARNING:${NC} $1" | tee -a "$RESTART_LOG"
}

# Counter for restarts
RESTART_COUNT=0

log_message "=== Madara Auto-Restart Monitor Started ==="
log_message "Running from directory: $(pwd)"
log_message "Monitoring for errors: 'No space left on device' and 'Replacing chain tip' errors..."

while true; do
    RESTART_COUNT=$((RESTART_COUNT + 1))
    log_message "Starting Madara service (Attempt #$RESTART_COUNT)..."
    
    # Run the command and capture its exit status
    # Using your specific command parameters
    RUST_LOG=info cargo run --release -- \
        --name madara \
        --base-path /home/ubuntu/home/ubuntu/madara_paradex_mock_db \
        --rpc-port 9946 \
        --rpc-cors "*" \
        --rpc-external \
        --analytics-prometheus-endpoint-port 9455 \
        --sequencer \
        --gateway-port 8989 \
        --gateway-external \
        --rpc-unsafe \
        --rpc-admin \
        --rpc-admin-external \
        --gateway-enable \
        --no-l1-sync \
        --chain-config-path ../configs/presets/paradex_mock.yaml \
        --skip-migration-backup \
        2>&1 | tee -a "$MADARA_LOG"
    
    EXIT_CODE=$?
    
    # Check if the process exited due to an error
    if [ $EXIT_CODE -ne 0 ]; then
        log_error "Service exited with code $EXIT_CODE"
        
        # Check the last few lines of the log for known errors
        LOG_TAIL=$(tail -50 "$MADARA_LOG" 2>/dev/null || echo "")
        
        if echo "$LOG_TAIL" | grep -q "No space left on device"; then
            log_warning "Detected 'No space left on device' error"
            log_message "Waiting 10 seconds before restart..."
            sleep 10
        elif echo "$LOG_TAIL" | grep -q "Replacing chain tip from confirmed to confirmed"; then
            log_warning "Detected 'Replacing chain tip' error (block synchronization issue)"
            log_message "This indicates a chain tip mismatch. Restarting to resync..."
            log_message "Waiting 10 seconds before restart..."
            sleep 10
        else
            log_error "Service failed for unknown reason. Waiting 10 seconds..."
            sleep 10
        fi
    else
        log_message "Service exited normally (exit code 0)"
        log_message "Waiting 10 seconds before restart..."
        sleep 10
    fi
    
    log_message "Restarting service now..."
done
