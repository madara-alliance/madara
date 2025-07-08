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

  - test-e2e          Run end-to-end tests
  - test-orchestrator Run unit tests with coverage report
  - test              Run all tests (e2e and unit)

  [ OTHER COMMANDS ]

  - help               Show this help message
  - git-hook           Setup git hooks path to .githooks

endef
export HELP

SECRETS        := .secrets/rpc_api.secret
DB_PATH        := /var/lib/madara

DOCKER_COMPOSE := docker compose -f compose.yaml
DOCKER_TAG     := madara:latest
DOCKER_IMAGE   := ghcr.io/madara-alliance/$(DOCKER_TAG)
DOCKER_GZ      := image.tar.gz
ARTIFACTS      := ./build-artifacts

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
	@if [ -d "$(ARTIFACTS)/argent"             ] || \
			[ -d "$(ARTIFACTS)/bravoos"            ] || \
			[ -d "$(ARTIFACTS)/cairo_lang"         ] || \
			[ -d "$(ARTIFACTS)/js_tests"           ] || \
			[ -d "$(ARTIFACTS)/orchestrator_tests" ] || \
			[ -d "$(ARTIFACTS)/starkgate_latest"   ] || \
			[ -d "$(ARTIFACTS)/starkgate_legacy"   ]; \
	then \
		echo -e "$(DIM)"artifacts" already exists, do you want to remove it?$(RESET) $(PASS)[y/N] $(RESET)" && \
		read ans && \
		case "$$ans" in \
			[yY]*) true;; \
		*) false;; \
	esac \
	fi
	@rm -rf "$(ARTIFACTS)/argent"
	@rm -rf "$(ARTIFACTS)/bravoos"
	@rm -rf "$(ARTIFACTS)/cairo_lang"
	@rm -rf "$(ARTIFACTS)/js_tests"
	@rm -rf "$(ARTIFACTS)/orchestrator_tests"
	@rm -rf "$(ARTIFACTS)/starkgate_latest"
	@rm -rf "$(ARTIFACTS)/starkgate_legacy"
	@docker build -f $(ARTIFACTS)/build.docker -t contracts .
	@ID=$$(docker create contracts do-nothing) && docker cp $${ID}:/artifacts/. $(ARTIFACTS) && docker rm $${ID} > /dev/null

.PHONY: check
check:
	@echo -e "$(DIM)Running code quality checks...$(RESET)"
	@echo -e "$(INFO)Running prettier check...$(RESET)"
	@npm install
	@npx prettier@3.5.3 --check .
	@echo -e "$(INFO)Running cargo fmt check...$(RESET)"
	@cargo fmt -- --check
	@echo -e "$(INFO)Running cargo clippy workspace checks...$(RESET)"
	@cargo clippy --workspace --no-deps -- -D warnings
	@echo -e "$(INFO)Running cargo clippy workspace tests...$(RESET)"
	@cargo clippy --workspace --tests --no-deps -- -D warnings
	@echo -e "$(INFO)Running cargo clippy with testing features...$(RESET)"
	@cargo clippy --workspace --exclude madara --features testing --no-deps -- -D warnings
	@echo -e "$(INFO)Running cargo clippy with testing features and tests...$(RESET)"
	@cargo clippy --workspace --exclude madara --features testing --tests --no-deps -- -D warnings
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

.PHONY: test-e2e
test-e2e:
	@echo -e "$(DIM)Running E2E tests...$(RESET)"
	@RUST_LOG=info cargo nextest run --release --features testing --workspace test_orchestrator_workflow -E 'test(test_orchestrator_workflow)' --no-fail-fast
	@echo -e "$(PASS)E2E tests completed!$(RESET)"

.PHONY: test-orchestrator
test-orchestrator:
	@echo -e "$(DIM)Running unit tests with coverage...$(RESET)"
	@RUST_LOG=debug RUST_BACKTRACE=1 cargo llvm-cov nextest \
		--release \
		--features testing \
		--lcov \
		--output-path lcov.info \
		--test-threads=1 \
		--package "orchestrator-*" \
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
	@cargo run --package orchestrator -- setup  --layer l2 --aws --aws-s3 --aws-sqs --aws-sns --aws-event-bridge --event-bridge-type schedule

.PHONY: setup-l3
setup-l3:
	@echo -e "$(DIM)Setting up orchestrator with L3 layer...$(RESET)"
	@cargo run --package orchestrator -- setup --layer l3 --aws --aws-s3 --aws-sqs --aws-sns --aws-event-bridge --event-bridge-type schedule

.PHONY: run-orchestrator-l2
run-orchestrator-l2:
	@echo -e "$(DIM)Running orchestrator...$(RESET)"
	@cargo run --release --package orchestrator -- run --layer l3 --aws --aws-s3 --aws-sqs --aws-sns --settle-on-ethereum --atlantic --da-on-ethereum 2>&1


.PHONY: run-orchestrator-l3
run-orchestrator-l3:
	@echo -e "$(DIM)Running orchestrator...$(RESET)"
	@cargo run --release --package orchestrator -- run --layer l3 --aws --aws-s3 --aws-sqs --aws-sns --settle-on-starknet --atlantic --da-on-starknet 2>&1

.PHONY: watch-orchestrator
watch-orchestrator:
	@echo -e "$(DIM)Watching orchestrator for changes...$(RESET)"
	@cargo watch -x 'run --release --package orchestrator -- run --layer l3 --aws --aws-s3 --aws-sqs --aws-sns --settle-on-starknet --atlantic --da-on-starknet' 2>&1
