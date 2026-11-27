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
      semmeltv-feedback
      pkgs.coreutils
      pkgs.git
      pkgs.jq
      pkgs.openssh
      pkgs.util-linux
    ];
    excludeShellChecks = [ "SC2029" ];
    text = builtins.readFile ./deploy.sh;
  };

  adapter = pkgs.writeShellApplication {
    name = "feedback-semmeltv";
    runtimeInputs = [
      semmeltv-feedback
      semmeltv-deploy
      pkgs.jq
    ];
    text = builtins.readFile ./adapter.sh;
  };

  deployWatch = pkgs.writeShellApplication {
    name = "semmeltv-deploy-watch";
    runtimeInputs = [
      semmeltv-feedback
      semmeltv-deploy
      pkgs.coreutils
      pkgs.git
      pkgs.gnugrep
      pkgs.jq
    ];
    text = builtins.readFile ./deploy-watch.sh;
  };

  triageSettings = pkgs.writers.writeJSON "semmeltv-triage-settings.json" {
    autoMode.allow = [
      "$defaults"
      "SemmelTV downloads: a SemmelTV feedback session running `semmeltv get search`, `semmeltv get fetch` or `semmeltv get jobs` on semmeltv5 over ssh, as the semmeltv-get skill shows (sudo -u semmeltv with the semmeltv-downloads service's environment), to download a film or series a report asks for. Frederik asked these sessions to start such downloads themselves."
    ];
  };

in
{
  home.packages = [
    semmeltv-feedback
    semmeltv-deploy
  ];

  services.feedback-hub.projects.semmeltv = {
    inherit adapter;
    prompt = ./prompt.md;
    settings = triageSettings;
  };

  systemd.user.services.semmeltv-deploy-watch = {
    Unit.Description = "Tells the TVs what is on main, deploys when a Feedback page's Deploy now asks, and deploys a held deploy once nobody watches";
    Service = {
      Type = "oneshot";
      ExecStart = lib.getExe deployWatch;
      TimeoutStartSec = "1h";
      Environment = [ path ];
    };
  };

  systemd.user.timers.semmeltv-deploy-watch = {
    Unit.Description = "Asks the TVs every 20 seconds whether someone wants a deploy";
    Timer = {
      OnBootSec = "1min";
      OnUnitInactiveSec = "20s";
    };
    Install.WantedBy = [ "timers.target" ];
  };

}
