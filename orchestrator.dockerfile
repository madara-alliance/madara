FROM python:3.9-bookworm AS builder

# Install system dependencies
RUN apt-get update && apt-get install -y \
    git libgmp3-dev wget bash curl \
    build-essential \
    nodejs npm clang mold \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# Install Rust using rustup (will respect rust-toolchain.toml)
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain nightly-2024-09-04
ENV PATH="/root/.cargo/bin:${PATH}"

# Set the working directory
WORKDIR /usr/src/madara/

# Copy the local codebase
COPY . .


# Install the toolchain specified in rust-toolchain.toml
# we might need to use toolchain.toml to install the correct toolchain for the project
# but it's not working as expected, so we're using custom nightly version temprarily
# RUN rustup show

# Setting it to avoid building artifacts again inside docker
ENV RUST_BUILD_DOCKER=true

# Build only the orchestrator binary
RUN cargo build --bin orchestrator --release

# Install Node.js dependencies for migrations
RUN cd orchestrator && npm install

# Dump information where kzg files must be copied
RUN mkdir -p /tmp && \
    (find /usr/local/cargo -type d -path "*/crates/starknet-os/kzg" > /tmp/kzg_dirs.txt 2>/dev/null || \
     find $HOME/.cargo -type d -path "*/crates/starknet-os/kzg" >> /tmp/kzg_dirs.txt 2>/dev/null || \
     touch /tmp/kzg_dirs.txt)


FROM debian:bookworm

# Install runtime dependencies
RUN apt-get -y update && \
    apt-get install -y openssl ca-certificates curl jq && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/* && \
    # Add backports repo for libssl1.1
    echo "deb http://security.debian.org/debian-security bullseye-security main" > /etc/apt/sources.list.d/bullseye-security.list && \
    apt-get update && \
    apt-get install -y libssl1.1 && \
    curl -fsSL https://deb.nodesource.com/setup_18.x | bash - && \
    apt-get update && \
    apt-get install -y nodejs && \
    apt-get autoremove -y && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*

# Set the working directory
WORKDIR /usr/local/bin

# Copy the compiled binary from the builder stage
COPY --from=builder /usr/src/madara/target/release/orchestrator .

# Copy Node.js files and dependencies
COPY --from=builder /usr/src/madara/orchestrator/node_modules ./node_modules
COPY --from=builder /usr/src/madara/orchestrator/package.json .
COPY --from=builder /usr/src/madara/orchestrator/migrate-mongo-config.js .
COPY --from=builder /usr/src/madara/orchestrator/migrations ./migrations

# # To be fixed by this https://github.com/keep-starknet-strange/snos/issues/404
COPY --from=builder /usr/src/madara/orchestrator/crates/da-clients/ethereum/trusted_setup.txt \
    /usr/src/madara/orchestrator/crates/settlement-clients/ethereum/src/trusted_setup.txt

# Copy the needed files from builder
COPY --from=builder /usr/src/madara/orchestrator/crates/da-clients/ethereum/trusted_setup.txt /tmp/trusted_setup.txt
COPY --from=builder /tmp/kzg_dirs.txt /tmp/kzg_dirs.txt

# Recreate the dynamic dirs and copy the trusted setup into them
RUN while read dir; do \
    mkdir -p "$dir" && \
    cp /tmp/trusted_setup.txt "$dir/trusted_setup.txt"; \
    done < /tmp/kzg_dirs.txt

# Expose the default port
EXPOSE 3000

# Create a startup script based on Makefile commands
RUN echo '#!/bin/bash' > start.sh && \
    echo 'set -e' >> start.sh && \
    echo '' >> start.sh && \
    echo '# Check if we want to run setup or run command' >> start.sh && \
    echo 'if [ "$1" = "setup-l2" ]; then' >> start.sh && \
    echo '    echo "Running orchestrator setup L2..."' >> start.sh && \
    echo '    ./orchestrator setup --layer l2 --aws --aws-s3 --aws-sqs --aws-sns --aws-event-bridge --event-bridge-type schedule' >> start.sh && \
    echo 'elif [ "$1" = "setup-l3" ]; then' >> start.sh && \
    echo '    echo "Running orchestrator setup L3..."' >> start.sh && \
    echo '    ./orchestrator setup --layer l3 --aws --aws-s3 --aws-sqs --aws-sns --aws-event-bridge --event-bridge-type schedule' >> start.sh && \
    echo 'elif [ "$1" = "run-l2" ]; then' >> start.sh && \
    echo '    echo "Running orchestrator L2..."' >> start.sh && \
    echo '    ./orchestrator run --layer l3 --aws --aws-s3 --aws-sqs --aws-sns --settle-on-ethereum --atlantic --da-on-ethereum' >> start.sh && \
    echo 'elif [ "$1" = "run-l3" ]; then' >> start.sh && \
    echo '    echo "Running orchestrator L3..."' >> start.sh && \
    echo '    ./orchestrator run --layer l3 --aws --aws-s3 --aws-sqs --aws-sns --settle-on-starknet --atlantic --da-on-starknet' >> start.sh && \
    echo 'else' >> start.sh && \
    echo '    echo "Usage: $0 {setup-l2|setup-l3|run-l2|run-l3}"' >> start.sh && \
    echo '    echo "Available commands:"' >> start.sh && \
    echo '    echo "  setup-l2  - Setup orchestrator with L2 layer"' >> start.sh && \
    echo '    echo "  setup-l3  - Setup orchestrator with L3 layer"' >> start.sh && \
    echo '    echo "  run-l2    - Run orchestrator with L2 (Ethereum settlement)"' >> start.sh && \
    echo '    echo "  run-l3    - Run orchestrator with L3 (Starknet settlement)"' >> start.sh && \
    echo '    exit 1' >> start.sh && \
    echo 'fi' >> start.sh && \
    chmod +x start.sh

ENTRYPOINT ["./start.sh"]
CMD ["run-l3"]
