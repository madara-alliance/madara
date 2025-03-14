# GitHub Workflows

This directory contains the GitHub Actions workflows used in the Madara project.
This README explains how the workflows are organized, how to use them, and how to test them locally.

## Directory Structure

- **workflows/**: Contains all the GitHub Actions workflow files (`.yml`) that define CI/CD processes
- **actions/**: Contains reusable custom actions shared across workflows
- **config/**: Contains configuration files like `env.yml` used across workflows
- **ISSUE_TEMPLATE/**: Contains templates for GitHub issues
- **PULL_REQUEST_TEMPLATE.md**: Template for pull requests

## Workflow Architecture

The workflow system is built with a modular approach:

1. **Trigger Workflows**: Files like `pull-request-main-ci.yml` that trigger when events occur (PR creation, push to branches, etc.)
2. **Task Workflows**: Reusable workflow files prefixed with `task-*` that perform specific tasks
3. **Schedule Workflows**: Files prefixed with `schedule-*` that run on a defined schedule

The trigger workflows typically call multiple task workflows as needed.

## Naming convention

Using this convention: `event`-`category`-`functionality`.yml

- _event_: `push`, `push-main`, `schedule`, `pull-request`, `task` (specific to reusable components)...
- _category_: `ci`, `release`, `build`, `lint`, `security`, `test`,...
- _functionality_: `code-style`, `docker`, `backup`, `frontend`, .... anything that describe what it does

## Shared Actions

Custom actions in the `actions/` directory are used across workflows to standardize common tasks:

### load-env

Located in `.github/actions/load-env/`, this action loads environment variables from the config files. It:

- Reads variables from `.github/config/env.yml`
- Sets these as environment variables for all workflow steps
- Handles prioritization of existing environment variables (system environment overrides those in env.yml. This allows easy testing with `act`)

### rust-setup

Located in `.github/actions/rust-setup/`, this action sets up the Rust environment with:

- Appropriate Rust version installation based on env vars
- Cache configuration for dependencies
- Other Rust tools installation
- Builds SNOS if needed

## Environment Configuration

The environment configuration is centralized in `.github/config/env.yml`. This contains various environment variables used across workflows:

```yaml
env:
  BUILD_RUST_VERSION: 1.85
  BUILD_SCARB_VERSION: 2.8.2
  BUILD_PYTHON_VERSION: 3.9
  BUILD_ACTION_CACHE_PREFIX: "madara"
  BUILD_FOUNDRY_VERSION: nightly
  BUILD_RUST_CACHE_KEY: "release"

  TESTING_RUST_VERSION: nightly-2024-09-05
  TESTING_RUST_CACHE_KEY: "testing"

  DEV_RUST_CACHE_KEY: "dev"

  CLIPPY_RUST_CACHE_KEY: "clippy"
```

These variables define:

- Versions for build and testing tools
- Cache keys for various environments
- Prefixes and other configuration values

Having different cache for build environments allows to reduce the total cache size while still increasing speed of builds.

## Running Workflows Locally with Act

[Act](https://github.com/nektos/act) is a tool that allows you to run GitHub Actions workflows locally. This is useful for testing and debugging workflows without having to push changes to GitHub.

**Warning**: Currently `act` doesn't work well with multiple level of dockerization. Meaning workflows which are using `uses: task-...` as jobs (ex: `pull-request-...` or `push-...`) won't work. However all the `task-...` workflows should work properly.

### Common usages

- `-s GITHUB_TOKEN="$(gh auth token)"` to setup the token
- `--artifact-server-path ./tmp/artifacts/` to maintain artifacts used across tasks
- `-P karnot-arc-runner-set=catthehacker/ubuntu:rust-22.04` allows task using `runs-on: karnot-arc-runner-set` to execute using act's ubuntu image with rust preinstalled

```bash
# Run the main build task (takes ~10min on good computer)
act -W .github/workflows/task-build-madara.yml -s GITHUB_TOKEN="$(gh auth token)"  --artifact-server-path ./tmp/artifacts/ workflow_call -P karnot-arc-runner-set=catthehacker/ubuntu:rust-22.04

# Output about the binaries hashes:
# | madara-binary-hash: c2098b181b6d5d54fcb4a23df9df933d0b842fd615b00b7f2ef5e5783686b7b4
# | orchestrator-binary-hash: ce354e4078a0a9ff2303e50255415f54edde012151edbd650e42187e91a8f0c1
# | cairo-artifacts-hash: 07660f3d160f65ee5c66ea05bfd3f5ccc35ed1b4589bffc2230f2c798f0a5c15

# Run the javascript tests (using --input to pass the binaries)
act -W .github/workflows/task-test-js.yml -s GITHUB_TOKEN="$(gh auth token)"  --artifact-server-path ./tmp/artifacts/ workflow_call --input madara-binary-hash="c2098b181b6d5d54fcb4a23df9df933d0b842fd615b00b7f2ef5e5783686b7b4" --input cairo-artifacts-hash="07660f3d160f65ee5c66ea05bfd3f5ccc35ed1b4589bffc2230f2c798f0a5c15"

# Run Orchestrator Coverage (using -s for passing secrets)
act -W .github/workflows/task-coverage-orchestrator.yml -s GITHUB_TOKEN="$(gh auth token)" --artifact-server-path ./tmp/artifacts/ workflow_call -P karnot-arc-runner-set=catthehacker/ubuntu:rust-22.04 -s ETHEREUM_SEPOLIA_BLAST_RPC="https://sepolia.blast.io" -s RPC_FOR_SNOS="..."

# Independantly build the production docker image used in the release process
docker build -f docker/madara-production.Dockerfile . --build-arg="PYTHON_VERSION=3.9" --build-arg="SCARB_VERSION=2.8.2" --build-arg=FOUNDRY_VERSION=nightly
```

### Troubleshooting

If you encounter issues:

1. Check if Act is using the correct Docker images
2. Ensure all required secrets are provided
3. Check for path-specific issues between different environments
4. Look at act DEBUG logs `act -v`

### Best Practices

- Create a `.actrc` file in the repository root to store common flags
- Use the `-n` flag to do a dry run and see what would be executed
- For complex workflows, test individual jobs separately
