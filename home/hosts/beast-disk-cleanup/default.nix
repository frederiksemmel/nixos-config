{
  config,
  lib,
  pkgs,
  ...
}:
let
  disk-fill = pkgs.writeShellApplication {
    name = "disk-fill";
    runtimeInputs = [ pkgs.util-linux ];
    text = ''
      fs=/sys/fs/btrfs/$(findmnt -no UUID /)
      size=0
      for f in "$fs"/devices/*/size; do
        size=$((size + $(< "$f") * 512))
      done
      used=0
      for t in data metadata system; do
        used=$((used + $(< "$fs/allocation/$t/disk_used")))
      done
      echo "$((used * 100 / size))%"
    '';
  };

  disk-cleanup = pkgs.writers.writePython3Bin "disk-cleanup" {
    flakeIgnore = [
      "E501"
      "W503"
    ];
  } (builtins.readFile ./disk_cleanup.py);
in
{
  age.secrets.beast_linear_api_key.file = ../../../secrets/beast_linear_api_key.age;

  home.packages = [ disk-fill ];

  systemd.user.services.disk-cleanup = {
    Unit = {
      Description = "Frees disk space with a Claude session once / is 90% full";
      After = [ "agenix.service" ];
    };
    Service = {
      Type = "oneshot";
      ExecStart = lib.getExe disk-cleanup;
      Environment = [
        "PATH=${disk-fill}/bin:/run/wrappers/bin:%h/.nix-profile/bin:/run/current-system/sw/bin"
        "DUA=${lib.getExe pkgs.dua}"
        "DISK_FILL=${lib.getExe disk-fill}"
        "CLAUDE=${lib.getExe config.programs.claude-code.finalPackage}"
        "PROMPT=${./prompt.md}"
        "LINEAR_KEY_FILE=${config.age.secrets.beast_linear_api_key.path}"
      ];
      RuntimeMaxSec = "1h";
    };
  };

  systemd.user.timers.disk-cleanup = {
    Unit.Description = "Checks every 15 minutes whether / needs a cleanup or a Linear answer came in";
    Timer.OnCalendar = "*:0/15";
    Install.WantedBy = [ "timers.target" ];
  };
}
