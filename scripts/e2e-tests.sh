#!/bin/bash
# This script is the same as `e2e-coverage` but does not run with llvm-cov. Use this for faster testing, because
# `e2e-coverage` will likely rebuild everything.
# Usage: `./scripts/e2e-tests.sh <name of the tests to run>`
set -e

# Configuration
export PROPTEST_CASES=10
export ETH_FORK_URL=https://eth.merkle.io
export COVERAGE_BIN=$(realpath target/debug/madara)

export ANVIL_URL=http://localhost:8545
export ANVIL_FORK_BLOCK_NUMBER=20395662
export ANVIL_DEFAULT_PORT=8545

subshell() {
  # We need to build madara first so that we can launch it in mc-e2e-tests.
  cargo build --manifest-path madara/Cargo.toml --bin madara --profile dev

  # Run the tests
  if cargo nextest run "${@:-"--workspace"}"; then
    echo "✅ All tests passed successfully!"
  else
    echo "❌ Some tests failed."
    exit 1
  fi
}

# Launch anvil
anvil --fork-url https://eth.merkle.io --fork-block-number 20395662 &

(subshell $@ && r=$?) || r=$?
pkill -P $$
exit $r
