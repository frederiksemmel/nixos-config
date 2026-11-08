{
  config,
  flake,
  lib,
  pkgs,
  ...
}:
let
  semmeltv = pkgs.callPackage ../../app { inherit (flake.inputs) crane; };
  inherit (config.services.semmeltv) stateDir;
in
{
  imports = [ flake.inputs.agenix.nixosModules.default ];

  age.secrets.tmdb_token = {
    file = ../../../secrets/tmdb_token.age;
    owner = "semmeltv";
  };

  systemd.services.semmeltv-artwork = {
    description = "Download SemmelTV artwork from TMDB";
    after = [
      "network-online.target"
      "semmeltv-mount.service"
    ];
    wants = [ "network-online.target" ];
    environment = {
      SEMMELTV_LIBRARY = "${stateDir}/library";
      SEMMELTV_CACHE = "${stateDir}/cache";
      TMDB_TOKEN_FILE = config.age.secrets.tmdb_token.path;
    };
    serviceConfig = {
      Type = "oneshot";
      User = "semmeltv";
      ExecStart = "${lib.getExe semmeltv} artwork";
    };
  };

  systemd.timers.semmeltv-artwork = {
    wantedBy = [ "timers.target" ];
    timerConfig = {
      OnCalendar = "hourly";
      OnBootSec = "5min";
      Persistent = true;
    };
  };
}
