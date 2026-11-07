{ nixos-raspberrypi, ... }:
{
  imports = [
    ../semmeltv
    nixos-raspberrypi.nixosModules.raspberry-pi-3.base
  ];

  networking.hostName = "semmeltv3";

  systemd.tmpfiles.settings.semmeltv."/var/lib/semmeltv/library".d = {
    user = "semmeltv";
    group = "users";
    mode = "0755";
  };
}
