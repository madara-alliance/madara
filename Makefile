# ============================================================================ #
#                              STARKNET NODE RUNNER                            #
# ============================================================================ #

define HELP
Madara Node Runner

Helper for running the starknet Madara node.

Usage:
  make <target>

Targets:

  [ SETUP ]

  - setup-l2                Setup orchestrator with L2 layer (default)
  - setup-l3                Setup orchestrator with L3 layer
  - run-orchestrator-l2     Run the orchestrator with AWS services and Ethereum settlement
  - run-orchestrator-l3     Run the orchestrator with AWS services and Starknet settlement

  [ RUNNING MADARA ]

  Runs Madara, automatically pulling the required image if it is not already
  available. Note that it is also required for you to have added the necessary
  secrets to `./secrets/`, or the nodes will fail to start.

  - start             Starts the Madara node
  - watch-orchestrator Run orchestrator with auto-restart on code changes

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

  [ CODE QUALITY ]

  Runs various code quality checks including formatting and linting.

  - check              Run code quality checks (fmt, clippy)
  - fmt                Format code using taplo and cargo fmt
  - pre-push         Run formatting and checks before committing / Pushing

  [ TESTING ]

  Runs various types of tests for the codebase.

  - test-orchestrator-e2e   Run end-to-end orchestrator workflow tests
  - test-e2e                Run end-to-end test
  - test-orchestrator       Run unit tests with coverage report
  - test                    Run all tests (e2e and unit)

  [ OTHER COMMANDS ]

  - help               Show this help message
  - git-hook           Setup git hooks path to .githooks
  - run-mock-atlantic-server  Run the mock Atlantic server (options: PORT=4002 FAILURE_RATE=0.1 MAX_CONCURRENT_JOBS=5 BIND_ADDR=0.0.0.0)

endef
export HELP

SECRETS        := .secrets/rpc_api.secret
DB_PATH        := /var/lib/madara

DOCKER_COMPOSE := docker compose -f compose.yaml
DOCKER_TAG     := madara:latest
DOCKER_IMAGE   := ghcr.io/madara-alliance/$(DOCKER_TAG)
DOCKER_GZ      := image.tar.gz
ARTIFACTS      := ./build-artifacts

# Configuration for E2E bridge tests
CARGO_TARGET_DIR ?= target
AWS_REGION ?= us-east-1
PATHFINDER_URL_MAC = https://github.com/karnotxyz/pathfinder/releases/download/v0.13.2.1-no-charge-fee/pathfinder-aarch64-apple-darwin.tar.gz

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
	@$(MAKE) --silent clean
	@echo -e "$(DIM)removing madara database on host$(RESET)"
	@rm -rf $(DB_PATH);

.PHONY: fclean
fclean: clean-db
	@echo -e "$(DIM)removing local images tar.gz$(RESET)"
	@rm -rf $(DOCKER_GZ)
	@echo -e "$(WARN)artefacts cleaned$(RESET)"

.PHONY: restart
restart: clean
	@$(MAKE) --silent start

.PHONY: frestart
frestart: fclean
	@$(MAKE) --silent start

.PHONY: artifacts
artifacts:
	./scripts/artifacts.sh


.PHONY: check
check:
	@echo -e "$(DIM)Running code quality checks...$(RESET)"
	@echo -e "$(INFO)Running prettier check...$(RESET)"
	@npm install
	@npx prettier --check .
	@echo -e "$(INFO)Running cargo fmt check...$(RESET)"
	@cargo fmt -- --check
	@echo -e "$(INFO)Running taplo fmt check...$(RESET)"
	@taplo fmt --config=./taplo/taplo.toml --check
	@echo -e "$(INFO)Running cargo clippy workspace checks...$(RESET)"
	@cargo clippy --workspace --no-deps -- -D warnings
	@echo -e "$(INFO)Running cargo clippy workspace tests...$(RESET)"
	@cargo clippy --workspace --tests --no-deps -- -D warnings
	@echo -e "$(INFO)Running cargo clippy with testing features...$(RESET)"
	@cargo clippy --workspace --exclude madara --features testing --no-deps -- -D warnings
	@echo -e "$(INFO)Running cargo clippy with testing features and tests...$(RESET)"
	@cargo clippy --workspace --exclude madara --features testing --tests --no-deps -- -D warnings
	@echo -e "$(INFO)Running markdownlint check...$(RESET)"
	@npx markdownlint -c .markdownlint.json -q -p .markdownlintignore .
	@echo -e "$(PASS)All code quality checks passed!$(RESET)"

