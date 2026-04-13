#!/bin/bash
# Shared Anvil + DummyContract bootstrap for JS RPC tests.
# Starts Anvil, deploys the combined DummyContract (messaging + state stubs),
# and exports ANVIL_PID, ANVIL_PORT, CORE_CONTRACT for the caller.
#
# Usage:
#   source scripts/setup-anvil.sh
#   # Now ANVIL_PID, ANVIL_PORT, CORE_CONTRACT are set

ANVIL_PORT=${ANVIL_PORT:-8545}
CORE_CONTRACT="0x5fbdb2315678afecb367f032d93f642f64180aa3"

echo "Starting Anvil on port $ANVIL_PORT..."
anvil --port "$ANVIL_PORT" --block-time 1 --chain-id 1337 --slots-in-an-epoch 1 > /dev/null 2>&1 &
ANVIL_PID=$!
sleep 2

if ! curl -s "http://127.0.0.1:$ANVIL_PORT" -X POST -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' | grep -q result; then
  echo "ERROR: Anvil failed to start"
  return 1 2>/dev/null || exit 1
fi

echo "Deploying DummyContract on Anvil..."
BYTECODE=$(cat tests/js_tests/fixtures/DummyContract.bytecode)
curl -s "http://127.0.0.1:$ANVIL_PORT" -X POST -H "Content-Type: application/json" \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_sendTransaction\",\"params\":[{\"from\":\"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266\",\"data\":\"$BYTECODE\",\"gas\":\"0x1000000\"}]}" > /dev/null
sleep 3

echo "DummyContract deployed at $CORE_CONTRACT"

export ANVIL_PID ANVIL_PORT CORE_CONTRACT
