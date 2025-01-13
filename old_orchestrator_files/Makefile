# Orchestrator
ORCHESTRATOR_PATH := $(shell pwd)

# Bootstrapper
OPERATOR_ADDRESS := 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
BOOTSTRAP_OUTPUT_PATH := $(shell pwd)/build/bootstrap.json
BOOTSTRAP_INPUT_PATH := $(shell pwd)/bootstrap_input.json
BOOTSTRAPPER_COMMIT := f717bf179581da53d68fee03b50ef78e0628ee20
BOOTSTRAPPER_PATH := $(shell pwd)/madara-bootstrapper
VERIFIER_ADDRESS := 0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512

# SNOS
CAIRO_LANG_COMMIT := a86e92bfde9c171c0856d7b46580c66e004922f3

# Madara
MADARA_PATH := $(shell pwd)/madara
MADARA_COMMIT := 93d57632e7a56707d74ab63eb0ca8558df9f0d8e
MADARA_DATA_PATH := $(shell pwd)/build/madara

# Pathfinder
PATHFINDER_COMMIT := 138140e5fd967ede92806fd62de47c2e6b65712a
PATHFINDER_DATA_PATH := $(shell pwd)/build/pathfinder
PATHFINDER_PATH := $(shell pwd)/pathfinder
ETHEREUM_WSS_RPC_URL := wss://eth-sepolia.g.alchemy.com/v2/${ETHEREUM_API_KEY}

# Environment file
ENV_FILE := $(shell pwd)/.makefile.json
PID_FILE := $(shell pwd)/.pids.json
JSON_UPDATER := $(shell pwd)/scripts/save_json.sh

.PHONY: all anvil madara core-contract update-madara eth-bridge pathfinder orchestrator cleanup

# JSON update functions

define save_pid
	./scripts/save_pid.sh $1 $2
endef

define get_pid
	./scripts/get_pid.sh $1
endef

define kill_pid
	./scripts/kill_pid.sh $1
endef

# function to update JSON
# Usage: $(call update-json-safe,<json-file>,<key>,<value>)
define update-json-safe
	$(JSON_UPDATER) $(1) $(2) $(3)
endef

setup:
	@echo "Starting setup process..."
	@make cleanup
	@trap 'make cleanup' EXIT INT TERM

	@make core-contract
	@echo "Core contract setup completed."

	@echo "Starting Madara in bootstrap mode..."
	@make madara-bootstrap-mode & $(call save_pid,madara,$$!)
	@echo "Waiting for Madara to start..."
	@while ! curl -s -o /dev/null -w '%{http_code}' http://localhost:9944/health | grep -q '200'; do sleep 1; done
	@echo "Madara started in background (PID: $$($(call get_pid,madara)))"

	@make udc
	@echo "UDC deployed on chain."

	@echo "Starting ETH Bridge setup..."
	@make eth-bridge

	# we need to sleep for a little as it's possible the block hasn't been sealed yet
	@sleep 20

	@echo "Terminating previous Madara instance..."
	$(call kill_pid,madara)

	@echo "Starting updated Madara..."
	@make madara & $(call save_pid,madara,$$!)
	@echo "Waiting for Madara to start..."
	@while ! curl -s -o /dev/null -w '%{http_code}' http://localhost:9944/health | grep -q '200'; do sleep 1; done
	@echo "Updated Madara started in background (PID: $$($(call get_pid,madara)))"

	@echo "Starting Pathfinder..."
	@make pathfinder & $(call save_pid,pathfinder,$$!)
	@echo "Waiting for Pathfinder to start..."
	@while ! curl -s -o /dev/null -w '%{http_code}' http://localhost:9545 | grep -q '200'; do sleep 1; done
	@echo "Pathfinder started in background (PID: $$($(call get_pid,pathfinder)))"

	@echo "Setting up Madara orchestrator"
	@make orchestrator-setup
	@echo "Madara orchestrator setup completed."

	@echo "Adding indexes to MongoDB"
	@make mongo-migrations
	@echo "Indexes added to MongoDB"

	@echo "Building Starknet OS"
	@if [ ! -f build/os_latest.json ]; then \
		make snos; \
	fi
	@echo "Starknet OS built"

	@make cleanup


anvil:
	anvil --block-time 1

define update_core_contract_address
	echo "Updating core contract address in YAML..."
	if [ -f "$(ENV_FILE)" ]; then \
		export CORE_CONTRACT_ADDRESS=$$(jq -r '.core_contract_address' $(BOOTSTRAP_INPUT_PATH)) && \
		if [ -n "$$CORE_CONTRACT_ADDRESS" ]; then \
			yq e '.eth_core_contract_address = strenv(CORE_CONTRACT_ADDRESS)' -i $(MADARA_PATH)/configs/presets/devnet.yaml; \
			echo "Core contract address updated in YAML."; \
		else \
			echo "Error: CORE_CONTRACT_ADDRESS not found in $(ENV_FILE)"; \
			exit 1; \
		fi; \
	else \
		echo "Error: $(ENV_FILE) not found"; \
		exit 1; \
	fi
endef

madara-bootstrap-mode:
	cd $(MADARA_PATH) && \
	git checkout $(MADARA_COMMIT) && \
	rm -rf $(MADARA_DATA_PATH) && \
	cargo run --release -- --name madara --base-path $(MADARA_DATA_PATH) --rpc-port 9944 --rpc-cors "*" --rpc-external --sequencer --chain-config-path configs/presets/devnet.yaml --feeder-gateway-enable --gateway-enable --gateway-external --gas-price 0 --blob-gas-price 0 --rpc-methods unsafe --no-l1-sync

