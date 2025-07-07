# GitHub Workflows

This directory contains the GitHub Actions workflows used in the Madara project.
This README explains how the workflows are organized and how to use them.

## Directory Structure

- **workflows/**: Contains all the GitHub Actions workflow files (`.yml`) that define CI/CD processes.
- **actions/**: Contains reusable custom actions shared across workflows.
- **config/**: Contains configuration files like `env.yml` used across workflows.
- **ISSUE_TEMPLATE/**: Contains templates for GitHub issues.
- **PULL_REQUEST_TEMPLATE.md**: Template for pull requests.

## Workflow Architecture

The workflow system is built around 3 phases:

1. **Artifact and database updates**: run on pull requests with a specific label, automatically updates build-time version info.
2. **Syntax checks**: makes sure that the changes follow clear style guidelines (prettier, clippy, taplo).
3. **Compilation checks**: makes sure that each target in the repository can compile (madara, orchestrator, bootstrapper).
4. **Unit tests**: runs a battery of tests against each target to make sure that nothing breaks.

> [!TIP]
> Tasks are run in parallel as much as possible and make heavy usage of cached dependencies to minimize CI runtime.

## Shared Actions

Custom actions in the `actions/` directory are used across workflows to standardize common tasks:

### load-env

Located in `.github/actions/load-env/`, this action loads environment variables from the config files. It:

- Reads variables from `.github/config/env.yml`
- Sets these as environment variables for all workflow steps

### setup-rust

Located in `.github/actions/setup-rust/`, this action sets up the Rust environment with:

- Appropriate Rust version installation based on env vars
- Cache configuration for dependencies

### setup-scarb

Located in `.github/actions/setup-scarb/`, this action sets up the Cairo environment with:

- Appropriate Scarb (Cairo package manager) version installation based on env vars
- Cache configuration for dependencies

### setup-foundry

Located in `.github/actions/setup-foundry/`, this action sets up the Foundry environment with various eth testing tools like anvil.

## Environment Configuration

The environment configuration is centralized in `.github/config/env.yml`. This configuration is loaded automatically into the `GITHUB_ENV` using `.github/actions/load-env`.

## Merge Queue Stubs

This repository makes use of the [Github Merge Queue] to separate test runs into two phases:

1. A first phase during which lighter and more essential tests are run to provide fast feedback.
2. A second phase during which heavier tests and release pipelines are run before a merge into `main`.

Due to the way in which the merge queue works on Github, you will see a lot of stub actions repeated throughout certain tasks: these actions can be recognized by their prefix `task-do-nothing`. For more context as to why this is needed, check out [this comment].

[github merge queue]: https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/configuring-pull-request-merges/managing-a-merge-queue
[this comment]: https://github.com/madara-alliance/madara/pull/633#discussion_r2086165929
