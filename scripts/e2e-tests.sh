#!/bin/bash
# This script is the same as `e2e-coverage` but does not run with llvm-cov. Use this for faster testing, because
# `e2e-coverage` will likely rebuild everything.
# Usage: ``./scripts/e2e-tests.sh <name of the tests to run>`
set -e

# will also launch anvil and automatically close it down on error or success

anvil --fork-url https://eth.merkle.io --fork-block-number 20395662 &

subshell() {
    set -e
    cargo build --bin madara --profile dev

    export COVERAGE_BIN=$(realpath target/debug/madara)
    export ETH_FORK_URL=https://eth.merkle.io

    # wait for anvil
    while ! nc -z localhost 8545; do
        sleep 1
    done

    cargo test --profile dev --workspace $@
}

(subshell $@ && r=$?) || r=$?
pkill -P $$
exit $r
