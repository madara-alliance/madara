# Stage 1: Build the application
FROM rust:bookworm AS builder

# Install build dependencies
RUN apt-get -y update && \
    apt-get install -y --no-install-recommends \
    clang \
    protobuf-compiler \
    build-essential

# Set the working directory
WORKDIR /usr/src/

# Copy the source code into the container
COPY . .

# Build the application in release mode
RUN cargo build --release

# Stage 2: Create the final runtime image
FROM debian:bookworm

# Install runtime dependencies
RUN apt-get -y update && \
    apt-get install -y --no-install-recommends \
    clang \
    protobuf-compiler

# Set the working directory
WORKDIR /usr/local/bin

# Copy the compiled binary from the builder stage
COPY --from=builder /usr/src/target/release/deoxys .

# Set the entrypoint
ENTRYPOINT ["./deoxys"]