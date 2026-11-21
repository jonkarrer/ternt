{
  description = "tokidoki — a Rust workspace";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    devshell-lib.url = "path:/Users/jokar/devjon/configs/shells/shared";
  };

  outputs = { self, nixpkgs, flake-utils, devshell-lib }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        devShells.default = devshell-lib.mkDevShell {
          inherit pkgs;
          name = "Tokidoki";
          emoji = "⏳";
          # Rust is deliberately not here: it comes from rustup.
          extraPackages = with pkgs; [
            turso     # ships `tursodb`, the local engine shell — not the cloud CLI
            turso-cli # ships `turso`, the Turso Cloud CLI
            jq
            bun       # the app's front end (app/) and the Tauri CLI
          ];
        };
      });
}
