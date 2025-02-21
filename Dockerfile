# ------ Stage 1: Build the application ------
FROM rust:1.81 AS builder

# Use --build-arg profile=release during building to build using the release profile.
ARG profile=production

# Install build dependencies
RUN apt-get -y update && \
    apt-get install -y clang openssl ca-certificates busybox protobuf-compiler

# Set the working directory
WORKDIR /usr/src/madara/
# Copy the source code into the container
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY .db-versions.yml .db-versions.yml ./
COPY cairo-artifacts cairo-artifacts

# Build the application in release mode

RUN cargo build --profile $profile

RUN mv /usr/src/madara/target/$profile/madara /usr/src/madara/target/madara

# ------ Stage 2: Create the final runtime image ------
FROM debian:bookworm
# todo: alpine/from scratch image would be appreciated

# Install runtime dependencies
RUN apt-get -y update && \
    apt-get install -y openssl ca-certificates tini curl &&\
    apt-get autoremove -y; \
    apt-get clean; \
    rm -rf /var/lib/apt/lists/*

# Set the working directory
WORKDIR /usr/local/bin

# Copy the compiled binary from the builder stage
COPY --from=builder /usr/src/madara/target/madara .

# Set the CMD
CMD ["./madara"]
