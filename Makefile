# ============================================================================ #
#                              STARKNET NODE RUNNER                            #
# ============================================================================ #

define HELP
Madara Node Runner

Helper for running the starknet Madara node.

Usage:
  make <target>

Targets:

  [ RUNNING MADARA ]

  Runs Madara, automatically pulling the required image if it is not already
  available. Note that it is also required for you to have added the necessary
  secrets to `./secrets/`, or the nodes will fail to start.

  - start             Starts the Madara node

  [ STOPPING MADARA ]

  Note that this will only pause container execution and not delete it, its
  volume or image.

  - stop              Stops the Madara node

  [ RESTARTING MADARA ]

  Restarts Madara, possibly cleaning containers, images and volumes in the
  process. Note that it is also required for you to have added the necessary
  secrets to `./secrets/`, or the nodes will fail to restart.

  - restart           Restarts the Madara node
  - frestart          Perform a full clean and restarts the Madara node

  [ LOGGING MADARA ]

  This will show logging outputs for the Madara container. Defaults to following
  the output, <Ctrl-C> to quit.

  - logs              View logs for Madara

  [ DOWLOAD DEPENDENCIES ]

  Images are downloaded from the github container registry. Note that to avoid
  continuousy downloading images those are exported to a `tar.gz` as artefacts.

  - images             Downloads the Madara Docker image

  [ CLEANING DEPENDECIES ]

  Will remove running containers, images and even local db. Use the latter with
  care as removing the local db will force a resync from genesys.

  - clean              Stop containers and prune images
  - clean-db           Perform clean and remove local database
  - fclean             Perform clean-db and remove local images

  [ OTHER COMMANDS ]

  - help               Show this help message

endef
export HELP

SECRETS        := .secrets/rpc_api.secret
DB_PATH        := /var/lib/madara

DOCKER_COMPOSE := docker compose -f compose.yaml
DOCKER_TAG     := madara:latest
DOCKER_IMAGE   := ghcr.io/madara-alliance/$(DOCKER_TAG)
DOCKER_GZ      := image.tar.gz

