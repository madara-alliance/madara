# Production Node for Madara
#
# To be executed from the repository root
FROM docker.io/library/ubuntu:22.04 AS builder

# Branch or tag to build from
ARG COMMIT="main"
ARG RUSTFLAGS=""

ARG SCARB_VERSION=""
RUN test -n "$SCARB_VERSION"

ARG PYTHON_VERSION=""
RUN test -n "$PYTHON_VERSION"

ARG FOUNDRY_VERSION=""
RUN test -n "$FOUNDRY_VERSION"


ENV SCARB_TAG="v$SCARB_VERSION"
ENV PYTHON_VERSION=$PYTHON_VERSION
ENV FOUNDRY_VERSION=$FOUNDRY_VERSION
ENV RUSTFLAGS=$RUSTFLAGS
ENV DEBIAN_FRONTEND=noninteractive

ENV SCARB_REPO="https://github.com/software-mansion/scarb/releases/download"
ENV SCARB_PLATFORM="x86_64-unknown-linux-gnu"
ENV SCARB_TARGET="/usr/src/scarb.tar.gz"

# Used to call python binary later (should be "3")
ENV PYTHON_MAJOR_VERSION=${PYTHON_VERSION%%.*}


WORKDIR /

RUN echo "*** Installing Basic dependencies ***"
RUN apt-get update && apt-get install --assume-yes ca-certificates && update-ca-certificates
# Required for python
RUN apt install --assume-yes software-properties-common && add-apt-repository ppa:deadsnakes/ppa
RUN apt update
# PYTHON_INSTALL_VERSION is the truncated version (e.g. 3.9) in case PYTHON_VERSION contains also the patch version (e.g. 3.9.1)
RUN PYTHON_INSTALL_VERSION=$(echo $PYTHON_VERSION | cut -d'.' -f1,2); \
    apt install --assume-yes git clang curl libssl-dev llvm libudev-dev make protobuf-compiler gcc g++ \
    build-essential pkg-config wget \
	python$PYTHON_INSTALL_VERSION-dev python$PYTHON_INSTALL_VERSION-venv python$PYTHON_INSTALL_VERSION \
	python$PYTHON_MAJOR_VERSION-pip
 

RUN set -e

RUN echo "*** Installing Rust environment ***"
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:$PATH"
RUN rustup default stable
# rustup version are pinned in the rust-toolchain file

# Clone the Madara repository
RUN echo "*** Cloning Madara ***" && \
	if git ls-remote --heads https://github.com/madara-alliance/madara.git $COMMIT | grep -q $COMMIT; then \
	echo "Cloning branch $COMMIT"; \
	git clone --depth=1 --branch $COMMIT https://github.com/madara-alliance/madara.git; \
	elif git ls-remote --tags https://github.com/madara-alliance/madara.git $COMMIT | grep -q $COMMIT; then \
	echo "Cloning tag $COMMIT"; \
	git clone --depth=1 --branch $COMMIT https://github.com/madara-alliance/madara.git; \
	else \
	echo "Cloning specific commit $COMMIT"; \
	git clone --depth=1 https://github.com/madara-alliance/madara.git && \
	cd madara && \
	git fetch origin $COMMIT && \
	git checkout $COMMIT; \
	fi

RUN echo "*** Installing Scarb ***"
RUN curl -fLS -o $SCARB_TARGET \
    $SCARB_REPO/$SCARB_TAG/scarb-$SCARB_TAG-$SCARB_PLATFORM.tar.gz && \
    tar -xz -C /usr/src/ --strip-components=1 -f $SCARB_TARGET && \
    mv /usr/src/bin/scarb /bin

RUN echo "*** Installing Foundry ***"
RUN curl -L https://foundry.paradigm.xyz | bash
RUN /root/.foundry/bin/foundryup -i $FOUNDRY_VERSION

RUN echo "*** Install Cairo 0 ***"
RUN apt install -y libgmp3-dev
RUN pip3 install ecdsa fastecdsa sympy
RUN pip3 install cairo-lang

WORKDIR /madara

RUN echo "*** Building SNOS ***"
RUN make snos

# Print target cpu
RUN rustc --print target-cpus

RUN echo "*** Building Madara ***"
RUN cargo build --profile=production --all

FROM debian:stable-slim
LABEL description="Production Binary for Madara Nodes"

# Define port arguments with defaults
ARG PARACHAIN_P2P_PORT=30333
ARG RELAYCHAIN_P2P_PORT=30334
ARG RPC_PORT=9944
ARG PROMETHEUS_PORT=9615

# Required for libssl
RUN apt-get update && apt-get install --assume-yes openssl

# Setup a lower privileged user
RUN useradd -m -u 1000 -U -s /bin/sh -d /madara madara && \
	mkdir -p /madara/.local/share && \
	mkdir /data && \
	chown -R madara:madara /data && \
	ln -s /data /madara/.local/share/madara && \
	rm -rf /usr/sbin

# Switch to the lower privileged user
USER madara
	
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=builder --chown=madara /madara/target/production/madara /madara/madara
COPY --from=builder --chown=madara /madara/target/production/orchestrator /madara/orchestrator

RUN chmod uog+x /madara/madara /madara/orchestrator

# 30333 for parachain p2p
# 30334 for relaychain p2p
# 9944 for Websocket & RPC call
# 9615 for Prometheus (metrics)
EXPOSE ${PARACHAIN_P2P_PORT} ${RELAYCHAIN_P2P_PORT} ${RPC_PORT} ${PROMETHEUS_PORT}

VOLUME ["/data"]

ENTRYPOINT ["/madara/madara"]
