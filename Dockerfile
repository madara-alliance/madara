# Stage 1: Build the application
FROM rust:slim-buster AS builder

# Install build dependencies
RUN apt-get -y update && \
    apt-get install -y --no-install-recommends \
        clang \
        protobuf-compiler \
        build-essential && \
    apt-get autoremove -y && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*

# Set the working directory
WORKDIR /usr/src/deoxys

# Copy the source code into the container
COPY . .

# Build the application in release mode
RUN cargo build --release

# Stage 2: Create the final runtime image
FROM debian:buster-slim

# Install runtime dependencies
RUN apt-get -y update && \
    apt-get install -y --no-install-recommends \
        clang \
        protobuf-compiler && \
    apt-get autoremove -y && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*

# Set the working directory
WORKDIR /usr/src/deoxys

# Copy the compiled binary from the builder stage
COPY --from=builder /usr/src/deoxys/target/release/deoxys .

# Set the entrypoint
ENTRYPOINT ["./deoxys"]
