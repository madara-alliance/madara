#!/bin/bash
set -e

# will also launch anvil and automatically close it down on error or success

anvil --fork-url https://eth.merkle.io --fork-block-number 20395662 &

subshell() {
    set -e
    rm -f target/madara-* lcov.info

    source <(cargo llvm-cov show-env --export-prefix)

    cargo build --bin madara --profile dev

    export COVERAGE_BIN=$(realpath target/debug/madara)
    cargo test --profile dev

    cargo llvm-cov report --lcov --output-path lcov.info
    cargo llvm-cov report
}

(subshell && r=$?) || r=$?
pkill -P $$
exit $r
