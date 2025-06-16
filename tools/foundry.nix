{pkgs}: let
  # Define versions and their hashes
  versions = {
    "1.2.3" = {
      hashes = {
        "darwin_arm64" = "a3f3f1417a7a02a16942e9fc86418d0121c2d904c113feb1b26c9d69dc6f287d";
        "darwin_amd64" = "e3e2b425c7e1b8c853ed454276b20ff500fa82d65cc95acad225b4b7063add4a";
        "linux_arm64" = "70612fd1da9df3a8b35484214e4f2b37ba1d6c41150849490642d7a053c31eaa";
        "linux_amd64" = "8202f38f1635c2793b2d1a4fe443ae6f7315190dc6eed219d7969a40ab78a286";
        "alpine_arm64" = "c40f4786bde394514c1074390a477b7de9574e7c95561ca4b03b275de3f13c17";
        "alpine_amd64" = "015180571c79ce7664717768db2f35e878e6ad98a883750547897420c3a2c461";
        "win32_amd64" = "sha256:9fead4afd459f356b6de4a904daf394db2e7192cb64548db657fca120145ab1f";
      };
    };
  };

  # System mapping
  systemMap = {
    "aarch64-darwin" = "darwin_arm64";
    "x86_64-darwin" = "darwin_amd64";

    # NOTE:
    # On NixOS, using the generic glibc-based binaries (e.g. linux_amd64) will fail at runtime
    # due to missing dynamic linker and runtime libraries not available in the Nix sandbox.
    # To avoid patchelf or wrapping with custom FHS environments, we use musl-based
    # statically-linked binaries instead, which work out-of-the-box in Nix environments.

    # "aarch64-linux" = "linux_arm64";
    # "x86_64-linux" = "linux_amd64";
    "aarch64-linux" = "alpine_arm64";
    "x86_64-linux" = "alpine_amd64";
    "aarch64-linux-musl" = "alpine_arm64";
    "x86_64-linux-musl" = "alpine_amd64";
    "x86_64-windows" = "win32_amd64";
  };

  # Get current system platform
  currentSystem = pkgs.stdenv.hostPlatform.system;
  platform = systemMap.${currentSystem} or (throw "Unsupported system: ${currentSystem}");

  # Get extension based on platform
  extension =
    if platform == "x86_64-pc-windows-msvc"
    then "zip"
    else "tar.gz";

  # Get version data
  version = "1.2.3";
  versionData = versions.${version} or (throw "Version ${version} not found");
  sha256 = versionData.hashes.${platform} or (throw "Hash not found for ${platform} in version ${version}");
in
  pkgs.stdenv.mkDerivation {
    pname = "foundry";
    inherit version;

    src = pkgs.fetchurl {
      url = "https://github.com/foundry-rs/foundry/releases/download/v${version}/foundry_v${version}_${platform}.${extension}";
      inherit sha256;
    };

    dontUnpack = true;

    # Add meta information
    meta = with pkgs.lib; {
      description = "Foundry is a blazing fast, portable and modular toolkit for Ethereum application development written in Rust.";
      homepage = "https://github.com/foundry-rs/foundry";
      license = licenses.mit;
      platforms = builtins.attrNames systemMap;
      maintainers = with maintainers; [];
    };

    # Installation phase
    installPhase = ''
      mkdir -p $out/bin

      ${
        if extension == "zip"
        then ''
          unzip $src
        ''
        else ''
          tar xf $src
        ''
      }

      echo "[DEBUG] Extracted files:"
      ls -lh

      # Verify installation of all required binaries
      required_bins=(
        anvil
        cast
        chisel
        forge
      )

      for bin in "''${required_bins[@]}"; do
        if [ -f "$bin" ]; then
          # Make binary executable
          chmod +x "$bin"
          # Install binarie
          mv "$bin" $out/bin/
        else
          echo "Error: $bin not found in archive"
          exit 1
        fi
      done
    '';

    # Add required build inputs
    nativeBuildInputs = with pkgs;
      [
        gnutar
        gzip
      ]
      ++ pkgs.lib.optionals (extension == "zip") [
        unzip
      ];
  }
