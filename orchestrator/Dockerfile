# Step 0: setup tooling (rust)
FROM rust:1.89-bookworm AS base-rust
WORKDIR /app

COPY security/distrust-emsign.py /tmp/distrust-emsign.py
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates openssl python3 python3-venv \
    && python3 /tmp/distrust-emsign.py --debian \
    && rm -rf /var/lib/apt/lists/*

# Note that we do not install sccache through cargo to avoid
# having to compile them from source
ENV SCCACHE_VERSION=v0.10.0
ENV SCCACHE_URL=https://github.com/mozilla/sccache/releases/download/${SCCACHE_VERSION}/sccache-${SCCACHE_VERSION}-x86_64-unknown-linux-musl.tar.gz
ENV SCCACHE_TAR=sccache-${SCCACHE_VERSION}-x86_64-unknown-linux-musl.tar.gz
ENV SCCACHE_BIN=/bin/sccache
ENV SCCACHE_DIR=/sccache
ENV SCCACHE=sccache-${SCCACHE_VERSION}-x86_64-unknown-linux-musl/sccache

ENV RUSTC_WRAPPER=/bin/sccache

ENV WGET="-O- --timeout=10 --waitretry=3 --retry-connrefused --progress=dot:mega"

# Copy Makefile for LLVM installation
COPY Makefile /tmp/Makefile

# Install LLVM 19 for Cairo Native support using Makefile target
RUN apt-get update -y && apt-get install -y \
  bash \
  build-essential \
  ca-certificates \
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

# Install BLST from source
RUN git clone --depth 1 --branch v0.3.15 https://github.com/supranational/blst.git /tmp/blst && \
  cd /tmp/blst && \
  ./build.sh && \
  mkdir -p /usr/local/lib /usr/local/include && \
  cp libblst.a /usr/local/lib/ && \
  cp bindings/blst.h /usr/local/include/ && \
  cp bindings/blst_aux.h /usr/local/include/ && \
  rm -rf /tmp/blst

# ENV to link BLST dependency
ENV RUSTFLAGS="-L /usr/local/lib -l blst"

# Update linker cache
RUN echo "/usr/local/lib" > /etc/ld.so.conf.d/blst.conf && ldconfig

# Use Bookworm Python 3.11, which supports the patched pip release.

RUN wget $SCCACHE_URL && tar -xvpf $SCCACHE_TAR && mv $SCCACHE $SCCACHE_BIN && mkdir sccache

# Step 1: Cache dependencies
FROM base-rust AS planner

COPY . .

# Fetch cargo dependencies and extract sequencer requirements.txt
RUN --mount=type=cache,target=/usr/local/cargo/registry \
  cargo fetch --locked && \
  REQUIREMENTS_FILE=$(find /usr/local/cargo/git/checkouts -name "requirements.txt" -path "*/scripts/requirements.txt" 2>/dev/null | grep -i sequencer | head -1) && \
  if [ -z "$REQUIREMENTS_FILE" ]; then \
  echo "Searching more broadly for requirements.txt..."; \
  REQUIREMENTS_FILE=$(find /usr/local/cargo/git/checkouts -name "requirements.txt" 2>/dev/null | head -1); \
  fi && \
  if [ -n "$REQUIREMENTS_FILE" ] && [ -f "$REQUIREMENTS_FILE" ]; then \
  echo "✓ Found requirements.txt at: $REQUIREMENTS_FILE"; \
  cp "$REQUIREMENTS_FILE" /app/sequencer-requirements.txt; \
  cat /app/sequencer-requirements.txt; \
  else \
  echo "ERROR: Could not find requirements.txt in cargo checkouts"; \
  exit 1; \
  fi

# Step 2: Build crate
FROM base-rust AS builder-rust

COPY . .
COPY --from=planner /app/sequencer-requirements.txt /app/sequencer-requirements.txt

# Setup Python virtual environment with sequencer requirements
RUN python3 -m venv /app/sequencer_venv && \
  . /app/sequencer_venv/bin/activate && \
  pip --use-deprecated=legacy-certs --cert /etc/ssl/certs/ca-certificates.crt install --upgrade pip==26.2.2 certifi==2026.10.19 && \
  echo "Installing dependencies from sequencer requirements.txt:" && \
  cat /app/sequencer-requirements.txt && \
  pip install -r /app/sequencer-requirements.txt -c /app/security/python-constraints.txt && \
    python /app/security/distrust-emsign.py --check-bundle "$(python -m certifi)" && \
    python /app/security/distrust-emsign.py --check-bundle "$(python -c 'import pip._vendor.certifi; print(pip._vendor.certifi.where())')"

