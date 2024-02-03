FROM rust:1.75.0-bookworm

RUN export DEBIAN_FRONTEND=noninteractive \
    && apt-get update \
    && apt-get install -y \
        libasound2-dev \
        protobuf-compiler \
        build-essential \
        clang \
        zsh \
    && rustup toolchain install nightly \
    && rustup default nightly \
    && sh -c "$(curl -fsSL https://raw.githubusercontent.com/ohmyzsh/ohmyzsh/master/tools/install.sh)"