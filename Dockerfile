FROM rust:slim-buster

RUN apt-get -y update && \
    apt-get install -y --no-install-recommends \
        clang \
        protobuf-compiler \
        build-essential && \
    apt-get autoremove -y && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*

COPY . .

RUN cargo build --release

ENTRYPOINT ["./target/release/deoxys"]
