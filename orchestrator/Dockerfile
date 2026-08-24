ARG ORCHESTRATOR_BUILDER_BASE=ghcr.io/madara-alliance/orchestrator-rust-builder-base:rust-1.89-bookworm-llvm19-python3.9.16-sccache0.10.0-chef0.1.71-blst0.3.15-v1
ARG ORCHESTRATOR_RUNTIME_BASE=ghcr.io/madara-alliance/orchestrator-runtime-base:bookworm-node18-tini0.19-v1
FROM ${ORCHESTRATOR_BUILDER_BASE} AS base-rust
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
COPY orchestrator orchestrator
COPY test_utils test_utils
COPY build-artifacts build-artifacts

# Run cargo build with venv activateed
RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
  --mount=type=cache,target=/usr/local/cargo/registry \
  . /app/sequencer_venv/bin/activate && \
  CARGO_TARGET_DIR=target RUST_BUILD_DOCKER=1 cargo build --manifest-path orchestrator/Cargo.toml --release

# Dump information where kzg files must be copied
RUN mkdir -p /tmp && \
  (find /usr/local/cargo -type d -path "*/crates/starknet-os/kzg" > /tmp/kzg_dirs.txt 2>/dev/null || \
  find $HOME/.cargo -type d -path "*/crates/starknet-os/kzg" >> /tmp/kzg_dirs.txt 2>/dev/null || \
  touch /tmp/kzg_dirs.txt)

# Step 3: runner
ARG ORCHESTRATOR_RUNTIME_BASE
FROM ${ORCHESTRATOR_RUNTIME_BASE} AS runner

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

# Set the entrypoint
ENTRYPOINT ["tini", "--", "/app/entrypoint.sh"]
CMD ["--help"]
