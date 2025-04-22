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
          })
        ];

        pkgs = import nixpkgs {
          inherit system overlays;
        };

        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
      in {
        # Export the scarb package
        packages.scarb = pkgs.scarb;
        packages.default = pkgs.scarb;

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
            gnumake
            wget
            git
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

          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          PROTOC = "${pkgs.protobuf}/bin/protoc";
          ROCKSDB_LIB_DIR = "${pkgs.rocksdb}/lib";
        };
      }
    );
}
