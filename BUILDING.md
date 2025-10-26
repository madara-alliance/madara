# Building Madara

This guide explains how to build Madara with the required Cairo 0 environment.

## Quick Start

The easiest way to build Madara is using the provided Makefile target:

```bash
make build
```

This command will:

1. Automatically set up a Python virtual environment (`sequencer_venv`)
2. Install Cairo 0 and all required dependencies
3. Build Madara in release mode with the Cairo environment activated

## Pre-Push Checks

Before pushing code, run:

```bash
make pre-push
```

This will automatically set up the Cairo environment and run all code quality checks.

## Manual Setup

If you need to set up the Cairo 0 environment manually:

```bash
make setup-cairo
```

This creates the virtual environment and installs `cairo-compile` and dependencies.

## Development Workflow

For development builds, you can use the standard cargo commands with the activated environment:

```bash
. sequencer_venv/bin/activate
cargo build
```

Or for release builds:

```bash
. sequencer_venv/bin/activate
cargo build --release
```

## Troubleshooting

### Cairo Compile Not Found

If you see an error about `cairo-compile` not being found, ensure the virtual environment is set up:

```bash
make setup-cairo
```

### NumPy Build Errors

The setup automatically handles NumPy compatibility issues by using `numpy<2.0` instead of the pinned `numpy==2.0.2` which has build issues on some systems.

### Sequencer Requirements Not Found

If the sequencer requirements file cannot be found at the expected path, the setup will automatically create a minimal requirements file with just `cairo-lang==0.14.0.1` and `numpy<2.0`.

## Available Make Targets

- `make build` - Build Madara (automatically sets up Cairo 0)
- `make setup-cairo` - Set up Cairo 0 environment only
- `make pre-push` - Run all pre-push checks (automatically sets up Cairo 0)
- `make help` - Show all available commands