.PHONY: fmt
fmt:
	@echo -e "$(DIM)Running code formatters...$(RESET)"
	@echo -e "$(INFO)Running taplo formatter...$(RESET)"
	@npm install
	@npx prettier --write .
	@echo -e "$(PASS)Code formatting complete!$(RESET)"
	@echo -e "$(DIM)Running code formatters...$(RESET)"
	@echo -e "$(INFO)Running taplo formatter...$(RESET)"
	@taplo format --config=./taplo/taplo.toml
	@echo -e "$(INFO)Running cargo fmt...$(RESET)"
	@cargo fmt
	@echo -e "$(PASS)Code formatting complete!$(RESET)"

.PHONY: test-orchestrator-e2e
test-orchestrator-e2e:
	@echo -e "$(DIM)Running E2E tests...$(RESET)"
	@RUST_LOG=info cargo nextest run --release --features testing --workspace test_orchestrator_workflow -E 'test(test_orchestrator_workflow)' --no-fail-fast
	@echo -e "$(PASS)E2E tests completed!$(RESET)"

# ============================================================================ #
#                          E2E BRIDGE TESTS (MAC)                              #
# ============================================================================ #

.PHONY: test-e2e
test-e2e: check-e2e-env check-e2e-mac check-e2e-dependencies pull-e2e-docker-images build-e2e-binaries download-pathfinder-mac make-e2e-binaries-executable run-e2e clean-up-after-e2e
	@echo -e "$(PASS)E2E test completed!$(RESET)"

.PHONY: check-e2e-env
check-e2e-env:
	@echo -e "$(DIM)Checking for MADARA_ORCHESTRATOR_ATLANTIC_API_KEY in .env.e2e...$(RESET)"
	@if [ ! -f .env.e2e ]; then \
		echo -e "$(WARN)⚠️  WARNING: .env.e2e file not found!$(RESET)"; \
		echo -e "$(WARN)⚠️  Please create .env.e2e and add MADARA_ORCHESTRATOR_ATLANTIC_API_KEY$(RESET)"; \
		echo -e "$(DIM)Press Enter to continue or Ctrl+C to cancel...$(RESET)"; \
		read -r; \
	elif ! grep -v "^[[:space:]]*#" .env.e2e | grep -q "MADARA_ORCHESTRATOR_ATLANTIC_API_KEY"; then \
		echo -e "$(WARN)⚠️  WARNING: MADARA_ORCHESTRATOR_ATLANTIC_API_KEY not found in .env.e2e$(RESET)"; \
		echo -e "$(WARN)⚠️  Please add MADARA_ORCHESTRATOR_ATLANTIC_API_KEY to .env.e2e$(RESET)"; \
		echo -e "$(DIM)Press Enter to continue or Ctrl+C to cancel...$(RESET)"; \
		read -r; \
	else \
		API_KEY=$$(grep -v "^[[:space:]]*#" .env.e2e | grep "MADARA_ORCHESTRATOR_ATLANTIC_API_KEY" | cut -d '=' -f 2 | tr -d ' "'); \
		if [ -z "$$API_KEY" ]; then \
			echo -e "$(WARN)⚠️  WARNING: MADARA_ORCHESTRATOR_ATLANTIC_API_KEY is empty in .env.e2e$(RESET)"; \
			echo -e "$(DIM)Press Enter to continue or Ctrl+C to cancel...$(RESET)"; \
			read -r; \
		else \
			MASKED_KEY=$$(echo $$API_KEY | sed 's/\(.\{4\}\).*/\1****/'); \
			echo -e "$(PASS)✅ Found MADARA_ORCHESTRATOR_ATLANTIC_API_KEY: $$MASKED_KEY$(RESET)"; \
		fi; \
	fi
	@# Check for CARGO_TARGET_DIR in .env.e2e
	@if [ -f .env.e2e ]; then \
		if grep -v "^[[:space:]]*#" .env.e2e | grep -q "CARGO_TARGET_DIR"; then \
			ENV_TARGET_DIR=$$(grep -v "^[[:space:]]*#" .env.e2e | grep "CARGO_TARGET_DIR" | cut -d '=' -f 2 | tr -d ' "'); \
			if [ "$$ENV_TARGET_DIR" != "$(CARGO_TARGET_DIR)" ]; then \
				echo -e "$(WARN)⚠️  WARNING: CARGO_TARGET_DIR in .env.e2e ($$ENV_TARGET_DIR) differs from Makefile value$(RESET)"; \
				echo -e "$(INFO)Updating .env.e2e to use: $(CARGO_TARGET_DIR)$(RESET)"; \
				sed -i.bak '/^[[:space:]]*CARGO_TARGET_DIR/d' .env.e2e && rm -f .env.e2e.bak; \
				echo "CARGO_TARGET_DIR=$(CARGO_TARGET_DIR)" >> .env.e2e; \
			else \
				echo -e "$(PASS)✅ CARGO_TARGET_DIR already set correctly: $(CARGO_TARGET_DIR)$(RESET)"; \
			fi; \
		else \
			echo -e "$(INFO)Adding CARGO_TARGET_DIR to .env.e2e: $(CARGO_TARGET_DIR)$(RESET)"; \
			echo "CARGO_TARGET_DIR=$(CARGO_TARGET_DIR)" >> .env.e2e; \
		fi; \
	fi

