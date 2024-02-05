#!/bin/bash

echo ">> Starting setup.sh script execution."

# Uncomment this if you want to remove the db on each container start
# rm -rf /tmp/deoxys

# Sets up Deoxys environment
cargo run -- setup --chain starknet --from-remote --base-path /tmp/deoxys \
&& cargo fmt

# Uncomment this if you want to run deoxys on container start
# cargo run -- \
#     --deoxys \
#     --rpc-port 9944 \ 
#     --network main \
#     --pruning archive \
#     --rpc-cors=all \
#     --l1-endpoint "key_url" # replace with your own l1 provider url key

echo ">> Finished setup.sh script execution."