# dim white italic
DIM            := \033[2;3;37m

# bold cyan
INFO           := \033[1;36m

# bold green
PASS           := \033[1;32m

# bold red
WARN           := \033[1;31m

RESET          := \033[0m

.PHONY: all
all: help

.PHONY: help
help:
	@echo "$$HELP"

.PHONY: start
start: images $(SECRETS)
	@echo -e "$(DIM)running$(RESET) $(PASS)madara$(RESET)"
	@$(DOCKER_COMPOSE) up -d

.PHONY: stop
stop:
	@echo -e "$(DIM)stopping$(RESET) $(WARN)madara$(RESET)"
	@$(DOCKER_COMPOSE) stop

.PHONY: logs
logs:
	@echo -e "$(DIM)logs for$(RESET) $(INFO)madara$(RESET)";
	@$(DOCKER_COMPOSE) logs -f -n 100 madara;

.PHONY: images
images: $(DOCKER_GZ)

$(DOCKER_GZ):
	@echo -e "$(DIM)downloading$(RESET) $(PASS)madara$(RESET)"
	@docker pull $(DOCKER_IMAGE)
	@docker tag $(DOCKER_IMAGE) $(DOCKER_TAG)
	@docker rmi $(DOCKER_IMAGE)
	@docker image save -o $(DOCKER_GZ) $(DOCKER_TAG)

.PHONY: clean
clean: stop
	@echo -e "$(DIM)pruning containers$(RESET)"
	@docker container prune -f
	@echo -e "$(DIM)pruning images$(RESET)"
	@docker image prune -f
	@echo -e "$(WARN)images cleaned$(RESET)"

.PHONY: clean-db
clean-db:
	@echo -e "$(WARN)This action will result in irrecoverable loss of data!$(RESET)"
	@echo -e "$(DIM)Are you sure you want to proceed?$(RESET) $(PASS)[y/N] $(RESET)" && \
	read ans && \
	case "$$ans" in \
		[yY]*) true;; \
		*) false;; \
	esac
	@make --silent clean
	@echo -e "$(DIM)removing madara database on host$(RESET)"
	@rm -rf $(DB_PATH);

.PHONY: fclean
fclean: clean-db
	@echo -e "$(DIM)removing local images tar.gz$(RESET)"
	@rm -rf $(DOCKER_GZ)
	@echo -e "$(WARN)artefacts cleaned$(RESET)"

.PHONY: restart
restart: clean
	@make --silent start

.PHONY: frestart
frestart: fclean
	@make --silent start


snos:
	rm -rf orchestrator_venv && \
	python3.9 -m venv orchestrator_venv && \
	. ./orchestrator_venv/bin/activate && \
	pip install cairo-lang==0.13.2 "sympy<1.13.0" && \
	mkdir -p orchestrator/build && \
	git submodule update --init --recursive && \
	cd orchestrator/cairo-lang && \
	git checkout $(CAIRO_LANG_COMMIT) && \
	cd ../.. && \
	cairo-compile orchestrator/cairo-lang/src/starkware/starknet/core/os/os.cairo --output orchestrator/build/os_latest.json --cairo_path orchestrator/cairo-lang/src

# =============================================================================
# Starknet Contract Build System
# =============================================================================
# This Makefile manages the build process for various Starknet contracts including
# StarkGate, Braavos, and Argent. It handles both legacy and latest versions,
# supporting multiple architectures (amd64/arm64).

.PHONY: setup setup-linux ensure-asdf starkgate-contracts-latest braavos-account-cairo argent-contracts-starknet artifacts starkgate-contracts-legacy

# =============================================================================
# Configuration Variables
# =============================================================================

# Contract version tags and commit hashes
STARKGATE_CONTRACTS_VERSION_TAG="v2.0.1"
ARGENT_CONTRACTS_COMMIT_HASH="1352198956f36fb35fa544c4e46a3507a3ec20e3"
BRAAVOS_CONTRACTS_COMMIT_HASH="12b82a87b93ba9bfdf2cbbde2566437df2e0c6c8"

# Environment setup
SHELL := /bin/bash
HOME_DIR := $(HOME)

# Virtual environment paths
VENV_DIR := $(PWD)/venv
VENV_BIN := $(VENV_DIR)/bin
PYTHON := $(VENV_BIN)/python
PIP := $(VENV_BIN)/pip
SOLC_SELECT := $(VENV_BIN)/solc-select
SOLC ?= $(VENV_BIN)/solc

# =============================================================================
# Virtual Environment Setup
# =============================================================================

# Target: setup-venv
# Creates and configures Python virtual environment with required dependencies
setup-venv:
	python3 -m venv $(VENV_DIR)
	$(PIP) install --upgrade pip
	$(PIP) install solc-select
	$(SOLC_SELECT) install 0.8.24
	$(SOLC_SELECT) use 0.8.24
	$(SOLC) --version

# =============================================================================
# ASDF Version Manager Setup
# =============================================================================

# Target: ensure-asdf
# Installs and configures ASDF version manager for Scarb
# Uses direct git clone installation for all platforms (macOS/Linux)
ensure-asdf:
	@if [ ! -f "$(HOME_DIR)/.asdf/asdf.sh" ]; then \
		echo "Cleaning up existing ASDF installation..."; \
		rm -rf $(HOME_DIR)/.asdf; \
		echo "Installing ASDF..."; \
		git clone https://github.com/asdf-vm/asdf.git $(HOME_DIR)/.asdf --branch v0.14.1; \
		echo '. "$(HOME_DIR)/.asdf/asdf.sh"' >> $(HOME_DIR)/.bashrc; \
	fi; \
	if [ -f "$(HOME_DIR)/.asdf/asdf.sh" ]; then \
		. "$(HOME_DIR)/.asdf/asdf.sh" && \
		if ! asdf plugin list | grep -q scarb; then \
			asdf plugin add scarb https://github.com/software-mansion/asdf-scarb.git; \
		fi; \
	else \
		echo "Failed to install ASDF properly"; \
		exit 1; \
	fi

# =============================================================================
# Cairo Environment Setup
# =============================================================================

# Target: setup
# Sets up Cairo environment for macOS (arm64/amd64)
setup:
	mkdir -p .cairo
	cd .cairo && \
	wget -c https://github.com/starkware-libs/cairo/releases/download/v2.3.0/release-aarch64-apple-darwin.tar -O - | tar -xz

# Target: setup-linux
# Sets up Cairo environment for Linux (x86_64)
setup-linux:
	mkdir -p .cairo
	cd .cairo && \
	wget -c https://github.com/starkware-libs/cairo/releases/download/v2.3.0/release-x86_64-unknown-linux-musl.tar.gz -O - | tar -xz

# =============================================================================
# Contract Building Targets
# =============================================================================

# Target: starkgate-contracts-latest
# Builds the latest version of StarkGate contracts
# Prerequisites: setup-venv
starkgate-contracts-latest: setup-venv
	# Building L2 contracts
	# =====================
	cd lib/starkgate-contracts-latest && \
 	git checkout $(STARKGATE_CONTRACTS_VERSION_TAG) && \
 	rm -rf starkware && \
 	tar xvf .dep/starkware-solidity-dependencies.tar && \
 	mkdir -p cairo_contracts && \
	../../.cairo/cairo/bin/starknet-compile src  --contract-path src::strk::erc20_lockable::ERC20Lockable cairo_contracts/ERC20Lockable.sierra && \
	../../.cairo/cairo/bin/starknet-compile src  --contract-path src::token_bridge::TokenBridge cairo_contracts/TokenBridge.sierra && \
	../../.cairo/cairo/bin/starknet-compile src  --contract-path openzeppelin::token::erc20_v070::erc20::ERC20 cairo_contracts/ERC20.sierra && \
	../../.cairo/cairo/bin/starknet-compile src  --contract-path src::legacy_bridge_eic::LegacyBridgeUpgradeEIC cairo_contracts/LegacyBridgeUpgradeEIC.sierra && \
	../../.cairo/cairo/bin/starknet-compile src  --contract-path src::roles_init_eic::RolesExternalInitializer cairo_contracts/RolesExternalInitializer.sierra
	# Compiling Casm
	./.cairo/cairo/bin/starknet-sierra-compile ./lib/starkgate-contracts-latest/cairo_contracts/ERC20Lockable.sierra ./lib/starkgate-contracts-latest/cairo_contracts/ERC20Lockable.casm
	./.cairo/cairo/bin/starknet-sierra-compile ./lib/starkgate-contracts-latest/cairo_contracts/TokenBridge.sierra ./lib/starkgate-contracts-latest/cairo_contracts/TokenBridge.casm
	./.cairo/cairo/bin/starknet-sierra-compile ./lib/starkgate-contracts-latest/cairo_contracts/LegacyBridgeUpgradeEIC.sierra ./lib/starkgate-contracts-latest/cairo_contracts/LegacyBridgeUpgradeEIC.casm
	./.cairo/cairo/bin/starknet-sierra-compile ./lib/starkgate-contracts-latest/cairo_contracts/RolesExternalInitializer.sierra ./lib/starkgate-contracts-latest/cairo_contracts/RolesExternalInitializer.casm
	./.cairo/cairo/bin/starknet-sierra-compile ./lib/starkgate-contracts-latest/cairo_contracts/ERC20.sierra ./lib/starkgate-contracts-latest/cairo_contracts/ERC20.casm
	# Copying Contracts
	mkdir -p artifacts
	cp ./lib/starkgate-contracts-latest/cairo_contracts/ERC20Lockable.sierra ./artifacts/erc20.sierra.json
	cp ./lib/starkgate-contracts-latest/cairo_contracts/ERC20Lockable.casm ./artifacts/erc20.casm.json
	cp ./lib/starkgate-contracts-latest/cairo_contracts/TokenBridge.sierra ./artifacts/token_bridge.sierra.json
	cp ./lib/starkgate-contracts-latest/cairo_contracts/TokenBridge.casm ./artifacts/token_bridge.casm.json
	cp ./lib/starkgate-contracts-latest/cairo_contracts/LegacyBridgeUpgradeEIC.sierra ./artifacts/token_bridge_eic.sierra.json
	cp ./lib/starkgate-contracts-latest/cairo_contracts/LegacyBridgeUpgradeEIC.casm ./artifacts/token_bridge_eic.casm.json
	cp ./lib/starkgate-contracts-latest/cairo_contracts/RolesExternalInitializer.sierra ./artifacts/eth_token_eic.sierra.json
	cp ./lib/starkgate-contracts-latest/cairo_contracts/RolesExternalInitializer.casm ./artifacts/eth_token_eic.casm.json
	cp ./lib/starkgate-contracts-latest/cairo_contracts/ERC20.sierra ./artifacts/ERC20_070.sierra.json
	cp ./lib/starkgate-contracts-latest/cairo_contracts/ERC20.casm ./artifacts/ERC20_070.casm.json
	# Building L1 contracts
	# =====================
	# Configure solidity version
	$(SOLC_SELECT) install 0.8.24 && $(SOLC_SELECT) use 0.8.24 && \
	cd lib/starkgate-contracts-latest && \
	./scripts/setup.sh && \
	FILES=$$(cat src/solidity/files_to_compile.txt) && \
	$(SOLC) $$FILES --allow-paths .=., --optimize --optimize-runs 200 --overwrite --combined-json abi,bin -o artifacts && \
	./scripts/extract_artifacts.py	
	# Copying Contracts
	mkdir -p artifacts/upgrade-contracts
	cp lib/starkgate-contracts-latest/artifacts/StarknetEthBridge.json artifacts/upgrade-contracts/eth_bridge_upgraded.json
	cp lib/starkgate-contracts-latest/artifacts/StarkgateUpgradeAssistExternalInitializer.json artifacts/upgrade-contracts/eic_eth_bridge.json

# Target: starkgate-contracts-legacy
# Builds legacy StarkGate contracts using Docker
starkgate-contracts-legacy:
	# Building Contracts
	rm -rf lib/starkgate-contracts-old/Dockerfile
	cp ./build-artifacts/Dockerfile ./lib/starkgate-contracts-old/Dockerfile
	cd lib/starkgate-contracts-old && \
	docker build -t starkgate-build . && \
	mkdir -p starkgate-artifacts && \
	docker run -v ./starkgate-artifacts/:/mnt starkgate-build
	# Copying Contracts
	mkdir -p artifacts
	cp ./lib/starkgate-contracts-old/starkgate-artifacts/starkware/starknet/apps/starkgate/artifacts/cairo/token_bridge_1.json ./artifacts/legacy_token_bridge.json
	cp ./lib/starkgate-contracts-old/starkgate-artifacts/starkware/starknet/std_contracts/upgradability_proxy/proxy.json ./artifacts/proxy_starkgate.json
	cp ./lib/starkgate-contracts-old/starkgate-artifacts/starkware/starknet/std_contracts/ERC20/ERC20.json ./artifacts/ERC20.json

# Helper function for Scarb-based builds
define scarb_build
	@. "$(HOME_DIR)/.asdf/asdf.sh" && \
	cd $(1) && \
	git checkout $(2) && \
	asdf install scarb $(3) && \
	echo "scarb $(3)" > .tool-versions && \
	scarb build
endef

# Target: braavos-account-cairo
# Builds Braavos account contracts
# Prerequisites: ensure-asdf
braavos-account-cairo: ensure-asdf
	# Building
	$(call scarb_build,./lib/braavos-account-cairo,$(BRAAVOS_CONTRACTS_COMMIT_HASH),2.8.4)
	# Copying Contracts
	cp ./lib/braavos-account-cairo/target/dev/braavos_account_BraavosAccount.contract_class.json ./artifacts/BraavosAccount.sierra.json
	cp ./lib/braavos-account-cairo/target/dev/braavos_account_BraavosAccount.compiled_contract_class.json ./artifacts/BraavosAccount.casm.json
	cp ./lib/braavos-account-cairo/target/dev/braavos_account_BraavosBaseAccount.contract_class.json ./artifacts/BraavosBaseAccount.sierra.json
	cp ./lib/braavos-account-cairo/target/dev/braavos_account_BraavosBaseAccount.compiled_contract_class.json ./artifacts/BraavosBaseAccount.casm.json

# Target: argent-contracts-starknet
# Builds Argent account contracts
# Prerequisites: ensure-asdf
argent-contracts-starknet: ensure-asdf
	# Building
	$(call scarb_build,./lib/argent-contracts-starknet,$(ARGENT_CONTRACTS_COMMIT_HASH),2.6.3)
	# Copying Contracts
	cp ./lib/argent-contracts-starknet/target/dev/argent_ArgentAccount.contract_class.json ./artifacts/ArgentAccount.sierra.json
	cp ./lib/argent-contracts-starknet/target/dev/argent_ArgentAccount.compiled_contract_class.json ./artifacts/ArgentAccount.casm.json

# =============================================================================
# Main Build Targets
# =============================================================================

# Target: build-contracts
# Builds all contracts (legacy and latest versions)
build-contracts:
	make starkgate-contracts-legacy
	make starkgate-contracts-latest
	make braavos-account-cairo
	make argent-contracts-starknet

# Target: artifacts-linux
# Builds all artifacts for Linux environment
artifacts-linux:
	make setup-linux
	make build-contracts

# Target: artifacts
# Builds all artifacts for macOS environment
artifacts:
	make setup
	make build-contracts