# Step 0: setup tooling (rust)
FROM rust:1.86 AS base-rust
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

RUN apt-get update -y && \
    apt-get install -y \
    wget \
    clang \
    nodejs \
    npm

RUN wget $SCCACHE_URL && tar -xvpf $SCCACHE_TAR && mv $SCCACHE $SCCACHE_BIN && mkdir sccache
RUN wget $CHEF_URL && tar -xvpf $CHEF_TAR && mv cargo-chef /bin

# Step 1: Cache dependencies
FROM base-rust AS planner

WORKDIR /app
COPY . .
RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry \
    RUST_BUILD_DOCKER=1 cargo chef prepare --recipe-path recipe.json

# Step 2: Build crate
FROM base-rust AS builder-rust

WORKDIR /app

COPY --from=planner /app/recipe.json recipe.json

COPY Cargo.toml Cargo.lock .
COPY build-artifacts build-artifacts
COPY orchestrator orchestrator

RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry \
    CARGO_TARGET_DIR=target RUST_BUILD_DOCKER=1 cargo chef cook --release --recipe-path recipe.json

RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry \
    CARGO_TARGET_DIR=target RUST_BUILD_DOCKER=1 cargo build --bin orchestrator --release

# Install Node.js dependencies for migrations
RUN cd orchestrator && npm install

# Dump information where kzg files must be copied
RUN mkdir -p /tmp && \
    (find /usr/local/cargo -type d -path "*/crates/starknet-os/kzg" > /tmp/kzg_dirs.txt 2>/dev/null || \
     find $HOME/.cargo -type d -path "*/crates/starknet-os/kzg" >> /tmp/kzg_dirs.txt 2>/dev/null || \
     touch /tmp/kzg_dirs.txt)

# Step 3: runner
FROM debian:bookworm-slim AS runner

RUN apt-get -y update && \
    apt-get install -y openssl ca-certificates tini curl jq && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/* && \
    echo "deb http://security.debian.org/debian-security bullseye-security main" > /etc/apt/sources.list.d/bullseye-security.list && \
    apt-get update && \
    apt-get install -y libssl1.1 && \
    curl -fsSL https://deb.nodesource.com/setup_18.x | bash - && \
    apt-get update && \
    apt-get install -y nodejs && \
    apt-get autoremove -y && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /usr/local/bin

COPY --from=builder-rust /app/target/release/orchestrator .
COPY --from=builder-rust /app/orchestrator/node_modules ./node_modules
COPY --from=builder-rust /app/orchestrator/package.json .
COPY --from=builder-rust /app/orchestrator/migrate-mongo-config.js .
COPY --from=builder-rust /app/orchestrator/migrations ./migrations

COPY --from=builder-rust /app/orchestrator/crates/da-clients/ethereum/trusted_setup.txt \
    /app/orchestrator/crates/settlement-clients/ethereum/src/trusted_setup.txt
COPY --from=builder-rust /app/orchestrator/crates/da-clients/ethereum/trusted_setup.txt /tmp/trusted_setup.txt
COPY --from=builder-rust /tmp/kzg_dirs.txt /tmp/kzg_dirs.txt

RUN while read dir; do \
    mkdir -p "$dir" && \
    cp /tmp/trusted_setup.txt "$dir/trusted_setup.txt"; \
    done < /tmp/kzg_dirs.txt

EXPOSE 3000

ENV TINI_VERSION=v0.19.0
ADD https://github.com/krallin/tini/releases/download/${TINI_VERSION}/tini /bin/tini
RUN chmod +x /bin/tini

ENTRYPOINT ["tini", "--", "./orchestrator"]