.PHONY: check-e2e-mac
check-e2e-mac:
	@echo -e "$(DIM)Checking if running on Mac...$(RESET)"
	@if [ "$$(uname)" != "Darwin" ]; then \
		echo -e "$(WARN)❌ This test must be run on macOS$(RESET)"; \
		echo -e "$(INFO)Detected OS: $$(uname)$(RESET)"; \
		exit 1; \
	fi
	@echo -e "$(PASS)✅ Running on macOS$(RESET)"

.PHONY: check-e2e-dependencies
check-e2e-dependencies:
	@echo -e "$(DIM)Checking E2E dependencies...$(RESET)"
	@# Check Docker installation
	@if ! command -v docker &> /dev/null; then \
		echo -e "$(WARN)❌ Docker is not installed or not in PATH$(RESET)"; \
		exit 1; \
	fi
	@echo -e "$(PASS)✅ Docker is installed$(RESET)"
	@# Check if Docker daemon is running
	@if ! docker info &> /dev/null 2>&1; then \
		echo -e "$(WARN)❌ Docker daemon is not running. Please start Docker.$(RESET)"; \
		exit 1; \
	fi
	@echo -e "$(PASS)✅ Docker daemon is running$(RESET)"
	@# Check Anvil installation
	@if ! command -v anvil &> /dev/null; then \
		echo -e "$(WARN)❌ Anvil is not installed or not in PATH$(RESET)"; \
		exit 1; \
	fi
	@echo -e "$(PASS)✅ Anvil is installed$(RESET)"
	@# Check Forge installation
	@if ! command -v forge &> /dev/null; then \
		echo -e "$(WARN)❌ Forge is not installed or not in PATH$(RESET)"; \
		exit 1; \
	fi
	@echo -e "$(PASS)✅ Forge is installed$(RESET)"

.PHONY: pull-e2e-docker-images
pull-e2e-docker-images:
	@echo -e "$(DIM)Checking Docker images for E2E tests...$(RESET)"
	@if ! docker image inspect localstack/localstack@sha256:763947722c6c8d33d5fbf7e8d52b4bddec5be35274a0998fdc6176d733375314 > /dev/null 2>&1; then \
		echo -e "$(INFO)Pulling localstack image...$(RESET)"; \
		docker pull localstack/localstack@sha256:763947722c6c8d33d5fbf7e8d52b4bddec5be35274a0998fdc6176d733375314; \
	else \
		echo -e "$(PASS)✅ LocalStack image already exists$(RESET)"; \
	fi
	@if ! docker image inspect mongo:latest > /dev/null 2>&1; then \
		echo -e "$(INFO)Pulling mongo image...$(RESET)"; \
		docker pull mongo:latest; \
	else \
		echo -e "$(PASS)✅ Mongo image already exists$(RESET)"; \
	fi
	@echo -e "$(PASS)✅ All Docker images ready$(RESET)"


