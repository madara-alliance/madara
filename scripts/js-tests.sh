#!/bin/bash

CARGO_TARGET_DIR=target cargo build --manifest-path madara/Cargo.toml  --bin madara --release

ANVIL_PORT=8545
RPC_PORT=9944
CORE_CONTRACT="0x5fbdb2315678afecb367f032d93f642f64180aa3"

# Start Anvil (L1 simulator) with 1-second block time
if command -v anvil &> /dev/null; then
  echo "Starting Anvil on port $ANVIL_PORT..."
  anvil --port $ANVIL_PORT --block-time 1 --chain-id 1337 --slots-in-an-epoch 1 --silent &
  ANVIL_PID=$!
  sleep 2

  # Deploy the DummyContract (messaging + state stubs)
  BYTECODE=$(cat tests/js_tests/fixtures/DummyContract.bytecode)
  curl -s "http://127.0.0.1:$ANVIL_PORT" -X POST -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_sendTransaction\",\"params\":[{\"from\":\"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266\",\"data\":\"$BYTECODE\",\"gas\":\"0x1000000\"}]}" > /dev/null
  sleep 3
  echo "DummyContract deployed at $CORE_CONTRACT"

  ANVIL_ENV="ANVIL_PORT=$ANVIL_PORT CORE_CONTRACT=$CORE_CONTRACT"
else
  echo "Anvil not found, running without L1 messaging tests"
  ANVIL_ENV=""
fi

if [ -n "$ANVIL_PID" ]; then
  ./target/release/madara    \
    --name madara            \
    --base-path ../madara_db \
    --rpc-port $RPC_PORT     \
    --rpc-cors "*"           \
    --rpc-external           \
    --rpc-admin              \
    --devnet                 \
    --preset devnet          \
    --l1-gas-price 0         \
    --blob-gas-price 0       \
    --strk-per-eth 1         \
    --l1-endpoint "http://127.0.0.1:$ANVIL_PORT" \
    --unsafe-skip-l1-message-consumed-check \
    --rpc-pre-v0-9-preconfirmed-as-pending \
    --chain-config-override "block_time=1h,eth_core_contract_address=$CORE_CONTRACT" &
else
  ./target/release/madara    \
    --name madara            \
    --base-path ../madara_db \
    --rpc-port $RPC_PORT     \
    --rpc-cors "*"           \
    --rpc-external           \
    --rpc-admin              \
    --devnet                 \
    --preset devnet          \
    --l1-gas-price 0         \
    --blob-gas-price 0       \
    --strk-per-eth 1         \
    --no-l1-sync             \
    --rpc-pre-v0-9-preconfirmed-as-pending \
    --chain-config-override block_time=1h &
fi

MADARA_PID=$!

while ! echo exit | nc localhost $RPC_PORT
  do sleep 1;
done

cd tests/js_tests || exit
npm install
eval "$ANVIL_ENV npm test"

kill $MADARA_PID
[ -n "$ANVIL_PID" ] && kill "$ANVIL_PID" 2>/dev/null || true
