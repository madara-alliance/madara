#!/bin/bash
set -e

if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
  echo "CARGO_TARGET_DIR is not set. Load your shell profile (e.g. source ~/.zshrc) and retry."
  exit 1
fi
TARGET_DIR="$CARGO_TARGET_DIR"
export CARGO_TARGET_DIR="$TARGET_DIR"

# Configuration
export PROPTEST_CASES=5
export ETH_FORK_URL=https://eth.merkle.io

# Clean up previous coverage data
rm -f "$TARGET_DIR"/madara-* lcov.info

# Build the binary with coverage instrumentation
cargo build --manifest-path madara/Cargo.toml  --bin madara --profile dev
export COVERAGE_BIN=$(realpath "$TARGET_DIR/debug/madara")

# Run tests with coverage collection and generate reports in one command
if cargo llvm-cov nextest --profile dev "${@:-"--workspace"}"; then
  echo "✅ All tests passed successfully!"
else
  echo "❌ Some tests failed."
fi

# Generate coverage reports
cargo llvm-cov report --lcov --output-path lcov.info    # Generate LCOV report
cargo llvm-cov report   # Display coverage summary in terminal
