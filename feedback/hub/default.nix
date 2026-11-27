{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.feedback-hub;
  path = "PATH=%h/.nix-profile/bin:/run/wrappers/bin:/etc/profiles/per-user/%u/bin:/run/current-system/sw/bin";

  configFile = pkgs.writers.writeJSON "feedback-hub.json" {
    inherit (cfg) repo;
    common = ./common.md;
    projects = lib.mapAttrsToList (name: project: {
      inherit name;
      adapter = lib.getExe project.adapter;
      inherit (project) prompt quiet;
      settings = project.settings;
    }) cfg.projects;
  };

  watch = pkgs.writeShellApplication {
    name = "feedback-hub-watch";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.gnugrep
      pkgs.gnused
      pkgs.jq
    ];
    text = builtins.readFile ./watch.sh;
  };
in
{
  options.services.feedback-hub = {
    enable = lib.mkEnableOption "Claude sessions that take up the comments sent from feedback mode, one session per project for all its open reports";

    repo = lib.mkOption {
      type = lib.types.str;
      default = "${config.home.homeDirectory}/repos/nixos-config";
      description = "The checkout sessions start in, each in a worktree of its own.";
    };

    projects = lib.mkOption {
      default = { };
      description = "Projects whose reports the hub hands to Claude, by name.";
      type = lib.types.attrsOf (
        lib.types.submodule {
          options = {
            adapter = lib.mkOption {
              type = lib.types.package;
              description = "The project's adapter CLI (feedback/README.md), installed for the sessions too.";
            };
            prompt = lib.mkOption {
              type = lib.types.path;
              description = "What a session needs to know about this project: where it lives, how to test and land it.";
            };
            quiet = lib.mkOption {
              type = lib.types.ints.unsigned;
              default = 120;
              description = "Seconds the newest open report waits before a session starts, so batches sent close together go together.";
            };
            settings = lib.mkOption {
              type = lib.types.nullOr lib.types.path;
              default = null;
              description = "Claude settings file for the sessions, such as auto mode allowances.";
            };
          };
        }
      );
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = lib.mapAttrsToList (_: project: project.adapter) cfg.projects;

    systemd.user.services.feedback-hub-watch = {
      Unit.Description = "Starts one Claude session per project for the open feedback reports";
      Service = {
        Type = "oneshot";
        ExecStart = lib.getExe watch;
        Environment = [
          path
          "CONFIG=${configFile}"
        ];
      };
    };

    systemd.user.timers.feedback-hub-watch = {
      Unit.Description = "Looks for open feedback reports every two minutes";
      Timer = {
        OnBootSec = "2min";
        OnUnitInactiveSec = "2min";
      };
      Install.WantedBy = [ "timers.target" ];
    };
  };
}
