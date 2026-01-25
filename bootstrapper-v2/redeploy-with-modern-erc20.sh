#!/bin/bash
set -e

echo "==================================================="
echo "  Redeploying Madara with Sierra 1.7.0 ERC20"
echo "==================================================="

# Configuration
MADARA_DIR="/Users/heemankverma/Work/Karnot/RvsC/madara/madara"
BOOTSTRAPPER_DIR="/Users/heemankverma/Work/Karnot/RvsC/madara/bootstrapper-v2"

echo ""
echo "Step 1: Stopping Madara..."
pkill -f "madara.*--sequencer" || echo "Madara not running"
sleep 2

echo ""
echo "Step 2: Cleaning database..."
rm -rf "$MADARA_DIR/db"
echo "Database cleaned"

echo ""
echo "Step 3: Starting Madara with fresh database..."
echo "Please run this in a separate terminal:"
echo "  cd $MADARA_DIR"
echo "  make run-madara-sequencer"
echo ""
read -p "Press ENTER once Madara is running and ready..."

echo ""
echo "Step 4: Running bootstrapper deployment..."
cd "$BOOTSTRAPPER_DIR"
RUST_LOG=info cargo run --release --bin bootstrapper-v2 -- \
  skip-base-layer \
  --madara-rpc-url http://localhost:9944 \
  --output-path output/madara_addresses.json

echo ""
echo "==================================================="
echo "  Deployment Complete!"
echo "==================================================="
echo ""
echo "Checking fee token Sierra version..."
sleep 2

# Get fee token address from output
FEE_TOKEN=$(jq -r '.addresses.l2_eth_token' output/madara_addresses.json)
echo "Fee token address: $FEE_TOKEN"

# Check Sierra version
SIERRA_VERSION=$(curl -s -X POST http://localhost:9944 \
  -H "Content-Type: application/json" \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"starknet_getClassHashAt\",\"params\":{\"block_id\":\"latest\",\"contract_address\":\"$FEE_TOKEN\"},\"id\":1}" \
  | jq -r '.result' \
  | xargs -I {} curl -s -X POST http://localhost:9944 \
  -H "Content-Type: application/json" \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"starknet_getClass\",\"params\":{\"block_id\":\"latest\",\"class_hash\":\"{}\"},\"id\":1}" \
  | jq -r '.result.sierra_program[0:3]')

echo "Sierra version: $SIERRA_VERSION"

if echo "$SIERRA_VERSION" | grep -q "0x7"; then
  echo "✅ SUCCESS: Fee token is using Sierra 1.7.0!"
else
  echo "⚠️  WARNING: Fee token is using an older Sierra version"
fi

echo ""
echo "==================================================="
echo "  Next Steps:"
echo "==================================================="
echo ""
echo "1. Deploy Kamehameha contracts:"
echo "   cd $BOOTSTRAPPER_DIR"
echo "   for contract in SimpleCounter CounterWithEvent Random100Hashes MathBenchmark; do"
echo "     cargo run --release --bin deploy-contract \$contract"
echo "   done"
echo ""
echo "2. Create a new user account:"
echo "   cargo run --release --bin deploy-new-account <your_private_key>"
echo ""
echo "3. Transfer ETH from bootstrap account to user account"
echo ""
echo "All infrastructure contracts deployed at:"
cat output/madara_addresses.json
echo ""
