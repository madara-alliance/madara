# Copyright 2026 Madara contributors. SPDX-License-Identifier: Apache-2.0
ARG BASE_BUILDER=oss-verify-base-builder
FROM ${BASE_BUILDER}
ARG REPO
ARG COMMIT
COPY . $SRC/madara
RUN lay-out-sources $SRC/madara/.oss-verify/sources $WORK && cp $SRC/madara/.oss-verify/build.sh $SRC/build.sh
WORKDIR $SRC
