#!/bin/bash

if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
  echo "CARGO_TARGET_DIR is not set. Load your shell profile (e.g. source ~/.zshrc) and retry."
  exit 1
fi
TARGET_DIR="$CARGO_TARGET_DIR"
export CARGO_TARGET_DIR="$TARGET_DIR"

cargo build --manifest-path madara/Cargo.toml  --bin madara --release
MADARA_BIN=$(realpath "$TARGET_DIR/release/madara")
"$MADARA_BIN"              \
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
  --no-l1-sync             \
  --rpc-pre-v0-9-preconfirmed-as-pending &

MADARA_PID=$!

while ! echo exit | nc localhost 9944
  do sleep 1;
done

cd tests/js_tests
npm install
npm test

kill $MADARA_PID
