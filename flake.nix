{
  description = "tokidoki — a Rust workspace";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    # The shared dev-shell builder; after changing it, `nix flake update devshell`.
    devshell.url = "path:/Users/jokar/devjon/configs/shells/shared";
  };

  outputs = { self, nixpkgs, flake-utils, devshell }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        # The shared shell brings git, just, tmux and zsh. No Rust toolchain here: use your own (e.g. rustup).
        devShells.default = devshell.mkDevShell {
          inherit pkgs;
          name = "Tokidoki";
          emoji = "⏳";
          extraPackages = with pkgs; [
            turso      # `tursodb`: shell for Turso's Rust database engine (local files)
            turso-cli  # `turso`: Turso Cloud (databases, tokens, replicas)
            jq         # read JSON output
          ];
        };
      });
}
