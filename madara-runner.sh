#!/bin/sh
export RPC_API_KEY=$(cat $RPC_API_KEY_FILE)

./madara                   \
	--name madara            \
	--network mainnet        \
	--rpc-external           \
	--rpc-cors all           \
	--full                   \
	--l1-endpoint $RPC_API_KEY
