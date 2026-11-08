{ pkgs, ... }:
{
  packages = [
    pkgs.deploy-rs
    pkgs.nh
    pkgs.nixfmt
  ];
}
