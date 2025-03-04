#!/bin/sh

mkdir -p /usr/share/madara/data/classes

# exec tini -- ./madara \
exec ./madara \
  --devnet \
  --chain-config-override=block_time=5s,pending_block_update_time=500ms,mempool_tx_limit=20000,execution_batch_size=64\
  --name Madara \
  --base-path /usr/share/madara/data \
  --gateway-port 8080 \
  --gateway-enable \
  --feeder-gateway-enable \
  --gateway-external \
  --rpc-external \
  --rpc-cors "*" \
  # --sequencer \
  # --preset devnet \
  # --feeder-gateway-enable \
  # --rpc-port 9945 \
  # --gas-price 10 \
  # --blob-gas-price 20 \
  # --l1-endpoint http://anvil:8545