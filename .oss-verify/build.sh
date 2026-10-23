#!/bin/sh -e
# Copyright 2026 Madara contributors. SPDX-License-Identifier: Apache-2.0
[ -z "${BASH_VERSION:-}" ] || exec sh -e "$0"
. /usr/local/lib/oss-verify/release-target.sh

(
ov_target_begin event_bloom
CARGO_FUZZ_DIR=.oss-verify/fuzz
FUZZ_BIN=event_bloom
CARGO_DEBUG_ASSERTIONS=0
CARGO_FEATURES=""
CARGO_NO_DEFAULT_FEATURES=0
CARGO_PROFILE_ENV=""
OFT_RUST_QUIET_STDIO=0
. /usr/local/lib/oss-verify/rust/recipes.sh
build_lib() {
  oft_build_cargo_shim
  oft_seal_getenv
}
LIB_ARCHIVE="lib/liboft.a"
ov_target_build event_bloom
)
