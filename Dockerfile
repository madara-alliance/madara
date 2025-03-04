# Stage 1: Build the application
FROM debian:12 AS builder

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
    pkg-config

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

# RUN cargo build --release --features cairo_native
RUN cargo build --release

WORKDIR /usr/local/bin
RUN cp /usr/src/madara/target/release/madara ./
    
# Set the entrypoint
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["./madara"]