{ config, nixos-raspberrypi, ... }:
{
  imports = [
    ../common.nix
    nixos-raspberrypi.nixosModules.raspberry-pi-3.base
  ];

  networking.hostName = "semmeltv3";

  systemd.tmpfiles.settings.semmeltv."${config.services.semmeltv.stateDir}/library".d = {
    user = "semmeltv";
    group = "users";
    mode = "0755";
  };
}
