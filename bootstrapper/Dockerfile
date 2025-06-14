
# Set working directory
WORKDIR /app

# Copy the entire project
COPY . .

# Initialize and update submodules

RUN apt-get update && apt-get install -y \
    wget

RUN curl https://binaries.soliditylang.org/linux-amd64/solc-linux-amd64-v0.6.12+commit.27d51765 -o /usr/local/bin/solc-0.6.12
RUN echo 'f6cb519b01dabc61cab4c184a3db11aa591d18151e362fcae850e42cffdfb09a /usr/local/bin/solc-0.6.12' | sha256sum --check
RUN chmod +x /usr/local/bin/solc-0.6.12
RUN npm install -g --unsafe-perm ganache-cli@6.12.2

# First run setup-linux
RUN make setup-linux

# Build legacy starkgate contracts
# Note: This section implements the build steps for old starkgate contracts,
# replacing 'make starkgate-contracts-legacy' from the Makefile. We include these steps
# directly here because running the Makefile command would create a nested
# Docker container inside this build.

RUN cd lib/starkgate-contracts-old && \
    # First verify ganache-cli installation
    which ganache-cli && \
    ganache-cli --version && \
    # Start ganache-cli in background with specific host and port
    nohup ganache-cli \
    --host 0.0.0.0 \
    --port 8545 \
    --networkId 1234 \
    --accounts 10 \
    --defaultBalanceEther 1000 \
    --mnemonic "test test test test test test test test test test test junk" \
    --db /tmp/ganache_db \
    > ganache.log 2>&1 & \
    # Store PID and wait
    GANACHE_PID=$! && \
    echo "Started Ganache with PID: $GANACHE_PID" && \
    sleep 15 && \
    # Debug: show ganache logs
    echo "Ganache logs:" && \
    cat ganache.log && \
    # Debug: check if process is running
    ps aux | grep ganache && \
    # Verify ganache is running
    curl -v http://localhost:8545 && \
    # Continue with build
    rm -rf build && \
    ./build.sh && \
    mkdir -p build/Release && \
    mkdir -p starkgate-artifacts && \
    cp -r build/Release/src/* starkgate-artifacts/ && \
    # Kill ganache after build
    kill $GANACHE_PID || true

# Remove existing Node.js and related packages
RUN apt-get purge -y nodejs nodejs-doc node-gyp libnode-dev && \
    apt-get autoremove -y && \
    rm -rf /etc/apt/sources.list.d/nodesource.list && \
    rm -rf /var/lib/apt/lists/*

# Install Node.js 18.x
RUN curl -fsSL https://deb.nodesource.com/setup_18.x | bash - && \
    apt-get update && \
    apt-get install -y nodejs && \
    # Install a specific compatible version of npm
    npm install -g npm@9.8.1

# Install and verify ganache with explicit path and shell
RUN npm install -g --unsafe-perm ganache@7.9.0 && \
    # Verify ganache installation and keep trying if it fails
    (for i in {1..5}; do \
    if which ganache && ganache --version; then \
        echo "Ganache installation verified" && \
        break; \
    else \
        echo "Attempt $i: Ganache not found, trying again..." && \
        echo "Searching for ganache in PATH..." && \
        find / -name ganache 2>/dev/null && \
        # Clear npm cache and reinstall
        npm cache clean --force && \
        npm install -g --unsafe-perm ganache@7.9.0 && \
        # Add environment variables
        export PATH="/usr/local/lib/node_modules/.bin:$PATH" && \
        # Add a small delay to let npm finish
        sleep 5; \
    fi; \
    if [ $i -eq 5 ]; then \
        echo "Failed to install ganache after 5 attempts" && \
        exit 1; \
    fi \
    done)

# Generate other artifacts
RUN . "$HOME/.asdf/asdf.sh" && \
    ls -la /app/.cairo && \
    ls -la /app/.cairo/cairo/bin && \
    export PATH="/app/.cairo/cairo/bin:$PATH" && \
    echo $PATH && \
    which starknet-compile && \
    make starkgate-contracts-latest && \
    make braavos-account-cairo && \
    make argent-contracts-starknet

# Build the Rust project with specific binary name
RUN cargo build --release --workspace --bin madara-bootstrapper

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y curl

# Copy only the compiled binary and artifacts
COPY --from=builder /app/target/release/madara-bootstrapper /usr/local/bin/
COPY --from=builder /app/artifacts /app/artifacts
COPY --from=builder /app/src/contracts/ /app/src/contracts/


# Set working directory

# Environment variables
ENV RUST_LOG=info

# Run the binary
ENTRYPOINT ["/usr/local/bin/madara-bootstrapper"]
