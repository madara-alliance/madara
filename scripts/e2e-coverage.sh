#!/bin/bash
set -e

anvil --fork-url https://eth.merkle.io --fork-block-number 20395662 &

subshell() {
    set -e
    rm -f target/madara-* lcov.info

    source <(cargo llvm-cov show-env --export-prefix)

    cargo build --bin madara --profile dev

    export COVERAGE_BIN=$(realpath target/debug/madara)
    export ETH_FORK_URL=https://eth.merkle.io

    # wait for anvil
    while ! nc -z localhost 8545; do
        sleep 1
    done

    cargo test --profile dev --workspace $@

    cargo llvm-cov report --lcov --output-path lcov.info
    cargo llvm-cov report
}

(subshell $@ && r=$?) || r=$?
pkill -P $$
exit $r
