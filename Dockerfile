# Stage 1: Build the application
FROM rust:1.78 as builder

# Install build dependencies
RUN apt-get -y update && \
    apt-get install -y clang && \
    apt-get autoremove -y; \
    apt-get clean; \
    rm -rf /var/lib/apt/lists/*

# Set the working directory
WORKDIR /usr/src/madara/

# Copy the source code into the container
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY cairo cairo
COPY cairo_0 cairo_0


# Installing scarb, new since devnet integration
# Installation steps are taken from the scarb build script
# https://github.com/software-mansion/scarb/blob/main/install.sh
ENV SCARB_VERSION="v2.8.2"
ENV SHELL /bin/bash
RUN curl --proto '=https' --tlsv1.2 -sSf https://docs.swmansion.com/scarb/install.sh | sh -s -- ${SCARB_VERSION}
ENV PATH="/root/.local/bin:${PATH}"
RUN scarb --version

# Install runtime dependencies
RUN apt-get -y update && \
    apt-get install -y openssl ca-certificates busybox && \
    apt-get autoremove -y; \
    apt-get clean; \
    rm -rf /var/lib/apt/lists/*

# Build the application in release mode
RUN cargo build --release

# Stage 2: Create the final runtime image
FROM debian:bookworm

# Install runtime dependencies
RUN apt-get -y update && \
    apt-get install -y openssl ca-certificates tini &&\
    apt-get autoremove -y; \
    apt-get clean; \
    rm -rf /var/lib/apt/lists/*

# Set the working directory
WORKDIR /usr/local/bin

# Copy the compiled binary from the builder stage
COPY --from=builder /usr/src/madara/target/release/madara .

# Set the entrypoint
ENTRYPOINT ["./madara"]