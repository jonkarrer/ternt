{
  description = "Ternt: an Arc-style browser and terminal (Tauri v2 + Svelte 5), macOS";

  # Provides Bun. Rust is NOT provided: use rustup. C compiling and linking
  # deliberately go to Apple's toolchain (Xcode / Command Line Tools), not a
  # Nix one: a Nix cc links /nix/store libraries into the app, which then
  # can't run on other Macs and fails macOS library validation (see
  # docs/RELEASING.md).
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

  outputs =
    { nixpkgs, ... }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
      ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      devShells = forAll (pkgs: {
        # NoCC: no Nix compiler, linker or SDK in the shell.
        default = pkgs.mkShellNoCC {
          packages = [ pkgs.bun ]; # package manager, scripts, tests

          shellHook = ''
            # Entered from another Nix shell? Drop its compiler setup so cargo
            # uses Apple's clang and the macOS SDK from Xcode.
            unset CC CXX LD AR NM RANLIB STRIP OBJCOPY SIZE STRINGS \
              DEVELOPER_DIR SDKROOT NIX_APPLE_SDK_VERSION \
              NIX_CC NIX_BINTOOLS NIX_CFLAGS_COMPILE NIX_LDFLAGS NIX_CFLAGS_LINK \
              NIX_HARDENING_ENABLE NIX_ENFORCE_NO_NATIVE NIX_DONT_SET_RPATH \
              NIX_DONT_SET_RPATH_FOR_BUILD NIX_NO_SELF_RPATH NIX_IGNORE_LD_THROUGH_GCC
            unset "''${!NIX_CC_WRAPPER_TARGET_@}" "''${!NIX_BINTOOLS_WRAPPER_TARGET_@}" 2>/dev/null
            # ...and its compiler, linker, SDK and xcrun from PATH (they'd
            # shadow /usr/bin; Nix's xcrun can't run notarytool or stapler).
            PATH=$(printf '%s' "$PATH" | tr ':' '\n' \
              | grep -vE '^/nix/store/[a-z0-9]+-(clang|cctools|xcbuild|ld64|apple-sdk|libcxx|binutils|llvm|compiler-rt)' \
              | paste -sd: -)
            export PATH
            export CC=/usr/bin/clang CXX=/usr/bin/clang++
            export CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER=/usr/bin/clang
            export CARGO_TARGET_X86_64_APPLE_DARWIN_LINKER=/usr/bin/clang
            # Matches bundle.macOS.minimumSystemVersion in tauri.conf.json.
            export MACOSX_DEPLOYMENT_TARGET=14.0

            if ! /usr/bin/xcrun --find SetFile >/dev/null 2>&1; then
              echo "ternt: Apple's developer tools are missing: xcode-select --install" >&2
            fi
            if ! command -v rustup >/dev/null; then
              echo "ternt: Rust isn't provided by this shell; install rustup (https://rustup.rs)" >&2
            fi
          '';
        };
      });
    };
}
