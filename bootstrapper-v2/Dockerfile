ARG BOOTSTRAPPER_V2_BUILDER_BASE=ghcr.io/madara-alliance/bootstrapper-v2-rust-builder-base:rust-1.89-bookworm-llvm19-python3.9.16-sccache0.10.0-chef0.1.71-v1
ARG BOOTSTRAPPER_V2_RUNTIME_BASE=ghcr.io/madara-alliance/bootstrapper-v2-runtime-base:bookworm-tini0.19-v1
FROM ${BOOTSTRAPPER_V2_BUILDER_BASE} AS base-rust
ARG SCCACHE_DIR=/sccache
WORKDIR /app

SHELL ["/bin/bash", "-c"]

# Step 1: Cache dependencies
FROM base-rust AS planner

COPY . .
RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry \
    RUST_BUILD_DOCKER=1 cargo chef prepare --recipe-path recipe.json

# Fetch cargo dependencies and extract sequencer requirements.txt
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo fetch && \
    REQUIREMENTS_FILE=$(find /usr/local/cargo/git/checkouts -path "*sequencer*/scripts/requirements.txt" 2>/dev/null | head -1) && \
    if [ -z "$REQUIREMENTS_FILE" ]; then \
        echo "ERROR: Could not find requirements.txt in sequencer checkout"; \
        exit 1; \
    fi && \
    echo "Found requirements.txt at: $REQUIREMENTS_FILE" && \
    cp "$REQUIREMENTS_FILE" /app/sequencer-requirements.txt && \
    cat /app/sequencer-requirements.txt

# Step 2: Build crate
FROM base-rust AS builder-rust

COPY --from=planner /app/recipe.json recipe.json
COPY --from=planner /app/sequencer-requirements.txt /app/sequencer-requirements.txt

# Setup Python virtual environment with sequencer requirements
RUN python3.9 -m venv /app/sequencer_venv && \
    . /app/sequencer_venv/bin/activate && \
    pip install --upgrade pip && \
    pip install -r /app/sequencer-requirements.txt

RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry \
    . /app/sequencer_venv/bin/activate && \
    CARGO_TARGET_DIR=target RUST_BUILD_DOCKER=1 cargo chef cook --release --recipe-path recipe.json

COPY Cargo.toml Cargo.lock ./
COPY bootstrapper-v2 bootstrapper-v2
COPY build-artifacts build-artifacts

RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry \
    . /app/sequencer_venv/bin/activate && \
    CARGO_TARGET_DIR=target RUST_BUILD_DOCKER=1 cargo build --manifest-path bootstrapper-v2/Cargo.toml --release

# Step 3: runner
ARG BOOTSTRAPPER_V2_RUNTIME_BASE
FROM ${BOOTSTRAPPER_V2_RUNTIME_BASE} AS runner

WORKDIR /app

COPY --from=builder-rust /app/target/release/bootstrapper-v2 /bin
COPY --from=builder-rust /app/build-artifacts /app/build-artifacts

# Set the entrypoint
ENTRYPOINT ["tini", "--", "bootstrapper-v2"]
CMD ["--help"]
