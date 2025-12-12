# Step 0: setup tooling (rust)
FROM rust:1.89-bookworm AS base-rust
WORKDIR /app

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

# Install Python 3.9
RUN wget https://www.python.org/ftp/python/3.9.16/Python-3.9.16.tgz \
    && tar xzf Python-3.9.16.tgz \
    && cd Python-3.9.16 \
    && ./configure --enable-optimizations \
    && make altinstall \
    && cd .. \
    && rm -rf Python-3.9.16 Python-3.9.16.tgz

# Install pip and venv for Python 3.9
RUN wget https://bootstrap.pypa.io/get-pip.py \
    && python3.9 get-pip.py \
    && rm get-pip.py \
    && python3.9 -m pip install virtualenv

RUN wget $SCCACHE_URL && tar -xvpf $SCCACHE_TAR && mv $SCCACHE $SCCACHE_BIN && mkdir sccache
RUN wget $CHEF_URL && tar -xvpf $CHEF_TAR && mv cargo-chef /bin

# Step 1: Cache dependencies
FROM base-rust AS planner

COPY . .
# RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
#     --mount=type=cache,target=/usr/local/cargo/registry \
#     RUST_BUILD_DOCKER=1 cargo chef prepare --recipe-path recipe.json

# Fetch cargo dependencies and extract sequencer requirements.txt
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo fetch && \
    SEQUENCER_REV=$(grep -A 1 'blockifier = { git = "https://github.com/karnotxyz/sequencer"' madara/Cargo.toml | grep -oP 'rev = "\K[^"]+' | head -1) && \
    if [ -z "$SEQUENCER_REV" ]; then \
        echo "ERROR: Could not extract sequencer revision from madara/Cargo.toml"; \
        exit 1; \
    fi && \
    SEQUENCER_SHORT_REV=$(echo "$SEQUENCER_REV" | cut -c1-7) && \
    echo "Looking for sequencer requirements.txt with revision: $SEQUENCER_REV (short: $SEQUENCER_SHORT_REV)" && \
    REQUIREMENTS_FILE=$(find /usr/local/cargo/git/checkouts -maxdepth 1 -type d -name "*sequencer*" 2>/dev/null | while read -r checkout; do \
        target_dir=$(find "$checkout" -maxdepth 2 -type d -name "${SEQUENCER_SHORT_REV}*" 2>/dev/null | head -1); \
        if [ -n "$target_dir" ] && [ -f "$target_dir/scripts/requirements.txt" ]; then \
            echo "$target_dir/scripts/requirements.txt"; \
            break; \
        fi; \
    done) && \
    if [ -z "$REQUIREMENTS_FILE" ]; then \
        echo "ERROR: Could not find requirements.txt for sequencer revision $SEQUENCER_REV ($SEQUENCER_SHORT_REV)"; \
        echo "Available sequencer directories:"; \
        find /usr/local/cargo/git/checkouts -type d -name "*sequencer*" 2>/dev/null | head -5; \
        exit 1; \
    fi && \
    if [ -f "$REQUIREMENTS_FILE" ]; then \
        echo "✓ Found requirements.txt at: $REQUIREMENTS_FILE"; \
        cp "$REQUIREMENTS_FILE" /app/sequencer-requirements.txt; \
        cat /app/sequencer-requirements.txt; \
    else \
        echo "ERROR: requirements.txt file not found at: $REQUIREMENTS_FILE"; \
        exit 1; \
    fi

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
    CARGO_TARGET_DIR=target RUST_BUILD_DOCKER=1 cargo build --manifest-path madara/Cargo.toml --release

# Step 3: runner
FROM debian:bookworm-slim AS runner

RUN apt-get -y update && \
    apt-get install -y openssl ca-certificates tini curl &&\
    apt-get autoremove -y; \
    apt-get clean; \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder-rust /app/target/release/madara /bin

ENV TINI_VERSION=v0.19.0
ADD https://github.com/krallin/tini/releases/download/${TINI_VERSION}/tini /bin/tini
RUN chmod +x /bin/tini

# Set the entrypoint
ENTRYPOINT ["tini", "--", "madara"]
CMD ["--help"]
