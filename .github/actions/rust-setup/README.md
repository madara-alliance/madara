# Rust Setup Composite Action

This composite action sets up the Rust environment with configurable toolchain, cache, and dependencies.

## Inputs

| Name | Description | Required | Default |
|------|-------------|----------|---------|
| `rust-version` | Rust toolchain version to use | Yes | `1.81` |
| `cache-key` | Custom cache key for rust-cache | No | `cache` |
| `install-mold` | Whether to install mold linker | No | `true` |
| `install-scarb` | Whether to install Scarb | No | `true` |
| `scarb-version` | Scarb version to install | No | `2.8.2` |
| `install-foundry` | Whether to install Foundry | No | `false` |
| `install-cairo0` | Whether to install Cairo 0 | No | `false` |
| `build-snos` | Whether to build SNOS files | No | `false` |

## Usage

```yaml
steps:
  - name: Checkout repository
    uses: actions/checkout@v4

  - name: Setup Rust Environment
    uses: ./.github/actions/rust-setup
    with:
      rust-version: '1.81'
      scarb-version: '2.8.2'
      install-scarb: 'true'
      install-mold: 'true'
      cache-key: 'my-cache-key'
```

## Features

This action:

1. Sets up the specified Rust toolchain
2. Configures Rust caching for faster builds
3. Installs system dependencies
4. Optionally installs the mold linker for faster linking
5. Optionally installs Scarb with the specified version
6. Optionally installs Foundry
7. Optionally installs Cairo 0
8. Optionally builds SNOS files