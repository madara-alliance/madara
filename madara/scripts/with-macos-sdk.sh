#!/usr/bin/env bash

set -euo pipefail

if [[ $# -eq 0 ]]; then
  echo "usage: $0 <command> [args...]" >&2
  exit 2
fi

if [[ "$(uname -s)" == "Darwin" ]]; then
  sdkroot="$(xcrun --sdk macosx --show-sdk-path)"
  export SDKROOT="${SDKROOT:-$sdkroot}"
  export BINDGEN_EXTRA_CLANG_ARGS="${BINDGEN_EXTRA_CLANG_ARGS:-"--sysroot ${SDKROOT}"}"
  export CFLAGS="${CFLAGS:-} --sysroot ${SDKROOT}"
  export CXXFLAGS="${CXXFLAGS:-} --sysroot ${SDKROOT}"
fi

exec "$@"
