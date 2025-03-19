# Stage 1: Build the application
FROM debian:12 AS builder

# Use --build-arg profile=release during building to build using the release profile.
ARG profile=production

# Install build dependencies
RUN apt update -y && apt install -y \
    lsb-release \
    wget \
    curl \
    git \
    build-essential \
    clang-19 \
    libclang-dev \
    libz-dev \
    libzstd-dev \
    libssl-dev \
    pkg-config \
    protobuf-compiler

# Install LLVM 19 and MLIR
RUN echo "deb http://apt.llvm.org/bookworm/ llvm-toolchain-bookworm-19 main" > /etc/apt/sources.list.d/llvm-19.list
RUN echo "deb-src http://apt.llvm.org/bookworm/ llvm-toolchain-bookworm-19 main" >> /etc/apt/sources.list.d/llvm-19.list
RUN wget -O - https://apt.llvm.org/llvm-snapshot.gpg.key | apt-key add -
RUN apt update -y && apt install -y \
    libmlir-19-dev \
    libpolly-19-dev \
    llvm-19-dev \
    mlir-19-tools

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | bash -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

# Set up LLVM and MLIR paths
ENV MLIR_SYS_190_PREFIX=/usr/lib/llvm-19
ENV LLVM_SYS_191_PREFIX=/usr/lib/llvm-19
ENV TABLEGEN_190_PREFIX=/usr/lib/llvm-19
ENV LIBCLANG_PATH="/usr/lib/llvm-19/lib"

WORKDIR /usr/src/madara

COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY .db-versions.yml .db-versions.yml ./
COPY cairo-artifacts cairo-artifacts

# Build the application

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
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["./madara"]
