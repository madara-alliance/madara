#!/bin/bash
# This script is the same as `e2e-coverage` but does not run with llvm-cov. Use this for faster testing, because
# `e2e-coverage` will likely rebuild everything.
set -e

# will also launch anvil and automatically close it down on error or success

anvil --fork-url https://eth.merkle.io --fork-block-number 20395662 &

subshell() {
    set -e
    cargo build --bin madara --profile dev
    cargo test --profile dev
}

(subshell && r=$?) || r=$?
pkill -P $$
exit $r
