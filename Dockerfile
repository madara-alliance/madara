# Stage 1: Build the application
FROM rust:bookworm AS builder

# Install build dependencies
RUN apt-get -y update && \
    apt-get install -y --no-install-recommends \
        clang \
        protobuf-compiler \
        build-essential && \

# Set the working directory
WORKDIR /usr/local/bin

# Cache dependencies by copying the Cargo.toml and Cargo.lock files first
COPY Cargo.toml Cargo.lock ./

# Create a dummy main file and build dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release

# Remove the dummy main file
RUN rm -rf src

# Copy the rest of the source code into the container
COPY . .

# Build the application in release mode
RUN cargo build --release

# Stage 2: Create the final runtime image
FROM debian:bookworm

# Install runtime dependencies
RUN apt-get -y update && \
    apt-get install -y --no-install-recommends \
        clang \
        protobuf-compiler && \

# Set the working directory
WORKDIR /usr/local/bin

# Copy the compiled binary from the builder stage
COPY --from=builder /usr/local/bin/target/release/deoxys .

# Set the entrypoint
ENTRYPOINT ["./deoxys"]
