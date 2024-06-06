FROM rust:1.75.0-bookworm

ARG USERNAME=vscode
# replace this with your own user id (outside the container)
# you can retreive your user id with `id -u`
# this makes it so files created in the container are still owned by the host user (and not root)
ARG USER_UID=1001 
ARG USER_GID=$USER_UID

RUN export DEBIAN_FRONTEND=noninteractive \
    && groupadd --gid $USER_GID $USERNAME \
    && useradd --uid $USER_UID --gid $USER_GID -m $USERNAME \
    && apt-get update \
    && apt-get install -y sudo \
    && echo $USERNAME ALL=\(root\) NOPASSWD:ALL > /etc/sudoers.d/$USERNAME \
    && chmod 0440 /etc/sudoers.d/$USERNAME

USER vscode

RUN sudo apt-get update \
    && sudo apt-get install -y \
        libasound2-dev \
        protobuf-compiler \
        build-essential \
        clang \
        zsh \
        htop \
        jq \
        tree \
    && sh -c "$(curl -fsSL https://raw.githubusercontent.com/ohmyzsh/ohmyzsh/master/tools/install.sh)"