{ nixos-raspberrypi, ... }:
{
  imports = with nixos-raspberrypi.nixosModules; [
    ../semmeltv
    raspberry-pi-5.base
    raspberry-pi-5.display-vc4
  ];

  networking.hostName = "semmeltv5";
}
