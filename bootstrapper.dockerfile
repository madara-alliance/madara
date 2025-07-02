# ==============================================
# Karnot Bootstrapper
# ==============================================
FROM ubuntu:22.04 AS builder

# Install basic dependencies
RUN apt-get update && apt-get install -y \
    curl \
    build-essential \
    clang \
    mold \
    pkg-config \
    libssl-dev \
    git \
    libffi-dev \
    nodejs \
    npm \
    make \
    libgmp-dev \
    g++ \
    unzip \
    cmake \
    wget \
    software-properties-common \
    && rm -rf /var/lib/apt/lists/*

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

# Set working directory
WORKDIR /app

# Copy the entire project
COPY . .

# Setting it to avoid building artifacts again inside docker
ENV RUST_BUILD_DOCKER=true

# Build the Rust project with specific binary name
RUN cargo build --release --workspace --bin bootstrapper

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y curl

# Copy only the compiled binary and artifacts
COPY --from=builder /app/target/release/bootstrapper /usr/local/bin/
COPY --from=builder /app/build-artifacts /app/build-artifacts
COPY --from=builder /app/bootstrapper/src/contracts/ /app/bootstrapper/src/contracts/


# Set working directory
WORKDIR /app

# Environment variables
ENV RUST_LOG=info

# Run the binary
ENTRYPOINT ["/usr/local/bin/bootstrapper"]
