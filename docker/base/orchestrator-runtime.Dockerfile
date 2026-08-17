FROM debian:bookworm-slim

RUN apt-get -y update && \
  apt-get install -y openssl ca-certificates curl jq && \
  curl -fsSL https://deb.nodesource.com/setup_18.x | bash - && \
  apt-get update && \
  apt-get install -y nodejs && \
  apt-get autoremove -y; \
  apt-get clean; \
  rm -rf /var/lib/apt/lists/*

ENV TINI_VERSION=v0.19.0
ADD https://github.com/krallin/tini/releases/download/${TINI_VERSION}/tini /bin/tini
RUN chmod +x /bin/tini
