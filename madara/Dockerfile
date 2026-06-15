ARG MADARA_BUILDER_BASE=ghcr.io/madara-alliance/madara-rust-builder-base:rust-1.89-bookworm-llvm19-python3.9.16-sccache0.10.0-chef0.1.71-v1
ARG MADARA_RUNTIME_BASE=ghcr.io/madara-alliance/madara-runtime-base:bookworm-llvm19-tini0.19-v1
FROM ${MADARA_BUILDER_BASE} AS base-rust
ARG SCCACHE_DIR=/sccache
WORKDIR /app

# Step 1: Cache dependencies
FROM base-rust AS planner

COPY . .
RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry \
    RUST_BUILD_DOCKER=1 cargo chef prepare --recipe-path recipe.json

# TODO(prakhar,20/12/2025): Find a proper fix for this!
# Fetch cargo dependencies and extract sequencer requirements.txt
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo fetch && \
    echo "Looking for sequencer requirements.txt in cargo git checkouts..." && \
    REQUIREMENTS_FILE=$(find /usr/local/cargo/git/checkouts -path "*sequencer*/scripts/requirements.txt" 2>/dev/null | head -1) && \
    if [ -z "$REQUIREMENTS_FILE" ]; then \
        echo "ERROR: Could not find requirements.txt in sequencer checkout"; \
        echo "Available sequencer directories:"; \
        find /usr/local/cargo/git/checkouts -type d -name "*sequencer*" 2>/dev/null | head -10; \
        echo "Looking for any requirements.txt files:"; \
        find /usr/local/cargo/git/checkouts -name "requirements.txt" 2>/dev/null | head -5; \
        exit 1; \
    fi && \
    echo "✓ Found requirements.txt at: $REQUIREMENTS_FILE" && \
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
    echo "Installing dependencies from sequencer requirements.txt:" && \
    cat /app/sequencer-requirements.txt && \
    pip install -r /app/sequencer-requirements.txt

RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry \
    . /app/sequencer_venv/bin/activate && \
    CARGO_TARGET_DIR=target RUST_BUILD_DOCKER=1 cargo chef cook --release --recipe-path recipe.json

COPY Cargo.toml Cargo.lock ./
COPY madara madara
COPY build-artifacts build-artifacts
COPY .db-versions.yml .db-versions.yml

RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry \
    . /app/sequencer_venv/bin/activate && \
    CARGO_TARGET_DIR=target RUST_BUILD_DOCKER=1 cargo build --release --bin madara

# Step 3: runner
ARG MADARA_RUNTIME_BASE
FROM ${MADARA_RUNTIME_BASE} AS runner

WORKDIR /app

COPY --from=builder-rust /app/target/release/madara /bin

# Set the entrypoint
ENTRYPOINT ["tini", "--", "madara"]
CMD ["--help"]
