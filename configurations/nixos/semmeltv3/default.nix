{ nixos-raspberrypi, ... }:
{
  imports = [
    ../semmeltv
    nixos-raspberrypi.nixosModules.raspberry-pi-3.base
  ];

  networking.hostName = "semmeltv3";
}
