# Stage 1: Build the application
FROM rust:1.81 AS builder
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
ENV SCARB_VERSION="2.8.2"
RUN git clone https://github.com/asdf-vm/asdf.git ~/.asdf --branch v0.14.1
RUN echo '. "$HOME/.asdf/asdf.sh"' >> ~/.bashrc
RUN echo '. "$HOME/.asdf/completions/asdf.bash"' >> ~/.bashrc
ENV PATH="/root/.asdf/shims:/root/.asdf/bin:${PATH}"
RUN asdf plugin add scarb
RUN asdf install scarb ${SCARB_VERSION}
RUN asdf global scarb ${SCARB_VERSION}
RUN scarb --version

# Install runtime dependencies
RUN apt-get -y update && \
    apt-get install -y openssl ca-certificates busybox && \
    apt-get autoremove -y; \
    apt-get clean; \
    rm -rf /var/lib/apt/lists/*

# Build the application in release mode
RUN cargo build --release --bin madara
# Stage 2: Create the final runtime image
FROM debian:bookworm
# Install runtime dependencies
RUN apt-get -y update && \
    apt-get install -y openssl ca-certificates &&\
    apt-get autoremove -y; \
    apt-get clean; \
    rm -rf /var/lib/apt/lists/*
# Set the working directory
WORKDIR /usr/local/bin
# Copy the compiled binary from the builder stage
COPY --from=builder /usr/src/madara/target/release/madara .

# Set the entrypoint
ENTRYPOINT ["./madara"]
