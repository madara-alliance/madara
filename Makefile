run:
	cargo run \
	--release \
	-- \
	--name deoxys \
	--base-path ../deoxys-db \
	--rpc-port 9944 \
	--network main \
	--l1-endpoint https://eth-mainnet.g.alchemy.com/v2/Oqh-00iLC3Nx08nkJOg241ky-7JA41cJ \
	--rpc-cors all \

# run_bis:
	cargo run --release -- \
	--name deoxys \
	--base-path ../deoxys-db \
	--chain starknet \
	--network main \
	--l1-endpoint https://eth-mainnet.g.alchemy.com/v2/Oqh-00iLC3Nx08nkJOg241ky-7JA41cJ \
	--rpc-port 9944 \
	--rpc-cors "*" \
	--rpc-external

# --l1-endpoint https://eth-mainnet.g.alchemy.com/v2/Oqh-00iLC3Nx08nkJOg241ky-7JA41cJ