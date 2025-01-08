#!/bin/bash
# This script is the same as `e2e-coverage` but does not run with llvm-cov. Use this for faster testing, because
# `e2e-coverage` will likely rebuild everything.
# Usage: ``./scripts/e2e-tests.sh <name of the tests to run>`
set -e

# Configuration
export PROPTEST_CASES=10
export ETH_FORK_URL=https://eth.merkle.io

# Build the binary
cargo build --bin madara --profile dev
export BINARY_PATH=$(realpath target/debug/madara)

# Run the tests
cargo test --profile dev "${@:-"--workspace"}"
