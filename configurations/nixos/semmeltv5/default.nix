{ nixos-raspberrypi, ... }:
{
  imports = with nixos-raspberrypi.nixosModules; [
    ../semmeltv
    ./artwork.nix
    ./mount.nix
    raspberry-pi-5.base
    raspberry-pi-5.display-vc4
  ];

  networking.hostName = "semmeltv5";

  semmeltv.offlineCacheGiB = 32;
}
