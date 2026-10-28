# Step 0: setup tooling (rust)
FROM rust:1.89-bookworm AS base-rust
WORKDIR /app

COPY security/distrust-emsign.py /tmp/distrust-emsign.py
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates openssl python3 python3-venv \
    && python3 /tmp/distrust-emsign.py --debian \
    && rm -rf /var/lib/apt/lists/*

# Note that we do not install cargo chef and sccache through docker to avoid
# having to compile them from source
ENV SCCACHE_VERSION=v0.10.0
ENV SCCACHE_URL=https://github.com/mozilla/sccache/releases/download/${SCCACHE_VERSION}/sccache-${SCCACHE_VERSION}-x86_64-unknown-linux-musl.tar.gz
ENV SCCACHE_TAR=sccache-${SCCACHE_VERSION}-x86_64-unknown-linux-musl.tar.gz
ENV SCCACHE_BIN=/bin/sccache
ENV SCCACHE_DIR=/sccache
ENV SCCACHE=sccache-${SCCACHE_VERSION}-x86_64-unknown-linux-musl/sccache

ENV CHEF_VERSION=v0.1.71
ENV CHEF_URL=https://github.com/LukeMathWalker/cargo-chef/releases/download/${CHEF_VERSION}/cargo-chef-x86_64-unknown-linux-gnu.tar.gz
ENV CHEF_TAR=cargo-chef-x86_64-unknown-linux-gnu.tar.gz

ENV RUSTC_WRAPPER=/bin/sccache

# Keep published images portable; specialized builders can opt into a newer CPU baseline.
ARG MADARA_RUSTFLAGS=""
ENV RUSTFLAGS="${MADARA_RUSTFLAGS}"

ENV WGET="-O- --timeout=10 --waitretry=3 --retry-connrefused --progress=dot:mega"

# Copy Makefile for LLVM installation
COPY Makefile /tmp/Makefile

# Install LLVM 19 for Cairo Native support using Makefile target
RUN apt-get update -y && apt-get install -y wget gnupg software-properties-common make && \
    cd /tmp && make -f Makefile install-llvm19 CODENAME=bookworm && \
    apt-get clean && rm -rf /var/lib/apt/lists/*

# Set LLVM environment variables for Cairo Native
ENV MLIR_SYS_190_PREFIX=/usr/lib/llvm-19
ENV LLVM_SYS_191_PREFIX=/usr/lib/llvm-19
ENV TABLEGEN_190_PREFIX=/usr/lib/llvm-19
ENV PATH="/usr/lib/llvm-19/bin:${PATH}"

# Set explicit compilers for Cairo Native runtime compilation
# Note: clang-19 handles both C and C++ compilation
ENV CC=clang-19
ENV CXX=clang-19

# Configure linker search paths for Cairo Native runtime compilation
ENV LIBRARY_PATH=/usr/lib/x86_64-linux-gnu:/usr/lib:/lib/x86_64-linux-gnu:/lib
ENV LD_LIBRARY_PATH=/usr/lib/x86_64-linux-gnu:/usr/lib:/lib/x86_64-linux-gnu:/lib

# Use Bookworm Python 3.11, which supports the patched pip release.

RUN wget $SCCACHE_URL && tar -xvpf $SCCACHE_TAR && mv $SCCACHE $SCCACHE_BIN && mkdir sccache
RUN wget $CHEF_URL && tar -xvpf $CHEF_TAR && mv cargo-chef /bin

# Step 1: Cache dependencies
FROM base-rust AS planner

COPY . .
RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry \
    RUST_BUILD_DOCKER=1 cargo chef prepare --recipe-path recipe.json

# TODO(prakhar,20/12/2025): Find a proper fix for this!
# Fetch cargo dependencies and extract sequencer requirements.txt
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo fetch --locked && \
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
# Local TLS patches are not workspace members and are omitted from the chef recipe.
COPY security security
COPY --from=planner /app/sequencer-requirements.txt /app/sequencer-requirements.txt

# Setup Python virtual environment with sequencer requirements
RUN python3 -m venv /app/sequencer_venv && \
    . /app/sequencer_venv/bin/activate && \
    python /app/security/bootstrap-python.py && \
    echo "Installing dependencies from sequencer requirements.txt:" && \
    cat /app/sequencer-requirements.txt && \
    sed -i -E 's/^certifi==[^[:space:]]+/certifi==2026.10.19/' /app/sequencer-requirements.txt && \
    pip install -r /app/sequencer-requirements.txt -c /app/security/python-constraints.txt && \
    python /app/security/distrust-emsign.py --check-bundle "$(python -m certifi)" && \
    python /app/security/distrust-emsign.py --check-bundle "$(python -c 'import pip._vendor.certifi; print(pip._vendor.certifi.where())')"

RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry \
    . /app/sequencer_venv/bin/activate && \
    CARGO_TARGET_DIR=target RUST_BUILD_DOCKER=1 cargo chef cook --locked --profile production --recipe-path recipe.json

COPY Cargo.toml Cargo.lock .db-versions.yml ./
COPY madara madara
COPY orchestrator orchestrator
COPY e2e e2e
COPY e2e-tests e2e-tests
COPY build-artifacts build-artifacts
COPY test_utils test_utils

RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry \
    . /app/sequencer_venv/bin/activate && \
    CARGO_TARGET_DIR=target RUST_BUILD_DOCKER=1 cargo build --locked --profile production --bin madara

# Step 3: runner
FROM debian:bookworm-slim AS runner

COPY security/distrust-emsign.py /tmp/distrust-emsign.py
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates openssl python3 python3-venv \
    && python3 /tmp/distrust-emsign.py --debian \
    && rm -rf /var/lib/apt/lists/*


# Copy Makefile for LLVM installation
COPY Makefile /tmp/Makefile

# Install LLVM 19 for Cairo Native runtime compilation (compiles Sierra to native on-the-fly)
RUN apt-get -y update && \
    apt-get install -y openssl ca-certificates tini curl wget gnupg software-properties-common make && \
    cd /tmp && make -f Makefile install-llvm19 CODENAME=bookworm && \
    apt-get autoremove -y && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/* /tmp/Makefile

# Set LLVM environment variables for Cairo Native runtime
ENV MLIR_SYS_190_PREFIX=/usr/lib/llvm-19
ENV LLVM_SYS_191_PREFIX=/usr/lib/llvm-19
ENV TABLEGEN_190_PREFIX=/usr/lib/llvm-19
ENV PATH="/usr/lib/llvm-19/bin:${PATH}"

# Set compilers for Cairo Native runtime compilation
ENV CC=clang-19
ENV CXX=clang-19

# Configure linker search paths for Cairo Native runtime compilation
ENV LIBRARY_PATH=/usr/lib/x86_64-linux-gnu:/usr/lib:/lib/x86_64-linux-gnu:/lib
ENV LD_LIBRARY_PATH=/usr/lib/x86_64-linux-gnu:/usr/lib:/lib/x86_64-linux-gnu:/lib

WORKDIR /app

COPY --from=builder-rust /app/target/production/madara /bin

ENV TINI_VERSION=v0.19.0
ADD https://github.com/krallin/tini/releases/download/${TINI_VERSION}/tini /bin/tini
RUN chmod +x /bin/tini

# Set the entrypoint
ENTRYPOINT ["tini", "--", "madara"]
CMD ["--help"]
