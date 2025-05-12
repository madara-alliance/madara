FROM debian:bookworm AS builder
WORKDIR /contracts

RUN apt-get -y update && apt-get install -y \
  python3-full \
  python3-pip \
  python3-venv \
  npm \
  wget \
  curl \
  git

# =========================================================================== #
#                              STARKGATE CONTRACTS                            #
# =========================================================================== #

FROM builder AS starkgate-latest
ENV STARKGATE_TAG="v2.0.1"
ENV STARKGATE_DIR="/contracts/starkgate-contracts"
ENV COMPILE_SIERRA=".downloads/cairo/bin/starknet-sierra-compile"

# clone repository
RUN git clone https://github.com/starknet-io/starkgate-contracts.git
WORKDIR starkgate-contracts
RUN git checkout $STARKGATE_TAG

# setup python
RUN python3 -m venv venv
ENV PATH="$STARKGATE_DIR/venv/bin:$PATH"
RUN echo $PATH $PWD
RUN pip install -r requirements.txt

# Download cairo and solc
RUN scripts/setup.sh

RUN scripts/build-cairo.sh                                                                                        && \
    scripts/build-solidity.sh                                                                                     && \
    $COMPILE_SIERRA cairo_contracts/ERC20Lockable.sierra cairo_contracts/ERC20Lockable.casm                       && \
    $COMPILE_SIERRA cairo_contracts/TokenBridge.sierra cairo_contracts/TokenBridge.casm                           && \
    $COMPILE_SIERRA cairo_contracts/LegacyBridgeUpgradeEIC.sierra cairo_contracts/LegacyBridgeUpgradeEIC.casm     && \
    $COMPILE_SIERRA cairo_contracts/RolesExternalInitializer.sierra cairo_contracts/RolesExternalInitializer.casm && \
    $COMPILE_SIERRA cairo_contracts/ERC20.sierra cairo_contracts/ERC20.casm

# =========================================================================== #
#                          STARKGATE CONTRACTS (legacy)                       #
# =========================================================================== #

FROM ciimage/python:3.7 AS starkgate-legacy
ENV STARKGATE_REV="c08863a1f08226c09f1d0748124192e848d73db9"

WORKDIR /contracts

RUN apt-get update -y
RUN apt -y -o Dpkg::Options::="--force-overwrite" install python3.7-dev
RUN apt-get install -y \
  libgmp3-dev \
  make \
  g++ \
  npm \
  unzip \
  git

# Installing cmake via apt doesn't bring the most up-to-date version.
RUN pip install cmake==3.22

RUN git clone https://github.com/starknet-io/starkgate-contracts.git
WORKDIR starkgate-contracts
RUN git checkout $STARKGATE_REV

# Install solc and ganache
RUN curl https://binaries.soliditylang.org/linux-amd64/solc-linux-amd64-v0.6.12+commit.27d51765 -o /usr/local/bin/solc-0.6.12
RUN echo 'f6cb519b01dabc61cab4c184a3db11aa591d18151e362fcae850e42cffdfb09a /usr/local/bin/solc-0.6.12' | sha256sum --check
RUN chmod +x /usr/local/bin/solc-0.6.12
RUN npm install -g --unsafe-perm ganache-cli@6.12.2

# Build.
RUN ./build.sh

# =========================================================================== #
#                               BRAAVOS CONTRACTS                             #
# =========================================================================== #

FROM builder AS bravoos
ENV BRAAVOS_REV="12b82a87b93ba9bfdf2cbbde2566437df2e0c6c8"
ENV SCARB_VERSION="2.8.4"
ENV SCARB_RELEASES="https://github.com/software-mansion/scarb/releases/download"
ENV SCARB_URL="$SCARB_RELEASES/v$SCARB_VERSION/scarb-v$SCARB_VERSION-x86_64-unknown-linux-gnu.tar.gz"

# clone repository
RUN git clone https://github.com/myBraavos/braavos-account-cairo.git
WORKDIR braavos-account-cairo
RUN git checkout $BRAAVOS_REV

# Setup scarb
RUN mkdir scarb && wget -c $SCARB_URL -O - | tar -xz -C scarb --strip-components=1

# Compile contracts
RUN ./scarb/bin/scarb build

# =========================================================================== #
#                               ARGENT CONTRACTS                              #
# =========================================================================== #

FROM builder as argent
ENV ARGENT_REV="1352198956f36fb35fa544c4e46a3507a3ec20e3"
ENV SCARB_VERSION="2.6.3"
ENV SCARB_RELEASES="https://github.com/software-mansion/scarb/releases/download"
ENV SCARB_URL="$SCARB_RELEASES/v$SCARB_VERSION/scarb-v$SCARB_VERSION-x86_64-unknown-linux-gnu.tar.gz"

# clone repository
RUN git clone https://github.com/argentlabs/argent-contracts-starknet.git
WORKDIR argent-contracts-starknet
RUN git checkout $ARGENT_REV

# Setup scarb
RUN mkdir scarb && wget -c $SCARB_URL -O - | tar -xz -C scarb --strip-components=1

# Compile contracts
RUN ./scarb/bin/scarb build
