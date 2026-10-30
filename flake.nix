{
  description = "ternt — Arc-style browser and terminal for macOS (Tauri v2 + Svelte 5)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        # Vendored dev-shell builder; see nix/devshell.nix for why it is not
        # the shared one from ~/devjon/configs.
        mkDevShell = import ./nix/devshell.nix;
      in
      {
        devShells.default = mkDevShell {
          inherit pkgs;
          name = "Ternt";
          emoji = "🌐";
          # Rust is deliberately not here: it comes from rustup (the release
          # build needs both Apple targets, see docs/RELEASING.md).
          extraPackages = with pkgs; [ bun ];
          # Release binaries must link only system libraries: a Nix compiler
          # links /nix/store dylibs other Macs don't have, and the app then
          # fails macOS library validation. iosMode puts Xcode's toolchain
          # first; the env below pins cargo's C compiler and linker to it
          # even when the shell is entered from another Nix shell.
          iosMode = true;
          extraEnv = {
            CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER = "/usr/bin/clang";
            CARGO_TARGET_X86_64_APPLE_DARWIN_LINKER = "/usr/bin/clang";
            # Matches bundle.macOS.minimumSystemVersion in tauri.conf.json.
            MACOSX_DEPLOYMENT_TARGET = "14.0";
          };
          # stdenv setup hooks run before the shell hook and overwrite these
          # (the apple-sdk hook points DEVELOPER_DIR at the Nix SDK, which
          # makes xcrun unable to find notarytool and stapler; cc-wrapper sets
          # CC to the bare name "clang"), so extraEnv cannot hold them.
          extraShellHook = ''
            export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
            export CC=/usr/bin/clang CXX=/usr/bin/clang++
          '';
        };
      });
}
