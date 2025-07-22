#!/bin/bash

# Default values
PRIVATE_KEY="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
ANVIL_URL="http://localhost:8545"
MOCK_GPS_VERIFIER="test_utils/scripts/artifacts/MockGPSVerifier.sol:MockGPSVerifier"
VERIFIER_FILE_NAME="verifier_address.txt"

# Function to display usage
usage() {
    echo "Usage: $0 [OPTIONS]"
    echo "Options:"
    echo "  --private-key <key>           Private key for deployment (default: anvil default key)"
    echo "  --anvil-url <url>             Anvil RPC URL (default: http://localhost:8545)"
    echo "  --mock-gps-verifier-path <path>  Path to MockGPSVerifier contract (default: orchestrator/scripts/artifacts/eth/MockGPSVerifier.sol:MockGPSVerifier)"
    echo "  --verifier-file-name <name>   Output file name (default: verifier_address.txt)"
    echo "  -h, --help                    Show this help message"
    exit 1
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --private-key)
            PRIVATE_KEY="$2"
            shift 2
            ;;
        --anvil-url)
            ANVIL_URL="$2"
            shift 2
            ;;
        --mock-gps-verifier-path)
            MOCK_GPS_VERIFIER="$2"
            shift 2
            ;;
        --verifier-file-name)
            VERIFIER_FILE_NAME="$2"
            shift 2
            ;;
        -h|--help)
            usage
            ;;
        *)
            echo "Unknown option: $1"
            usage
            ;;
    esac
done

# Validate required parameters
if [[ -z "$PRIVATE_KEY" ]]; then
    echo "Error: Private key is required"
    exit 1
fi

if [[ -z "$ANVIL_URL" ]]; then
    echo "Error: Anvil URL is required"
    exit 1
fi

if [[ -z "$MOCK_GPS_VERIFIER" ]]; then
    echo "Error: Mock GPS verifier path is required"
    exit 1
fi

if [[ -z "$VERIFIER_FILE_NAME" ]]; then
    echo "Error: Verifier file name is required"
    exit 1
fi

# Display configuration
echo "Configuration:"
echo "  Private Key: ${PRIVATE_KEY:0:10}..."
echo "  Anvil URL: $ANVIL_URL"
echo "  Mock GPS Verifier: $MOCK_GPS_VERIFIER"
echo "  Output File: $VERIFIER_FILE_NAME"
echo ""

# Deploy the verifier contract using forge create
echo -e "🚀 Deploying verifier contract...\n"
VERIFIER_RESULT=$(forge create \
    --rpc-url "$ANVIL_URL" \
    --broadcast \
    --private-key "$PRIVATE_KEY" \
    "$MOCK_GPS_VERIFIER" \
    2>&1)

if [ $? -ne 0 ]; then
    echo "Error deploying verifier contract:"
    echo "$VERIFIER_RESULT"
    exit 1
fi

# Extract contract address from forge create output
VERIFIER_ADDRESS=$(echo "$VERIFIER_RESULT" | grep "to" | awk '{print $3}')
echo -e "📦 Verifier deployed at: $VERIFIER_ADDRESS\n"

# Write only the address to the specified file on success
echo "$VERIFIER_ADDRESS" > "$VERIFIER_FILE_NAME"

echo "✅ Contract address saved to: $VERIFIER_FILE_NAME"
