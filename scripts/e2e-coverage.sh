#!/bin/bash
set -e

# Configuration
export PROPTEST_CASES=5
export ETH_FORK_URL=https://eth.merkle.io

# Clean up previous coverage data
rm -f target/madara-* lcov.info

# Build the binary with coverage instrumentation
cargo build --bin madara --profile dev
export COVERAGE_BIN=$(realpath target/debug/madara)

# Run tests with coverage collection and generate reports in one command
if cargo llvm-cov nextest --profile dev "${@:-"--workspace"}"; then
  echo "✅ All tests passed successfully!"
else
  echo "❌ Some tests failed."
fi

# Generate coverage reports
cargo llvm-cov report --lcov --output-path lcov.info    # Generate LCOV report
cargo llvm-cov report   # Display coverage summary in terminal