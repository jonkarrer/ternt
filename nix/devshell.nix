# Ternt's dev-shell builder: a vendored copy of the org's shared mkDevShell
# (~/devjon/configs/shells/shared/devshell.nix), kept in the repo on purpose.
#
# The shared one is a `path:` flake input, and Nix locks a path input by the
# hash of the whole directory. Every machine then needs a byte-identical
# working copy of that folder (uncommitted edits included) or `nix develop`
# refuses to start, and each re-lock breaks the other machine. A release build
# should not depend on the state of a dotfiles checkout, so ternt carries its
# own copy. Port shared-shell fixes here by hand when they matter.
#
# Differences from the shared builder:
#   - no nvim import (it resolves the config through `builtins.getEnv "HOME"`,
#     which needs `--impure`; this shell evaluates purely)
#   - the unused uvPython / sourceDotenv options are dropped
#
# Args:
#   pkgs           - the project's nixpkgs instance
#   name, emoji    - shell banner and $SHELL_NAME (shown in RPROMPT)
#   extraPackages  - project buildInputs on top of git/just/doppler/tmux/zsh
#   iosMode        - Xcode-toolchain workaround: DEVELOPER_DIR, /usr/bin ahead
#                    of PATH, and no SDKROOT/NIX_CFLAGS_COMPILE/NIX_LDFLAGS so
#                    Xcode's clang wins over nix's
#   extraEnv       - extra env attrs merged into mkShell's env
#   extraShellHook - raw shell appended after the standard hook, before the
#                    zsh handoff
{ pkgs
, name
, emoji ? ""
, extraPackages ? [ ]
, iosMode ? false
, extraEnv ? { }
, extraShellHook ? ""
}:

let
  banner = if emoji != "" then "${name} Shell ${emoji}" else "${name} Shell";
  shellName = if emoji != "" then "${name} ${emoji}" else name;

  # A generated ZDOTDIR that chains into the real $HOME/.zshrc, so the shell
  # follows whichever machine it runs on. The machine prompt themes under
  # ~/devjon/configs are optional: every lookup is guarded, and without them
  # the shell just gets the plain zsh prompt.
  zdotdir = pkgs.runCommand "devshell-zdotdir-${pkgs.lib.strings.sanitizeDerivationName name}" { } ''
    mkdir -p "$out"
    cat > "$out/.zshrc" <<'EOF'
    # ZDOTDIR="X" source ... is a POSIX special-builtin assignment: it
    # persists after the command, so it must be restored explicitly.
    _devshell_zdotdir="$ZDOTDIR"
    if [ -f "$HOME/.zshrc" ]; then
      ZDOTDIR="$HOME" source "$HOME/.zshrc"
    fi
    export ZDOTDIR="$_devshell_zdotdir"
    unset _devshell_zdotdir

    # macOS /etc/zshrc defaults HISTFILE to $ZDOTDIR/.zsh_history, which is
    # this read-only store path.
    export HISTFILE="$HOME/.zsh_history"

    _host="''${HOST%%.*}"
    for _pd in "$HOME/devjon/configs/shells/$_host.local" "$HOME/devjon/configs/shells/$_host" "$HOME/devjon/configs/shells/work.local"; do
      if [ -f "$_pd/prompt.zshrc" ]; then
        source "$_pd/prompt.zshrc"
        break
      fi
    done
    unset _host _pd

    [ -f "$ZSH_SYNTAX_HIGHLIGHTING/zsh-syntax-highlighting.zsh" ] && \
      source "$ZSH_SYNTAX_HIGHLIGHTING/zsh-syntax-highlighting.zsh"
    [ -f "$ZSH_AUTOSUGGESTIONS/zsh-autosuggestions.zsh" ] && \
      source "$ZSH_AUTOSUGGESTIONS/zsh-autosuggestions.zsh"
    EOF
  '';
in
pkgs.mkShell {
  buildInputs = with pkgs; [
    git
    just
    doppler
    tmux
    zsh
    zsh-syntax-highlighting
    zsh-autosuggestions
  ] ++ extraPackages;

  env = {
    SHELL_NAME = shellName;
    ZSH_SYNTAX_HIGHLIGHTING = "${pkgs.zsh-syntax-highlighting}/share/zsh-syntax-highlighting";
    ZSH_AUTOSUGGESTIONS = "${pkgs.zsh-autosuggestions}/share/zsh-autosuggestions";
  } // pkgs.lib.optionalAttrs iosMode {
    DEVELOPER_DIR = "/Applications/Xcode.app/Contents/Developer";
  } // extraEnv;

  shellHook = ''
    echo "${banner}"
  ''
  + pkgs.lib.optionalString iosMode ''
    export PATH="/usr/bin:/usr/sbin:$PATH"
    unset SDKROOT
    unset NIX_CFLAGS_COMPILE
    unset NIX_LDFLAGS
  ''
  + extraShellHook
  + ''

    if [ -z "$DEVSHELL_ACTIVE" ]; then
      # nix develop unsets this only after shellHook returns, and the exec
      # below means it never does. Left set, the cc-wrapper drops every -I
      # outside /nix/store and cargo builds of C crates (ring, onig_sys) fail.
      unset NIX_ENFORCE_PURITY
      export DEVSHELL_ACTIVE=1
      export ZDOTDIR="${zdotdir}"
      exec zsh -i
    fi
  '';
}