.PHONY: build-e2e-binaries
build-e2e-binaries:
	@echo -e "$(DIM)Building E2E binaries...$(RESET)"
	@mkdir -p $(CARGO_TARGET_DIR)/release
	@# Build Madara
	@echo -e "$(INFO)Building Madara...$(RESET)"
	@CARGO_TARGET_DIR=$(CARGO_TARGET_DIR) cargo build --manifest-path madara/Cargo.toml --bin madara --release
	@# Build Orchestrator
	@echo -e "$(INFO)Building Orchestrator...$(RESET)"
	@CARGO_TARGET_DIR=$(CARGO_TARGET_DIR) cargo build --package orchestrator --bin orchestrator --release
	@# Build Bootstrapper
	@echo -e "$(INFO)Building Bootstrapper...$(RESET)"
	@CARGO_TARGET_DIR=$(CARGO_TARGET_DIR) cargo build --package bootstrapper --bin bootstrapper --release
	@# Build E2E test package
	@echo -e "$(INFO)Building E2E test package...$(RESET)"
	@CARGO_TARGET_DIR=$(CARGO_TARGET_DIR) cargo build -p e2e
	@echo -e "$(PASS)✅ All binaries built$(RESET)"

.PHONY: download-pathfinder-mac
download-pathfinder-mac:
	@echo -e "$(DIM)Downloading Pathfinder binary for Mac...$(RESET)"
	@mkdir -p $(CARGO_TARGET_DIR)/release
	@if [ ! -f $(CARGO_TARGET_DIR)/release/pathfinder ]; then \
		curl -L -o pathfinder.tar.gz $(PATHFINDER_URL_MAC); \
		tar -xf pathfinder.tar.gz -C $(CARGO_TARGET_DIR)/release/; \
		rm pathfinder.tar.gz; \
		echo -e "$(PASS)✅ Pathfinder downloaded$(RESET)"; \
	else \
		echo -e "$(INFO)Pathfinder binary already exists, skipping download.$(RESET)"; \
	fi

.PHONY: make-e2e-binaries-executable
make-e2e-binaries-executable:
	@echo -e "$(DIM)Making binaries executable...$(RESET)"
	@chmod +x $(CARGO_TARGET_DIR)/release/madara
	@chmod +x $(CARGO_TARGET_DIR)/release/bootstrapper
	@chmod +x $(CARGO_TARGET_DIR)/release/pathfinder
	@chmod +x $(CARGO_TARGET_DIR)/release/orchestrator
	@chmod +x test_utils/scripts/deploy_dummy_verifier.sh
	@echo -e "$(PASS)✅ Binaries are executable$(RESET)"

.PHONY: run-e2e
run-e2e:
	@echo -e "$(DIM)Running E2E bridge tests...$(RESET)"
	@AWS_REGION=$(AWS_REGION) \
	CARGO_TARGET_DIR=$(CARGO_TARGET_DIR) \
	RUST_LOG=info cargo test \
		--package e2e test_bridge_deposit_and_withdraw \
		-- --test-threads=10 --nocapture
	@echo -e "$(PASS)✅ E2E bridge tests completed$(RESET)"

.PHONY: clean-up-after-e2e
clean-up-after-e2e:
	@echo -e "$(DIM)Cleaning up e2e_data directory...$(RESET)"
	@rm -rf e2e_data
	@echo -e "$(PASS)✅ e2e_data directory cleaned$(RESET)"

# ============================================================================ #

.PHONY: test-orchestrator
test-orchestrator:
	@echo -e "$(DIM)Running unit tests with coverage...$(RESET)"
	@RUST_LOG=debug RUST_BACKTRACE=1 cargo llvm-cov nextest \
		--release \
		--features testing \
		--lcov \
		--output-path lcov.info \
		--test-threads=1 \
		--package "orchestrator*" \
		--no-fail-fast
	@echo -e "$(PASS)Unit tests completed!$(RESET)"

.PHONY: test
test: test-e2e test-orchestrator
	@echo -e "$(PASS)All tests completed!$(RESET)"

.PHONY: pre-push
pre-push:
	@echo -e "$(DIM)Running pre-push checks...$(RESET)"
	@echo -e "$(INFO)Running code quality checks...$(RESET)"
	@$(MAKE) --silent check
	@echo -e "$(PASS)Pre-push checks completed successfully!$(RESET)"

.PHONY: git-hook
git-hook:
	@git config core.hooksPath .githooks

