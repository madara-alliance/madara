#!/bin/bash

# Check if jq is installed
if ! command -v jq &> /dev/null; then
    echo "Error: jq is required but not installed. Please install jq first."
    exit 1
fi

# Check if curl is installed
if ! command -v curl &> /dev/null; then
    echo "Error: curl is required but not installed. Please install curl first."
    exit 1
fi

# Check if required arguments are provided
if [ -z "$1" ] || [ -z "$2" ]; then
    echo "Usage: $0 <rpc-url> <block-number>"
    echo "Example: $0  http://localhost:9944 66644"
    exit 1
fi

# Read arguments
ABI_FILE='./e2e-tests/artifacts/contracts/Starknet.json'
RPC_URL=$1
BLOCK_NUMBER=$2

# Default Anvil private key
PRIVATE_KEY="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
ANVIL_URL="http://localhost:8545"

echo -e "\n🔍 Fetching state update for block $BLOCK_NUMBER..."

# Fetch state update from RPC with correct params structure
STATE_UPDATE=$(curl -s -X POST -H "Content-Type: application/json" --data "{
    \"jsonrpc\":\"2.0\",
    \"method\":\"starknet_getStateUpdate\",
    \"params\": {
        \"block_id\": {
            \"block_number\": $BLOCK_NUMBER
        }
    },
    \"id\":1
}" "$RPC_URL")

# Extract global root and block hash from the response
GLOBAL_ROOT=$(echo "$STATE_UPDATE" | jq -r '.result.new_root')
BLOCK_HASH=$(echo "$STATE_UPDATE" | jq -r '.result.block_hash')

if [ "$GLOBAL_ROOT" == "null" ] || [ "$BLOCK_HASH" == "null" ]; then
    echo "Error: Failed to fetch state update data"
    echo "Response: $STATE_UPDATE"
    exit 1
fi

echo -e "\n📊 State Update Data:"
echo "   Global Root: $GLOBAL_ROOT"
echo "   Block Hash: $BLOCK_HASH"
echo ""

# Deploy the verifier contract using forge create
echo -e "🚀 Deploying verifier contract...\n"
VERIFIER_RESULT=$(forge create \
    --rpc-url "$ANVIL_URL" \
    --private-key "$PRIVATE_KEY" \
    "scripts/artifacts/eth/MockGPSVerifier.sol:MockGPSVerifier" \
    2>&1)

if [ $? -ne 0 ]; then
    echo "Error deploying verifier contract:"
    echo "$VERIFIER_RESULT"
    exit 1
fi

# Extract contract address from forge create output
VERIFIER_ADDRESS=$(echo "$VERIFIER_RESULT" | grep "Deployed to" | awk '{print $3}')
echo -e "📦 Verifier deployed at: $VERIFIER_ADDRESS\n"

# Now deploy the main Starknet contract
echo -e "🚀 Deploying Starknet contract...\n"

# Extract bytecode from the JSON file
BYTECODE=$(jq -r '.bytecode.object' "$ABI_FILE" | sed 's/^0x//')

if [ "$BYTECODE" == "null" ] || [ -z "$BYTECODE" ]; then
    echo "Error: No bytecode found in the JSON file"
    exit 1
fi

# Deploy the contract using cast
RESULT=$(cast send \
    --private-key "$PRIVATE_KEY" \
    --rpc-url "$ANVIL_URL" \
    --create "0x$BYTECODE" \
    2>&1)

# Check if deployment was successful
if [ $? -eq 0 ]; then
    # Extract contract address from the result using grep and awk
    CONTRACT_ADDRESS=$(echo "$RESULT" | grep "contractAddress" | awk '{print $2}')
    
    if [ -n "$CONTRACT_ADDRESS" ]; then
        echo -e "📦 Starknet contract deployed successfully at: $CONTRACT_ADDRESS\n"

        # sleep for 2 seconds
        sleep 2

        # Initialize the contract with the required data
        echo -e "🔧 Initializing contract...\n"

        # Create the initialization data
        PROGRAM_HASH="853638403225561750106379562222782223909906501242604214771127703946595519856"
        AGGREGATOR_PROGRAM_HASH="0"
        CONFIG_HASH="1773546093672122186726825451867439478968296982619761985456743675021283370179"

        # Encode the initialization data
        INIT_DATA=$(cast abi-encode "f(uint256,uint256,address,uint256,uint256,int256,uint256)" \
            $PROGRAM_HASH \
            $AGGREGATOR_PROGRAM_HASH \
            $VERIFIER_ADDRESS \
            $CONFIG_HASH \
            $GLOBAL_ROOT \
            $BLOCK_NUMBER \
            $BLOCK_HASH)

        # Call initializeContractState
        INIT_RESULT=$(cast send \
            --private-key "$PRIVATE_KEY" \
            --rpc-url "$ANVIL_URL" \
            $CONTRACT_ADDRESS \
            "initializeContractState(bytes)" \
            $INIT_DATA)

        if [ $? -eq 0 ]; then
            TX_HASH=$(echo "$INIT_RESULT" | grep "transactionHash" | awk '{print $2}')
            echo -e "✅ Contract initialized successfully!"
            echo -e "   Transaction: $TX_HASH\n"
        else
            echo -e "❌ Error initializing contract\n"
            echo "$INIT_RESULT"
            exit 1
        fi
    else
        echo "❌ Error: Could not extract contract address from output"
        exit 1
    fi
else
    echo "❌ Error deploying contract:"
    echo "$RESULT"
    exit 1
fi