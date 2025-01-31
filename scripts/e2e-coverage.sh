#!/bin/bash
set -e

# Configuration
export PROPTEST_CASES=5
export ETH_FORK_URL=https://eth.merkle.io

# Clean up previous coverage data
rm -f target/madara-* lcov.info

# Set up LLVM coverage environment
source <(cargo llvm-cov show-env --export-prefix)

# Build the binary with coverage instrumentation
cargo build --bin madara --profile dev
export COVERAGE_BIN=$(realpath target/debug/madara)

# Run tests with coverage collection
if cargo test --profile dev "${@:-"--workspace"}"; then
  echo "✅ All tests passed successfully!"
else
  echo "❌ Some tests failed."
fi

# Generate coverage reports
cargo llvm-cov report --lcov --output-path lcov.info    # Generate LCOV report
cargo llvm-cov report   # Display coverage summary in terminal