#!/bin/bash
set -e

rm -f target/madara-* lcov.info

source <(cargo llvm-cov show-env --export-prefix)

cargo build --bin deoxys --profile dev

export COVERAGE_BIN=$(realpath target/debug/deoxys)
cargo test --profile dev

cargo llvm-cov report --lcov --output-path lcov.info
# cargo llvm-cov report
