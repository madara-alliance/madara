FROM debian:bookworm-slim

COPY Makefile /tmp/Makefile

RUN apt-get -y update && \
    apt-get install -y openssl ca-certificates curl wget gnupg software-properties-common make && \
    cd /tmp && make -f Makefile install-llvm19 CODENAME=bookworm && \
    apt-get autoremove -y && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/* /tmp/Makefile

ENV MLIR_SYS_190_PREFIX=/usr/lib/llvm-19
ENV LLVM_SYS_191_PREFIX=/usr/lib/llvm-19
ENV TABLEGEN_190_PREFIX=/usr/lib/llvm-19
ENV PATH="/usr/lib/llvm-19/bin:${PATH}"
ENV CC=clang-19
ENV CXX=clang-19
ENV LIBRARY_PATH=/usr/lib/x86_64-linux-gnu:/usr/lib:/lib/x86_64-linux-gnu:/lib
ENV LD_LIBRARY_PATH=/usr/lib/x86_64-linux-gnu:/usr/lib:/lib/x86_64-linux-gnu:/lib

ENV TINI_VERSION=v0.19.0
ADD https://github.com/krallin/tini/releases/download/${TINI_VERSION}/tini /bin/tini
RUN chmod +x /bin/tini
