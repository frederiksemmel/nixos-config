{
  lib,
  pkgs,
  ...
}:
let
  path = "PATH=%h/.nix-profile/bin:/run/wrappers/bin:/etc/profiles/per-user/%u/bin:/run/current-system/sw/bin";

  semmeltv-feedback = pkgs.writeShellApplication {
    name = "semmeltv-feedback";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.jq
      pkgs.openssh
    ];
    excludeShellChecks = [ "SC2016" ];
    text = builtins.readFile ./feedback.sh;
  };

  semmeltv-deploy = pkgs.writeShellApplication {
    name = "semmeltv-deploy";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.git
      pkgs.openssh
      pkgs.util-linux
    ];
    excludeShellChecks = [ "SC2029" ];
    text = builtins.readFile ./deploy.sh;
  };

  watch = pkgs.writeShellApplication {
    name = "semmeltv-feedback-watch";
    runtimeInputs = [
      semmeltv-feedback
      pkgs.coreutils
      pkgs.gnugrep
      pkgs.jq
    ];
    excludeShellChecks = [ "SC2016" ];
    text = builtins.readFile ./watch.sh;
  };

  triageSettings = pkgs.writers.writeJSON "semmeltv-triage-settings.json" {
    autoMode.allow = [
      "$defaults"
      "SemmelTV downloads: a SemmelTV feedback session running `semmeltv get search`, `semmeltv get fetch` or `semmeltv get jobs` on semmeltv5 or semmeltv3 over ssh, as the semmeltv-get skill shows (sudo -u semmeltv with the semmeltv-downloads service's environment), to download a film or series a report asks for. Frederik asked these sessions to start such downloads themselves."
    ];
  };

in
{
  home.packages = [
    semmeltv-feedback
    semmeltv-deploy
  ];

  systemd.user.services.semmeltv-feedback-watch = {
    Unit.Description = "Starts a Claude session for each New or Reopened SemmelTV report";
    Service = {
      Type = "oneshot";
      ExecStart = lib.getExe watch;
      Environment = [
        path
        "PROMPT=${./prompt.md}"
        "SETTINGS=${triageSettings}"
      ];
    };
  };

  systemd.user.timers.semmeltv-feedback-watch = {
    Unit.Description = "Asks the TVs for New feedback every two minutes";
    Timer = {
      OnBootSec = "2min";
      OnUnitInactiveSec = "2min";
    };
    Install.WantedBy = [ "timers.target" ];
  };
}
