FROM rust:slim-buster

RUN apt-get -y update; \
    apt-get install -y --no-install-recommends \
        libssl-dev make clang-11 g++ llvm protobuf-compiler libasound2-dev \
        pkg-config libz-dev zstd git build-essential; \
    apt-get autoremove -y; \
    apt-get clean; \
    rm -rf /var/lib/apt/lists/*

EXPOSE 9944

COPY . .

RUN cargo build --profile=production

ENTRYPOINT ["./target/production/madara"]