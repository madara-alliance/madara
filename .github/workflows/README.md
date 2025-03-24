# Madara GitHub Actions Workflows

This directory contains the GitHub Actions workflows used for the Madara project. The workflows have been refactored to improve readability, maintainability, and reusability.

## Organization

The workflows are organized into three categories:

1. **Reusable Workflows** - Common components that can be reused across different workflows
2. **Task Workflows** - Single-purpose workflows that perform specific tasks
3. **Orchestration Workflows** - Workflows that coordinate multiple task workflows

## Naming Convention

We've adopted a clear, consistent naming convention:

- `task-*.yml`: Single-purpose workflows that perform specific tasks
- `workflow-*.yml`: Orchestration workflows that coordinate multiple tasks

## Reusable Workflows

- `reusable-rust-setup.yml`: Sets up the Rust environment with configurable toolchain, cache, and dependencies
- `reusable-artifact-cache.yml`: Provides standardized artifact caching functionality

## Task Workflows

- `task-build-madara.yml`: Builds the Madara binary and related components
- `task-test-coverage.yml`: Runs tests and generates code coverage reports
- `task-lint-rust.yml`: Runs Rust-specific linters
- `task-lint-code-style.yml`: Runs linters for code style (Prettier, Markdown, TOML)
- `task-rust-check.yml`: Runs cargo check to verify code compilation

## Orchestration Workflows

- `workflow-pull-request.yml`: Main workflow for pull requests that coordinates all checks and tests

## Manual Release Workflow

The `manual-release.yml` workflow provides a way to manually create releases with automatic versioning, changelog generation, and tagging.

### Features

- Automatic version incrementing (major, minor, patch) or custom version input
- Automatic changelog generation based on commits since the last tag
- Updates all Cargo.toml files with the new version
- Creates a git tag for the release
- Creates a GitHub release with the generated changelog
- Options for pre-release and draft releases

### How to Use

1. Go to the "Actions" tab in your GitHub repository
2. Select the "Manual Release" workflow from the left sidebar
3. Click the "Run workflow" button
4. Configure the release options:
   - **Release type**: Choose between `major`, `minor`, or `patch` version increment
   - **Custom version**: Optionally specify a custom version number (format: X.Y.Z)
   - **Is this a pre-release?**: Mark as pre-release if needed
   - **Create as draft release?**: Create as draft to review before publishing
5. Click "Run workflow" to start the release process

The workflow will:

1. Update version numbers in all Cargo.toml files
2. Generate a changelog from commits since the last tag
3. Update the CHANGELOG.md file with the new version and changes
4. Commit the changes and push to the main branch
5. Create a git tag for the new version
6. Create a GitHub release with the generated changelog

### Permissions Required

This workflow requires write permissions for repository contents and pull requests.
Make sure the `GITHUB_TOKEN` has sufficient permissions or configure a personal access token with the required scopes.

## Variables and Standardization

The refactored workflows standardize:

- Common input variables like `rust-version` and `scarb-version`
- Action versions (using v4 where available)
- Step naming and documentation
- Concurrency settings to prevent redundant workflow runs

## Usage

### Running a Workflow Manually

You can run most workflows manually via the GitHub Actions UI by selecting the workflow and clicking "Run workflow".

### Using Reusable Workflows

Reusable workflows can be incorporated into other workflows like this:

```yaml
jobs:
  my_job:
    uses: ./.github/workflows/reusable-rust-setup.yml
    with:
      rust-version: "1.81"
      install-cairo0: true
```

### Workflow Dependencies

The main pull request workflow (`workflow-pull-request.yml`) orchestrates the following sequence:

1. Update DB version
2. Run linters and Rust checks
3. Build Madara
4. Run tests (JS, E2E, coverage)

## Best Practices

- Use inputs with default values to make workflows configurable
- Document all workflow inputs and outputs
- Use clear, descriptive names for jobs and steps
- Add comments to explain complex or non-obvious operations
- Set appropriate permissions for GitHub tokens
- Use concurrency settings to prevent redundant workflow runs

## Adding a New Workflow

When adding a new workflow:

1. Follow the naming convention
2. Use reusable workflows where appropriate
3. Add comprehensive documentation
4. Standardize variable names to match existing workflows
5. Add the workflow to this README
