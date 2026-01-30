# Repository Guidelines

## Project Structure & Module Organization

- `madara/` houses the core Rust node implementation and workspace crates.
- `orchestrator/` contains the orchestrator service and related crates.
- `bootstrapper/` and `bootstrapper-v2/` hold bootstrapper tooling and contracts.
- `cairo/` includes Cairo-related artifacts and tests; `build-artifacts/` stores generated build outputs.
- `e2e/`, `e2e-tests/`, and `tests/` contain end-to-end and unit/integration tests.
- `configs/`, `docker/`, `compose.yaml`, and `infra/` cover runtime configuration and deployment assets.

## Build, Test, and Development Commands

- `make help` lists all supported targets with details.
- `make build-madara` builds the Madara binary with Cairo 0 setup.
- `make build-orchestrator` builds the orchestrator binary.
- `make check` runs formatting, clippy, and markdownlint (use `NO_CAIRO_SETUP=1` to skip Cairo setup).
- `make fmt` formats Rust and TOML (`cargo fmt`, `taplo fmt`).
- `make test` runs all tests; `make test-orchestrator` runs unit tests with coverage; `make test-e2e` runs full E2E.
- Direct cargo builds are also used: `cargo build --release` or `cargo build --profile=production`.

## Coding Style & Naming Conventions

- Rust formatting is enforced by `rustfmt` (`rustfmt.toml`) and linted with `clippy` (`clippy.toml`).
- TOML formatting uses `taplo` (`taplo/taplo.toml`).
- Markdown is linted via `markdownlint` (see `package.json`).
- Prefer idiomatic Rust naming (`snake_case` for functions/modules, `CamelCase` for types).

## Testing Guidelines

- Rust tests use `cargo test` and `cargo nextest` (see `make test-orchestrator-e2e` and `make test-orchestrator`).
- Coverage is enforced in orchestrator tests via `cargo llvm-cov nextest`.
- Follow existing test naming conventions in `madara/crates/tests` and `e2e/` suites.

## Commit & Pull Request Guidelines

- Recent commits use a conventional prefix like `chore:`, `fix:`, or `update:`; keep messages short and imperative.
- Include a scope or PR/issue reference when relevant (e.g., `fix(orchestrator): ... (#123)`).
- PRs should include a clear summary, testing evidence (commands + results), and linked issues.
- Add screenshots/log excerpts when changes affect runtime behavior or configs.

## Agent-Specific Notes

- Prefer Makefile targets for repeatable workflows.
- Avoid touching generated artifacts in `build-artifacts/` unless the change requires regeneration.
