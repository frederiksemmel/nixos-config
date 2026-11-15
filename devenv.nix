{ pkgs, ... }:
{
  packages = [
    pkgs.deploy-rs
    pkgs.nh
    pkgs.nixfmt
  ];

  enterShell = ''
    git config core.hooksPath .githooks
  '';
}
