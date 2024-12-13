#!/bin/sh
set -e

# Makes an ethereum checkpoint override json for use with pathfinder's
# `--p2p.experimental.l1-checkpoint-override-json-path <path to pathfinder-checkpoint-override.json>`
# cli argument.
# Returns the path to the resulting json file, so that you can just supply
# `--p2p.experimental.l1-checkpoint-override-json-path $(./scripts/pathfinder-checkpoint-override.sh)` to
# pathfinder.

curl 'http://127.0.0.1:9944/' -s \
    --header 'Content-Type: application/json' \
    --data '{
        "jsonrpc": "2.0",
        "method": "starknet_getBlockWithTxHashes",
        "params": {
            "block_id": "latest"
        },
        "id": 1
    }' \
    | jq '{ block_hash: .result.block_hash, block_number: .result.block_number, state_root: .result.new_root }' \
    > pathfinder-checkpoint-override.json

realpath pathfinder-checkpoint-override.json
