#!/bin/sh
if [ -f "$RPC_API_KEY_FILE" ]; then
  export RPC_API_KEY=$(cat "$RPC_API_KEY_FILE")
else
  echo "Error: RPC_API_KEY_FILE not found!" >&2
  exit 1
fi

exec tini -- ./madara        \
	--name madara            \
	--network mainnet        \
	--rpc-external           \
	--full                   \
	--l1-endpoint $RPC_API_KEY
