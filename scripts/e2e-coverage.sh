#!/bin/bash
set -e

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
exit $r
