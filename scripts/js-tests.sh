#!/bin/bash

CARGO_TARGET_DIR=target cargo build --bin madara --release

RPC_PORT=9944

# Start Anvil and deploy DummyContract if anvil is available
if command -v anvil &> /dev/null; then
  source scripts/setup-anvil.sh
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