# Compile the actual vendored verifier sources; cargo-chef skeletons must not replace them.
RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
  --mount=type=cache,target=/usr/local/cargo/registry \
  . /app/sequencer_venv/bin/activate && \
  CARGO_TARGET_DIR=target RUST_BUILD_DOCKER=1 cargo build --locked --manifest-path orchestrator/Cargo.toml --release

# Dump information where kzg files must be copied
RUN mkdir -p /tmp && \
  (find /usr/local/cargo -type d -path "*/crates/starknet-os/kzg" > /tmp/kzg_dirs.txt 2>/dev/null || \
  find $HOME/.cargo -type d -path "*/crates/starknet-os/kzg" >> /tmp/kzg_dirs.txt 2>/dev/null || \
  touch /tmp/kzg_dirs.txt)

# Step 3: runner
FROM debian:bookworm-slim AS runner

COPY security/distrust-emsign.py /tmp/distrust-emsign.py
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates openssl python3 python3-venv \
    && python3 /tmp/distrust-emsign.py --debian \
    && rm -rf /var/lib/apt/lists/*


RUN apt-get -y update && \
  apt-get install -y openssl ca-certificates tini curl jq && \
  apt-get install -y --no-install-recommends xz-utils libatomic1 && \
  test "$(dpkg --print-architecture)" = amd64 && \
  curl --fail --show-error --location https://nodejs.org/dist/v22.23.4/node-v22.23.4-linux-x64.tar.xz -o /tmp/node.tar.xz && \
  echo "1805bcd35e6815559bd2eb97db438d3fdb0eb3a5e6a977b7c1badca76337e9ce  /tmp/node.tar.xz" | sha256sum --check - && \
  tar -xJf /tmp/node.tar.xz -C /usr/local --strip-components=1 && \
  test "$(node --version)" = v22.23.4 && \
  rm /tmp/node.tar.xz && apt-get clean && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder-rust /app/target/release/orchestrator /bin

# Copy trusted setup files
COPY --from=builder-rust /app/orchestrator/crates/settlement-clients/ethereum/src/trusted_setup.txt \
  /app/orchestrator/crates/settlement-clients/ethereum/src/trusted_setup.txt
COPY --from=builder-rust /app/orchestrator/crates/settlement-clients/ethereum/src/trusted_setup.txt /tmp/trusted_setup.txt
COPY --from=builder-rust /tmp/kzg_dirs.txt /tmp/kzg_dirs.txt

RUN while read dir; do \
  mkdir -p "$dir" && \
  cp /tmp/trusted_setup.txt "$dir/trusted_setup.txt"; \
  done < /tmp/kzg_dirs.txt

# Copy migration files and install dependencies
COPY --from=builder-rust /app/orchestrator/migrations /app/migrations
COPY --from=builder-rust /app/orchestrator/package.json /app/package.json
COPY --from=builder-rust /app/orchestrator/migrate-mongo-config.js /app/migrate-mongo-config.js

# Install npm dependencies for migrations
RUN npm install --omit=dev

# Create entrypoint script that runs migrations before starting orchestrator
# Note: migrate-mongo handles distributed locking via lockCollectionName and lockTtl config
RUN echo '#!/bin/bash\n\
  set -e\n\
  \n\
  # Run database migrations if MongoDB connection is configured\n\
  if [ -n "$MADARA_ORCHESTRATOR_MONGODB_CONNECTION_URL" ] && [ -n "$MADARA_ORCHESTRATOR_DATABASE_NAME" ]; then\n\
  echo "Running database migrations..."\n\
  cd /app && npx migrate-mongo up\n\
  echo "Migrations complete"\n\
  fi\n\
  \n\
  # Start orchestrator\n\
  exec orchestrator "$@"\n\
  ' > /app/entrypoint.sh && chmod +x /app/entrypoint.sh

EXPOSE 3000

ENV TINI_VERSION=v0.19.0
ADD https://github.com/krallin/tini/releases/download/${TINI_VERSION}/tini /bin/tini
RUN chmod +x /bin/tini

# Set the entrypoint
ENTRYPOINT ["tini", "--", "/app/entrypoint.sh"]
CMD ["--help"]
