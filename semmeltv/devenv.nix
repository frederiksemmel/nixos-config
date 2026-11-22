{ pkgs, ... }:
let
  buildEnv = import ./app/build-env.nix pkgs;
in
{
  languages.rust = {
    enable = true;
    rustflags = buildEnv.RUSTFLAGS;
  };

  packages = with pkgs; [
    bacon
    fontconfig
    mpv
    pkg-config
    slint-lsp
    wlr-randr
  ];

  env = {
    SEMMELTV_FONT_DIR = buildEnv.SEMMELTV_FONT_DIR;
  };

  scripts.semmeltv-stats = {
    description = "Who used SemmelTV and what they watched: semmeltv-stats [DAYS] [HOST], 7 days on semmeltv5 by default";
    exec = ''ssh "''${2:-semmeltv5}" semmeltv-stats "''${1:-7}"'';
  };

  tasks."semmeltv:warm-library" = {
    description = "List the pCloud library folder in one call so the app loads in seconds";
    before = [ "devenv:enterShell" ];
    exec = ''
      socket="$XDG_RUNTIME_DIR/rclone-pcloud.sock"
      library="''${SEMMELTV_LIBRARY:-$HOME/pcloud/SemmelTV}"
      case "$library" in
        "$HOME/pcloud/"*) ;;
        *) exit 0 ;;
      esac
      if [ ! -S "$socket" ]; then
        echo "no rclone rc socket at $socket; the library loads folder by folder"
        exit 0
      fi
      ${pkgs.lib.getExe pkgs.rclone} rc --unix-socket "$socket" vfs/refresh \
        dir="''${library#"$HOME/pcloud/"}" recursive=true > /dev/null \
        || echo "warming $library failed; the library loads folder by folder"
    '';
  };

  enterShell = ''
    export SEMMELTV_LIBRARY="''${SEMMELTV_LIBRARY:-$HOME/pcloud/SemmelTV}"
  '';
}
