#!/bin/bash

echo ">> Starting setup.sh script execution."

# Uncomment this if you want to remove the db on each container start
# rm -rf /tmp/madara

echo ">> Rust version:"
rustup show

echo ">> Node version:"
node --version

asdf install scarb 2.8.2
asdf set -u scarb 2.8.2 
echo ">> Scarb version:"
scarb --version

echo ">> Python version:"
python --version

# Uncomment this if you want to run madara on container start
# cargo run -- \
#     --madara \
#     --rpc-port 9944 \ 
#     --network main \
#     --pruning archive \
#     --rpc-cors=all \
#     --l1-endpoint "key_url" # replace with your own l1 provider url key

echo ">> Finished setup.sh script execution."
