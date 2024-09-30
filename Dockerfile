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

# Installing Scarb
# Dynamically detect the architecture
ARG SCARB_VERSION="v2.8.2"
ARG SCARB_REPO="https://github.com/software-mansion/scarb/releases/download"
RUN ARCH=$(uname -m); \
    if [ "$ARCH" = "x86_64" ]; then \
        PLATFORM="x86_64-unknown-linux-gnu"; \
    elif [ "$ARCH" = "aarch64" ] || [ "$ARCH" = "arm64" ]; then \
        PLATFORM="aarch64-unknown-linux-gnu"; \
    else \
        echo "Unsupported architecture: $ARCH"; exit 1; \
    fi && \
    curl -fLS -o /usr/src/scarb.tar.gz \
        $SCARB_REPO/$SCARB_VERSION/scarb-$SCARB_VERSION-$PLATFORM.tar.gz && \
    tar -xz -C /usr/local --strip-components=1 -f /usr/src/scarb.tar.gz

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

# Chain presets to be mounted at startup
VOLUME crates/primitives/chain_config/presets
VOLUME crates/primitives/chain_config/resources

# Set the entrypoint
ENTRYPOINT ["./madara"]
