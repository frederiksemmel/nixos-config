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

  env.SEMMELTV_FONT_DIR = buildEnv.SEMMELTV_FONT_DIR;

  enterShell = ''
    export SEMMELTV_LIBRARY="''${SEMMELTV_LIBRARY:-$HOME/pcloud/SemmelTV}"
  '';
}
