FROM python:3.7-slim

# Install system dependencies
RUN apt-get update && apt-get install -y \
    make \
    libgmp-dev \
    g++ \
    python3-dev \
    npm \
    unzip \
    curl \
    cmake \
    && rm -rf /var/lib/apt/lists/*

# Install Python dependencies with increased timeouts and retry mechanisms
RUN pip install --upgrade pip

# Create pip.conf with timeout and retry settings
RUN mkdir -p /etc/pip && \
    echo "[global]" > /etc/pip/pip.conf && \
    echo "timeout = 180" >> /etc/pip/pip.conf && \
    echo "retries = 5" >> /etc/pip/pip.conf && \
    echo "default-timeout = 180" >> /etc/pip/pip.conf

# Set environment variables for pip and other network operations
ENV PIP_DEFAULT_TIMEOUT=180 \
    PIP_RETRIES=5 \
    PIP_TIMEOUT=180 \
    HTTP_TIMEOUT=180 \
    PYTHONUNBUFFERED=1

# Install CMake with retry and increased timeout
RUN pip install --timeout 180 --retries 5 cmake==3.22

# Install solc
RUN curl https://binaries.soliditylang.org/linux-amd64/solc-linux-amd64-v0.6.12+commit.27d51765 -o /usr/local/bin/solc-0.6.12
RUN echo 'f6cb519b01dabc61cab4c184a3db11aa591d18151e362fcae850e42cffdfb09a /usr/local/bin/solc-0.6.12' | sha256sum --check
RUN chmod +x /usr/local/bin/solc-0.6.12

# Install ganache
RUN npm install -g --unsafe-perm ganache-cli@6.12.2

COPY . /app/

# Build
WORKDIR /app/
RUN rm -rf build
# Run build.sh with increased timeouts
RUN PIP_DEFAULT_TIMEOUT=180 PIP_RETRIES=5 HTTP_TIMEOUT=180 ./build.sh

# Create a simpler entrypoint script
RUN echo '#!/bin/sh' > /entrypoint.sh && \
    echo 'cp -r /app/build/Release/src/* /mnt/' >> /entrypoint.sh && \
    echo 'if [ -z "$1" ]; then' >> /entrypoint.sh && \
    echo '    sh' >> /entrypoint.sh && \
    echo 'else' >> /entrypoint.sh && \
    echo '    sh -c "$*"' >> /entrypoint.sh && \
    echo 'fi' >> /entrypoint.sh

# Make entrypoint executable
RUN chmod +x /entrypoint.sh

ENTRYPOINT ["/entrypoint.sh"]
CMD ["sh"]