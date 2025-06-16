{pkgs}: let
  # Define versions and their hashes
  versions = {
    "2.8.2" = {
      hashes = {
        "aarch64-apple-darwin" = "59d041fca5404b0c60e0e1ff22e9d1929b118c9914ba80c012cac7c372168e2c";
        "x86_64-apple-darwin" = "209979076e65ba3e0d53506cab826516a647f44c0d77fc4be221fea5bcb6607e";
        "aarch64-unknown-linux-gnu" = "c4e0066b00af0e644585f4c27ecea55abed732679b7e8654876303bae982e2f6";
        "x86_64-unknown-linux-gnu" = "2033173c4b8e72fd4bf1779baca6d5348d4f0b53327a14742cb2f65c2f0e892c";
        "aarch64-unknown-linux-musl" = "35bb6f035691fe71faf5836b8e90d05671f4407a14758e31658fd5c7e97d2a08";
        "x86_64-unknown-linux-musl" = "b8c99d5f917880f4416e443abfaeab9eb2af7b8d34d34a84e5c913b0e8149d22";
        "x86_64-pc-windows-msvc" = "ef9161f4305c4ac3262810fb0e888835a00204b98dd185ec07d2cc4e3e9c0330";
      };
    };
  };

  # System mapping
  systemMap = {
    "aarch64-darwin" = "aarch64-apple-darwin";
    "x86_64-darwin" = "x86_64-apple-darwin";

    # NOTE:
    # On NixOS, using the generic glibc-based binaries (e.g. linux_amd64) will fail at runtime
    # due to missing dynamic linker and runtime libraries not available in the Nix sandbox.
    # To avoid patchelf or wrapping with custom FHS environments, we use musl-based
    # statically-linked binaries instead, which work out-of-the-box in Nix environments.

    # "aarch64-linux" = "aarch64-unknown-linux-gnu";
    # "x86_64-linux" = "x86_64-unknown-linux-gnu";
    "aarch64-linux" = "aarch64-unknown-linux-musl";
    "x86_64-linux" = "x86_64-unknown-linux-musl";
    "aarch64-linux-musl" = "aarch64-unknown-linux-musl";
    "x86_64-linux-musl" = "x86_64-unknown-linux-musl";
    "x86_64-windows" = "x86_64-pc-windows-msvc";
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
  version = "2.8.2";
  versionData = versions.${version} or (throw "Version ${version} not found");
  sha256 = versionData.hashes.${platform} or (throw "Hash not found for ${platform} in version ${version}");
in
  pkgs.stdenv.mkDerivation {
    pname = "scarb";
    inherit version;

    src = pkgs.fetchurl {
      url = "https://github.com/software-mansion/scarb/releases/download/v${version}/scarb-v${version}-${platform}.${extension}";
      inherit sha256;
    };

    # Add meta information
    meta = with pkgs.lib; {
      description = "Scarb package manager for Cairo/Starknet development";
      homepage = "https://github.com/software-mansion/scarb";
      license = licenses.mit;
      platforms = builtins.attrNames systemMap;
      maintainers = with maintainers; [];
    };

    # Installation phase
    installPhase = ''
      mkdir -p $out/{bin,doc}

      ${
        if extension == "zip"
        then ''
          unzip $src
          cd scarb-v${version}-${platform}
        ''
        else ''
          tar xf $src
          cd scarb-v${version}-${platform}
        ''
      }

      # Install binaries
      mv bin/* $out/bin/

      # Install documentation
      mv doc/* $out/doc/

      # Make all binaries executable
      chmod +x $out/bin/*

      # Verify installation of all required binaries
      required_bins=(
        scarb
        scarb-cairo-language-server
        scarb-cairo-run
        scarb-cairo-test
        scarb-doc
        scarb-snforge-test-collector
      )

      for bin in "''${required_bins[@]}"; do
        if [ ! -x "$out/bin/$bin" ]; then
          echo "Error: Required binary $bin not found or not executable"
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
