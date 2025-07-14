{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };
  outputs = {
    self,
    nixpkgs,
    rust-overlay,
    flake-utils,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        overlays = [
          (import rust-overlay)
          (final: prev: {
            scarb = final.callPackage (./. + "/tools/scarb.nix") {};
            foundry = final.callPackage (./. + "/tools/foundry.nix") {};
          })
        ];

        pkgs = import nixpkgs {
          inherit system overlays;
        };

        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
      in {
        # Export the scarb and foundry package
        packages.scarb = pkgs.scarb;
        packages.foundry = pkgs.foundry;
        packages.default = with pkgs; [
          scarb
          foundry
        ];

        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            openssl
            pkg-config
            protobuf
            nodejs
            nodePackages.prettier
            nodePackages.markdownlint-cli
            taplo-cli
            alejandra
            yq
            scarb
            foundry
            gnumake
            wget
            git
            cargo-nextest
          ];

          buildInputs = with pkgs;
            [
              rustToolchain
              clang
              rocksdb
            ]
            ++ lib.optionals stdenv.isDarwin [
              darwin.apple_sdk.frameworks.Security
            ];

          shellHook = ''
            # Increase the limit of open file descriptors
            echo "[INFO] Increasing open file descriptor limit to 65535 (ulimit -n)"
            ulimit -n 65535 || echo "[WARN] Failed to set ulimit -n to 65535"

            # --- NPM Global Installation Workaround for Nix Shell ---
            # The starkgate-contracts setup script tries to install npm packages globally.
            # In a Nix environment, global installs to /nix/store fail due to its read-only nature.
            # This workaround redirects npm's global prefix to a local writable directory (`.npm-global`),
            # enabling global-like installs and allowing tools like `npm list -g` to function as expected.
            # Reference: https://nixos.wiki/wiki/Node.js#Install_to_your_home

            export NPM_GLOBAL_PREFIX="$PWD/.npm-global"
            export PATH="$NPM_GLOBAL_PREFIX/bin:$PATH"
            export NODE_PATH="$NPM_GLOBAL_PREFIX/lib/node_modules"

            mkdir -p "$NPM_GLOBAL_PREFIX/bin"
            mkdir -p "$NPM_GLOBAL_PREFIX/lib"

            echo "[INFO] Setting fake global prefix for npm: $NPM_GLOBAL_PREFIX"
            npm config set prefix "$NPM_GLOBAL_PREFIX"

            if ! npm list -g --depth=0 | grep -q ganache@7.9.0; then
              echo "[INFO] Installing ganache@7.9.0 into fake global dir..."
              npm install -g ganache@7.9.0
            fi

            if ! npm list -g --depth=0 | grep -q prettier@2.3.2; then
              echo "[INFO] Installing prettier@2.3.2 into fake global dir..."
              npm install -g prettier@2.3.2 prettier-plugin-solidity@1.0.0-beta.17
            fi
          '';

          env = {
            LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
            ROCKSDB_LIB_DIR = "${pkgs.rocksdb}/lib";
            PROTOC = "${pkgs.protobuf}/bin/protoc";
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
              pkgs.gcc.cc.lib
              pkgs.openssl
              pkgs.rocksdb
            ];
          };
        };
      }
    );
}
