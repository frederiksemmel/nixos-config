{
  config,
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
    text = builtins.readFile ./watch.sh;
  };

  triageSettings = pkgs.writers.writeJSON "semmeltv-triage-settings.json" {
    autoMode.allow = [
      "$defaults"
      "SemmelTV downloads: a SemmelTV feedback triage run running `semmeltv get search`, `semmeltv get fetch` or `semmeltv get jobs` on semmeltv5 or semmeltv3 over ssh, as the semmeltv-get skill shows (sudo -u semmeltv with the semmeltv-downloads service's environment), to download a film or series a report asks for. Frederik asked these runs to start such downloads themselves."
    ];
  };

  triage = pkgs.writeShellApplication {
    name = "semmeltv-feedback-triage";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.gawk
    ];
    text = builtins.readFile ./triage.sh;
  };
in
{
  home.packages = [
    semmeltv-feedback
    semmeltv-deploy
  ];

  systemd.user.services.semmeltv-feedback-watch = {
    Unit.Description = "Starts a SemmelTV feedback triage run when a TV holds New reports";
    Service = {
      Type = "oneshot";
      ExecStart = lib.getExe watch;
      Environment = [ path ];
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

  systemd.user.services.semmeltv-feedback-triage = {
    Unit = {
      Description = "Triages New SemmelTV feedback with a headless Claude run";
      X-SwitchMethod = "keep-old";
    };
    Service = {
      Type = "exec";
      ExecStart = lib.getExe triage;
      Environment = [
        path
        "CLAUDE=${lib.getExe config.programs.claude-code.finalPackage}"
        "PROMPT=${./prompt.md}"
        "SETTINGS=${triageSettings}"
      ];
      RuntimeMaxSec = "3h";
    };
  };
}
