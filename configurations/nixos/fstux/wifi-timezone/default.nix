# Sets the timezone from the Wi-Fi access points iwd sees, located by BeaconDB.
# Falls back to BeaconDB's IP lookup only while Mullvad is disconnected.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  wifi-timezone = pkgs.writers.writePython3Bin "wifi-timezone" {
    libraries = [ pkgs.python3Packages.timezonefinder ];
    flakeIgnore = [ "E501" ];
  } (builtins.readFile ./wifi_timezone.py);
in
{
  systemd.services.wifi-timezone = {
    description = "Set the timezone from nearby Wi-Fi access points";
    wants = [ "network-online.target" ];
    after = [
      "network-online.target"
      "iwd.service"
    ];
    path = [
      pkgs.systemd
      config.services.mullvad-vpn.package
    ];
    serviceConfig = {
      Type = "oneshot";
      ExecStart = lib.getExe wifi-timezone;
    };
  };

  systemd.timers.wifi-timezone = {
    wantedBy = [ "timers.target" ];
    timerConfig = {
      OnCalendar = "*:0/15";
      Persistent = true;
    };
  };
}
