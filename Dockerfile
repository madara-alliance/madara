# Stage 1: Build the application
FROM rust:1.78 AS builder

# Install build dependencies
RUN apt-get -y update &&        \
    apt-get install -y clang && \
    apt-get autoremove -y;      \
    apt-get clean;              \
    rm -rf /var/lib/apt/lists/*

# Set the working directory
WORKDIR /usr/src/madara/

# Copy the source code
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY cairo cairo
COPY cairo_0 cairo_0

# Installing scarb, new since devnet integration
# Installation steps are taken from the scarb build script
# https://github.com/software-mansion/scarb/blob/main/install.sh
ENV SCARB_VERSION="v2.8.2"
ENV SCARB_REPO="https://github.com/software-mansion/scarb/releases/download"
ENV PLATFORM="x86_64-unknown-linux-gnu"
ENV SCARB_TARGET="/usr/src/scarb.tar.gz"

RUN curl -fLS -o $SCARB_TARGET                                          \
    $SCARB_REPO/$SCARB_VERSION/scarb-$SCARB_VERSION-$PLATFORM.tar.gz && \
    tar -xz -C /usr/src/ --strip-components=1 -f $SCARB_TARGET &&       \
    mv /usr/src/bin/scarb /bin

# Build the application in release mode
RUN cargo build --release

# Stage 2: Create the final runtime image
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get -y update &&                          \
    apt-get install -y openssl ca-certificates && \
    apt-get autoremove -y;                        \
    apt-get clean;                                \
    rm -rf /var/lib/apt/lists/*

# Set the working directory
WORKDIR /usr/local/bin

# Copy the compiled binary from the builder stage
COPY --from=builder /usr/src/madara/target/release/madara .

# chain presets to be monted at startup
VOLUME crates/primitives/chain_config/presets
VOLUME crates/primitives/chain_config/resources

# Set the entrypoint
ENTRYPOINT ["./madara"]