core-contract:
	$(call update-json-safe,$(BOOTSTRAP_INPUT_PATH),operator_address,$(OPERATOR_ADDRESS)) && \
	$(call update-json-safe,$(BOOTSTRAP_INPUT_PATH),verifier_address,$(VERIFIER_ADDRESS)) && \
	echo "values with which we are updating the bootstrapper input file" && \
	cd $(BOOTSTRAPPER_PATH) && \
	rm -f $(BOOTSTRAP_OUTPUT_PATH) && \
	git checkout $(BOOTSTRAPPER_COMMIT) && \
	RUST_LOG=debug cargo run --release -- --mode setup-l1 --config $(BOOTSTRAP_INPUT_PATH) --output-file $(BOOTSTRAP_OUTPUT_PATH) && \
    $(call update-json-safe,$(BOOTSTRAP_INPUT_PATH),core_contract_address,$$(jq -r .starknet_contract_address $(BOOTSTRAP_OUTPUT_PATH))) && \
	$(call update-json-safe,$(BOOTSTRAP_INPUT_PATH),core_contract_implementation_address,$$(jq -r .starknet_contract_implementation_address $(BOOTSTRAP_OUTPUT_PATH)))
	

madara:
	$(call update_core_contract_address) && \
	cd $(MADARA_PATH) && \
	git checkout $(MADARA_COMMIT) && \
	cargo run --release -- --name madara --base-path $(MADARA_DATA_PATH) --rpc-port 9944 --rpc-cors "*" --rpc-external --sequencer --chain-config-path configs/presets/devnet.yaml --feeder-gateway-enable --gateway-enable --gateway-external --gas-price 2 --blob-gas-price 2 --strk-gas-price 2 --strk-blob-gas-price 2 --rpc-methods unsafe --l1-endpoint http://0.0.0.0:8545

eth-bridge:
	cd $(BOOTSTRAPPER_PATH) && \
	git checkout $(BOOTSTRAPPER_COMMIT) && \
	RUST_LOG=debug cargo run --release -- --mode eth-bridge --config $(BOOTSTRAP_INPUT_PATH) --output-file $(BOOTSTRAP_OUTPUT_PATH) && \
	$(call update-json-safe,$(ENV_FILE),L1_BRIDGE_ADDRESS,$$(jq -r .eth_bridge_setup_outputs.l1_bridge_address $(BOOTSTRAP_OUTPUT_PATH))) && \
    $(call update-json-safe,$(ENV_FILE),L2_ETH_TOKEN_ADDRESS,$$(jq -r .eth_bridge_setup_outputs.l2_eth_proxy_address $(BOOTSTRAP_OUTPUT_PATH))) && \
    $(call update-json-safe,$(ENV_FILE),L2_ETH_BRIDGE_ADDRESS,$$(jq -r .eth_bridge_setup_outputs.l2_eth_bridge_proxy_address $(BOOTSTRAP_OUTPUT_PATH)))

udc:
	cd $(BOOTSTRAPPER_PATH) && \
	git checkout $(BOOTSTRAPPER_COMMIT) && \
	RUST_LOG=debug cargo run --release -- --mode udc --config $(BOOTSTRAP_INPUT_PATH)

pathfinder:
	cd $(PATHFINDER_PATH) && \
	git checkout $(PATHFINDER_COMMIT) && \
	rm -rf $(PATHFINDER_DATA_PATH) && \
	cargo run --release --bin pathfinder -- --ethereum.url $(ETHEREUM_WSS_RPC_URL) --chain-id MADARA_DEVNET --feeder-gateway-url http://localhost:8080/feeder_gateway --gateway-url  http://localhost:8080/gateway --network custom --storage.state-tries archive --data-directory $(PATHFINDER_DATA_PATH)

mongo-migrations:
	cd $(ORCHESTRATOR_PATH) && \
	npm i && \
	npm run migrate up

orchestrator-setup:
	cd $(ORCHESTRATOR_PATH) && \
	npm i && \
	export L1_BRIDGE_ADDRESS=$$(jq -r '.L1_BRIDGE_ADDRESS' $(ENV_FILE)) && \
	export CORE_CONTRACT_ADDRESS=$$(jq -r '.core_contract_address' $(BOOTSTRAP_INPUT_PATH)) && \
	export L2_ETH_TOKEN_ADDRESS=$$(jq -r '.L2_ETH_TOKEN_ADDRESS' $(ENV_FILE)) && \
	export L2_ETH_BRIDGE_ADDRESS=$$(jq -r '.L2_ETH_BRIDGE_ADDRESS' $(ENV_FILE)) && \
	node scripts/init_state.js $$L1_BRIDGE_ADDRESS $$CORE_CONTRACT_ADDRESS $$L2_ETH_TOKEN_ADDRESS $$L2_ETH_BRIDGE_ADDRESS

orchestrator:
	@echo "Please update core contract"
	cd $(ORCHESTRATOR_PATH) && \
	cargo run --release --bin orchestrator

snos:
	python3.9 -m venv orchestrator_venv && \
	. ./orchestrator_venv/bin/activate && \
	pip install cairo-lang==0.13.2 "sympy<1.13.0" && \
	mkdir -p build && \
	git submodule update --init --recursive && \
	cd cairo-lang && \
	git checkout $(CAIRO_LANG_COMMIT) && \
	cd .. && \
	cairo-compile cairo-lang/src/starkware/starknet/core/os/os.cairo --output build/os_latest.json --cairo_path cairo-lang/src

cleanup:
	@echo "Cleaning up processes..."
	@if [ -f "$(PID_FILE)" ]; then \
		for pid in $$(jq -r 'values[]' "$(PID_FILE)"); do \
			kill $$pid 2>/dev/null || true; \
		done; \
		rm -f "$(PID_FILE)"; \
	fi
	@rm -f $(ENV_FILE)
	@echo "Cleanup completed."
