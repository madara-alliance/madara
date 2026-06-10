# Taplo

[Taplo](https://github.com/tamasfe/taplo) is a TOML validator and formatter. It
provides a command-line interface (CLI) for working with TOML files.

## Installation

Taplo is available in the repository Nix shell:

```bash
nix develop
```

You can also install Taplo using Cargo, Yarn, or NPM.

### Cargo

```bash
cargo install taplo-cli --locked
```

### Yarn

```bash
yarn global add @taplo/cli
```

### NPM

```bash
npm install -g @taplo/cli
```

### Usage

To check your TOML files for formatting issues, use the following command:

```bash
taplo fmt --config ./taplo/taplo.toml --check
```

To format all TOML files in your project, use the following command:

```bash
taplo fmt --config ./taplo/taplo.toml
```

The repository `make check` target runs Taplo in check mode, and `make fmt`
runs Taplo formatting.

### Configuration

Taplo allows you to customize the formatting rules by adding configuration
options. You can find the available options and how to use them in the
[Taplo configuration documentation](https://taplo.tamasfe.dev/configuration/formatter-options.html).
