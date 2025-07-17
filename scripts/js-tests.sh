#!/bin/bash

CARGO_TARGET_DIR=target cargo build --manifest-path madara/Cargo.toml  --bin madara --release
./target/release/madara    \
  --name madara            \
  --base-path ../madara_db \
  --rpc-port 9944          \
  --rpc-cors "*"           \
  --rpc-external           \
  --devnet                 \
  --preset devnet          \
  --l1-gas-price 0         \
  --blob-gas-price 0       \
  --strk-per-eth 1         \
  --no-l1-sync &

MADARA_PID=$!

while ! echo exit | nc localhost 9944
  do sleep 1;
done

cd tests/js_tests
npm install
npm test

kill $MADARA_PID