.PHONY: setup-l2
setup-l2:
	@echo -e "$(DIM)Setting up orchestrator with L2 layer...$(RESET)"
	@cargo run --package orchestrator -- setup --layer l2 --aws --aws-s3 --aws-sqs --aws-sns --aws-event-bridge --event-bridge-type rule

.PHONY: setup-l3
setup-l3:
	@echo -e "$(DIM)Setting up orchestrator with L3 layer...$(RESET)"
	@cargo run --package orchestrator -- setup --layer l3 --aws --aws-s3 --aws-sqs --aws-sns --aws-event-bridge --event-bridge-type rule

.PHONY: run-orchestrator-l2
run-orchestrator-l2:
	@echo -e "$(DIM)Running orchestrator...$(RESET)"
	@cargo run --release --package orchestrator -- run --layer l2 --aws --aws-s3 --aws-sqs --aws-sns --settle-on-ethereum --atlantic --da-on-ethereum --madara-version 0.13.2 2>&1


.PHONY: run-orchestrator-l3
run-orchestrator-l3:
	@echo -e "$(DIM)Running orchestrator...$(RESET)"
	@cargo run --release --package orchestrator -- run --layer l3 --aws --aws-s3 --aws-sqs --aws-sns --settle-on-starknet --atlantic --mock-atlantic-server --da-on-starknet 2>&1

.PHONY: watch-orchestrator
watch-orchestrator:
	@echo -e "$(DIM)Watching orchestrator for changes...$(RESET)"
	@cargo watch -x 'run --release --package orchestrator -- run --layer l3 --aws --aws-s3 --aws-sqs --aws-sns --settle-on-starknet --atlantic --da-on-starknet' 2>&1

# Run the mock Atlantic server with enhanced CLI
# Usage: make run-mock-atlantic-server
#        PORT=4002 make run-mock-atlantic-server                    # Custom port
#        MAX_CONCURRENT_JOBS=5 make run-mock-atlantic-server        # Limit concurrent jobs
#        PORT=8080 FAILURE_RATE=0.2 MAX_CONCURRENT_JOBS=3 make run-mock-atlantic-server  # All options
.PHONY: run-mock-atlantic-server
run-mock-atlantic-server:
	@echo -e "$(DIM)Starting mock Atlantic server...$(RESET)"
	@CMD="cargo run --release --package utils-mock-atlantic-server --"; \
	if [ ! -z "$(PORT)" ]; then \
		CMD="$$CMD --port $(PORT)"; \
		echo -e "$(INFO)Using custom port $(PORT)$(RESET)"; \
	fi; \
	if [ ! -z "$(FAILURE_RATE)" ]; then \
		CMD="$$CMD --failure-rate $(FAILURE_RATE)"; \
		echo -e "$(INFO)Using failure rate $(FAILURE_RATE)$(RESET)"; \
	fi; \
	if [ ! -z "$(BIND_ADDR)" ]; then \
		CMD="$$CMD --bind-addr $(BIND_ADDR)"; \
		echo -e "$(INFO)Binding to address $(BIND_ADDR)$(RESET)"; \
	fi; \
	if [ ! -z "$(MAX_CONCURRENT_JOBS)" ]; then \
		CMD="$$CMD --max-concurrent-jobs $(MAX_CONCURRENT_JOBS)"; \
		echo -e "$(INFO)Max concurrent jobs: $(MAX_CONCURRENT_JOBS)$(RESET)"; \
	fi; \
	$$CMD

.PHONY: setup-bootstrapper
setup-bootstrapper:
	@echo -e "$(DIM)Setting up bootstrapper...$(RESET)"
	@cp -r ./build-artifacts/bootstrapper/solidity/starkware/ ./bootstrapper-v2/contracts/ethereum/src/starkware/
	@cp -r ./build-artifacts/bootstrapper/solidity/third_party/ ./bootstrapper-v2/contracts/ethereum/src/third_party/
	@cp -r ./build-artifacts/bootstrapper/solidity/out/ ./bootstrapper-v2/contracts/ethereum/out/
	@cp -r ./build-artifacts/bootstrapper/cairo/target/ ./bootstrapper-v2/contracts/madara/target/
	@echo -e "$(PASS)Bootstrapper setup complete!$(RESET)"
