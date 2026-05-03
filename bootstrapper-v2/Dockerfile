# Step 0: setup tooling (rust)
FROM rust:1.89-bookworm AS base-rust
WORKDIR /app

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

# Install LLVM 19 for Cairo Native support (required by transitive workspace deps
# pulled in during `cargo chef cook`, notably tblgen/llvm-sys/mlir-sys).
RUN apt-get update -y && apt-get install -y \
  bash \
  build-essential \
  ca-certificates \
  clang \
  curl \
  git \
  gnupg \
  libgmp3-dev \
  libssl-dev \
  make \
  openssl \
  software-properties-common \
  wget && \
  cd /tmp && make -f Makefile install-llvm19 CODENAME=bookworm && \
  apt-get clean && rm -rf /var/lib/apt/lists/*

# Set LLVM environment variables for Cairo Native
ENV MLIR_SYS_190_PREFIX=/usr/lib/llvm-19
ENV LLVM_SYS_191_PREFIX=/usr/lib/llvm-19
ENV TABLEGEN_190_PREFIX=/usr/lib/llvm-19
ENV PATH="/usr/lib/llvm-19/bin:${PATH}"

# Set explicit compilers for Cairo Native runtime compilation
ENV CC=clang-19
ENV CXX=clang++-19

# Configure linker search paths for Cairo Native runtime compilation.
# DEB_HOST_MULTIARCH resolves to the correct lib directory for the build runner architecture.
RUN DEB_HOST_MULTIARCH="$(dpkg-architecture -qDEB_HOST_MULTIARCH)" && \
  printf '%s\n' \
    "export LIBRARY_PATH=/usr/lib/${DEB_HOST_MULTIARCH}:/usr/lib:/lib/${DEB_HOST_MULTIARCH}:/lib" \
    "export LD_LIBRARY_PATH=/usr/lib/${DEB_HOST_MULTIARCH}:/usr/lib:/lib/${DEB_HOST_MULTIARCH}:/lib" \
    > /etc/profile.d/cairo-native-libs.sh
# Use bash (non-login) so BASH_ENV is honored. A login shell would source
# /etc/profile, which on Debian unconditionally resets PATH and would wipe
# /usr/local/cargo/bin and /usr/lib/llvm-19/bin from PATH for every later RUN.
ENV BASH_ENV=/etc/profile.d/cairo-native-libs.sh
SHELL ["/bin/bash", "-c"]

# Install Python 3.9
RUN wget https://www.python.org/ftp/python/3.9.16/Python-3.9.16.tgz \
    && tar xzf Python-3.9.16.tgz \
    && cd Python-3.9.16 \
    && ./configure --enable-optimizations \
    && make altinstall \
    && cd .. \
    && rm -rf Python-3.9.16 Python-3.9.16.tgz

# Install pip and venv for Python 3.9
RUN wget https://bootstrap.pypa.io/pip/3.9/get-pip.py \
    && python3.9 get-pip.py \
    && rm get-pip.py \
    && python3.9 -m pip install virtualenv

RUN wget $SCCACHE_URL && tar -xvpf $SCCACHE_TAR && mv $SCCACHE $SCCACHE_BIN && mkdir sccache
RUN wget $CHEF_URL && tar -xvpf $CHEF_TAR && mv cargo-chef /bin

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
FROM debian:bookworm-slim AS runner

RUN apt-get -y update && \
    apt-get install -y openssl ca-certificates tini curl &&\
    apt-get autoremove -y; \
    apt-get clean; \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder-rust /app/target/release/bootstrapper-v2 /bin
COPY --from=builder-rust /app/build-artifacts /app/build-artifacts

ENV TINI_VERSION=v0.19.0
ADD https://github.com/krallin/tini/releases/download/${TINI_VERSION}/tini /bin/tini
RUN chmod +x /bin/tini

# Set the entrypoint
ENTRYPOINT ["tini", "--", "bootstrapper-v2"]
CMD ["--help"]
