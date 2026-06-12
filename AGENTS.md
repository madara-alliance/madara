# Agent Guide

This is the root entry point for coding agents working in Madara. Read the
scoped guidance for the area you are touching before editing code:

- `madara/CLAUDE.md` for the Madara node and `mc-*`/`mp-*` crates.
- `orchestrator/CLAUDE.md` for the orchestrator and its client crates.
- `e2e/CLAUDE.md` for full-stack bridge tests.
- `e2e-tests/CLAUDE.md` for orchestrator workflow tests.
- `bootstrapper-v2/CLAUDE.md` for the v2 bootstrapper.
- `UTILITIES_CLAUDE.md` for `cairo/`, `madaraup/`, `scripts/`,
  `tools/`, `test_utils/`, `build-artifacts/`, and `evm/`.

## Repository Shape

Madara is a Rust workspace with multiple binaries and test crates. The default
workspace members build the Madara node and orchestrator; test and utility
crates are included in the workspace but are often expensive to build or run.

Important local entry points:

- `Cargo.toml`: root Rust workspace.
- `Makefile`: local build, formatting, linting, and test commands.
- `rust-toolchain.toml`: pinned Rust toolchain.
- `flake.nix`: Nix development shell with Rust, Scarb, Foundry, Node, Taplo,
  and nextest.
- `.devcontainer/`: VS Code devcontainer setup.
- `.github/CI_README.md`: CI workflow overview.

## Setup

For a reproducible shell, prefer one of:

```bash
nix develop
```

or the VS Code devcontainer described in `.devcontainer/README.md`.

For host setup, install Rust 1.89, Clang/LLVM, OpenSSL, Node.js, Docker,
Taplo, and the test-specific tools needed by the area you are working on.
Some orchestrator and E2E paths also require Python Cairo tooling, Foundry
(`anvil`, `forge`), MongoDB, LocalStack, or Docker images.

## Validation Matrix

Use the narrowest command that covers your change. Avoid starting full E2E
flows unless the change touches those paths.

Fast checks:

```bash
cargo metadata --no-deps --format-version 1
cargo fmt --all -- --check
```

Code style and Rust lint checks:

```bash
make check
```

`make check` creates and uses `sequencer_venv` for Cairo 0 tooling. If you only
need to skip that Python virtualenv setup, use:

```bash
make check NO_CAIRO_SETUP=1
```

That skip does not install missing Rust, LLVM, Node, Taplo, Docker, or artifact
dependencies.

Formatting:

```bash
make fmt
```

Focused Rust tests:

```bash
cargo test -p <package>
cargo nextest run -p <package>
```

Heavy tests:

```bash
make test-orchestrator
make test-orchestrator-e2e
make test-e2e
```

`make test-e2e` is macOS-oriented and requires Docker, Anvil, Forge, Pathfinder
download support, and `.env.e2e`.

## Build Artifacts

There is no `make snos` target. Build scripts normally fetch published
artifacts from `ghcr.io/madara-alliance/artifacts` using
`.artifact-versions.yml`. To regenerate local contract artifacts, run:

```bash
make artifacts
```

This is Docker-heavy and may prompt before replacing existing artifact
directories.

## Safety Notes

- Do not run destructive targets such as `clean-db`, `fclean`, or scripts that
  remove local databases unless the user explicitly asks.
- `make artifacts`, E2E tests, and Docker Compose commands may pull images,
  create containers, or modify generated artifacts.
- `.env.test` and `.env.e2e` are tracked test fixtures. Treat any untracked
  `.env*` files as local user state.
- When the worktree is dirty, inspect diffs and stage only files that belong to
  the requested change.
