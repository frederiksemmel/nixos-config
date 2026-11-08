{
  flake,
  nixos-raspberrypi,
  pkgs,
  ...
}:
let
  ffmpeg = pkgs.callPackage "${nixos-raspberrypi}/pkgs/ffmpeg_8-rpi.nix" {
    ffmpeg = pkgs.ffmpeg_8;
  };
in
{
  imports = [
    ../modules/pi-host.nix
    flake.self.nixosModules.semmeltv
  ];

  services.semmeltv = {
    enable = true;
    adminUser = "frederik";
    mpv = pkgs.mpv.override {
      mpv-unwrapped = pkgs.mpv-unwrapped.override { inherit ffmpeg; };
    };
  };
}
