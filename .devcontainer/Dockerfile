FROM ubuntu:24.04

ARG USERNAME=vscode
# replace this with your own user id (outside the container)
# you can retreive your user id with `id -u`
# this makes it so files created in the container are still owned by the host user (and not root)
ARG USER_UID=1000
ARG USER_GID=$USER_UID
ENV DEBIAN_FRONTEND=noninteractive
ENV TZ=Etc/UTC

# Check if user with UID 1000 already exists, if not create it
RUN if ! id -u $USER_UID > /dev/null 2>&1; then \
    groupadd --gid $USER_GID $USERNAME \
    && useradd --uid $USER_UID --gid $USER_GID -m $USERNAME; \
    else \
    usermod -l $USERNAME $(getent passwd $USER_UID | cut -d: -f1) \
    && groupmod -n $USERNAME $(getent group $USER_GID | cut -d: -f1); \
    fi \
    && apt-get update \
    && apt-get install -y sudo tzdata \
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
    curl \
    git \
    wget \
    gnupg \
    software-properties-common \
    lsb-release \
    ca-certificates \
    # Python dependencies
    libssl-dev \
    zlib1g-dev \
    libbz2-dev \
    libreadline-dev \
    libsqlite3-dev \
    libncursesw5-dev \
    xz-utils \
    tk-dev \
    libxml2-dev \
    libxmlsec1-dev \
    libffi-dev \
    liblzma-dev \
    && sudo apt-get clean \
    && sh -c "$(curl -fsSL https://raw.githubusercontent.com/ohmyzsh/ohmyzsh/master/tools/install.sh)"

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y \
    && echo 'source $HOME/.cargo/env' >> $HOME/.zshrc

RUN sudo chsh -s /bin/zsh $USERNAME

RUN curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.5/install.sh | bash \
    && echo 'export NVM_DIR="$HOME/.nvm"' >> $HOME/.zshrc \
    && echo '[ -s "$NVM_DIR/nvm.sh" ] && \. "$NVM_DIR/nvm.sh"' >> $HOME/.zshrc \
    && echo '[ -s "$NVM_DIR/bash_completion" ] && \. "$NVM_DIR/bash_completion"' >> $HOME/.zshrc

RUN curl https://pyenv.run | bash \
    && echo 'export PYENV_ROOT="$HOME/.pyenv"' >> $HOME/.zshrc \
    && echo 'command -v pyenv >/dev/null || export PATH="$PYENV_ROOT/bin:$PATH"' >> $HOME/.zshrc \
    && echo 'eval "$(pyenv init -)"' >> $HOME/.zshrc

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.starkup.dev | sh -s -- -y\
    && echo 'export PATH="$HOME/.local/bin:$PATH"' >> $HOME/.zshrc \
    && echo 'export PATH="$HOME/.asdf/shims:$PATH"' >> $HOME/.zshrc

RUN $HOME/.pyenv/bin/pyenv install 3.9 \
    && $HOME/.pyenv/bin/pyenv global 3.9

RUN export NVM_DIR="$HOME/.nvm" && \
    [ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh" && \
    nvm install 22