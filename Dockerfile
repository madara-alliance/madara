# Step 0: setup tooling (rust)
FROM rust:1.85 AS base-rust
WORKDIR /app

ENV SCCACHE_URL=https://github.com/mozilla/sccache/releases/download/v0.10.0/sccache-v0.10.0-x86_64-unknown-linux-musl.tar.gz
ENV SCCACHE_TAR=sccache-v0.10.0-x86_64-unknown-linux-musl.tar.gz
ENV SCCACHE_BIN=/bin/sccache
ENV SCCACHE_DIR=/sccache
ENV SCCACHE=sccache-v0.10.0-x86_64-unknown-linux-musl/sccache
ENV CHEF_URL=https://github.com/LukeMathWalker/cargo-chef/releases/download/v0.1.71/cargo-chef-x86_64-unknown-linux-gnu.tar.gz
ENV CHEF_TAR=cargo-chef-x86_64-unknown-linux-gnu.tar.gz
ENV RUSTC_WRAPPER=/bin/sccache

RUN wget $SCCACHE_URL && tar -xvpf $SCCACHE_TAR && mv $SCCACHE $SCCACHE_BIN && mkdir sccache
RUN wget $CHEF_URL && tar -xvpf $CHEF_TAR && mv cargo-chef /bin

RUN --mount=type=cache,target=/var/cache/apt/archives \
    --mount=type=cache,target=/var/lib/apt/lists \
    apt-get -y update && \
    apt-get install -y clang && \
    apt-get autoremove -y; \
    apt-get clean;

# Step 1: Cache dependencies
FROM base-rust AS planner

COPY . .
RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry \
    cargo chef prepare --recipe-path recipe.json

# Step 2: Build crate
FROM base-rust AS builder-rust

COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry \
    cargo chef cook -p madara --release --recipe-path recipe.json

COPY Cargo.toml Cargo.lock .
COPY madara madara
COPY cairo-artifacts cairo-artifacts
COPY .db-versions.yml .db-versions.yml
RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build -p madara --release

# Step 5: runner
FROM debian:bookworm-slim

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
