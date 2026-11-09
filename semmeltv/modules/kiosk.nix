{ crane, nixpkgs }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.semmeltv;
  blankCursor = pkgs.callPackage ../app/blank-cursor.nix { };
in
{
  imports = [ (import ./web.nix { inherit crane nixpkgs; }) ];

  options.services.semmeltv = {
    enable = lib.mkEnableOption "the SemmelTV kiosk, the app full screen in cage on tty1";

    mpv = lib.mkPackageOption pkgs "mpv" { };

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.callPackage ../app {
        inherit crane;
        inherit (cfg) mpv;
      };
      defaultText = lib.literalMD "the app from `semmeltv/app`, playing with `services.semmeltv.mpv`";
      description = "The SemmelTV app.";
    };

    stateDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/semmeltv";
      description = "Home of the `semmeltv` user, holding `library` and `cache`.";
    };

    adminUser = lib.mkOption {
      type = lib.types.str;
      example = "frederik";
      description = "User allowed to restart the kiosk without a password.";
    };

    tv = {
      kind = lib.mkOption {
        type = lib.types.nullOr (
          lib.types.enum [
            "generic"
            "samsung"
          ]
        );
        default = null;
        description = "Which TV SemmelTV is plugged into, choosing how it is paired, or null to tell from the manufacturer its renderer reports.";
      };

      renderer = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "http://192.168.1.148:9197/dmr";
        description = "The TV's UPnP MediaRenderer description, whose RenderingControl sets the TV's volume, or null for no volume control. Power and input go over HDMI-CEC either way.";
      };
    };
  };

  config = lib.mkIf cfg.enable {
    users.users.semmeltv = {
      isNormalUser = true;
      home = cfg.stateDir;
      createHome = false;
      extraGroups = [
        "video"
        "render"
        "input"
        "audio"
      ];
    };

    systemd.tmpfiles.settings.semmeltv =
      lib.genAttrs
        [
          cfg.stateDir
          "${cfg.stateDir}/cache"
        ]
        (_: {
          d = {
            user = "semmeltv";
            group = "users";
            mode = "2775";
          };
        });

    security.polkit.extraConfig = ''
      polkit.addRule(function(action, subject) {
        if (action.id == "org.freedesktop.systemd1.manage-units"
            && action.lookup("unit") == "cage-tty1.service"
            && subject.user == "${cfg.adminUser}") {
          return polkit.Result.YES;
        }
      });
    '';

    services.cage = {
      enable = true;
      user = "semmeltv";
      program = lib.getExe cfg.package;
      environment = {
        SEMMELTV_LIBRARY = "${cfg.stateDir}/library";
        SEMMELTV_CACHE = "${cfg.stateDir}/cache";
        FONTCONFIG_FILE = "${pkgs.makeFontsConf {
          fontDirectories = [
            pkgs.dejavu_fonts
            pkgs.inter
          ];
        }}";
        XCURSOR_THEME = "blank";
        XCURSOR_PATH = "${blankCursor}/share/icons";
      }
      // lib.optionalAttrs (cfg.tv.kind != null) { SEMMELTV_TV_KIND = cfg.tv.kind; }
      // lib.optionalAttrs (cfg.tv.renderer != null) { SEMMELTV_TV_RENDERER = cfg.tv.renderer; };
    };

    systemd.services.cage-tty1 = {
      after = [ "systemd-tmpfiles-setup.service" ];
      restartIfChanged = lib.mkForce true;
      serviceConfig = {
        Restart = "always";
        RestartSec = 2;
      };
    };

    systemd.services."getty@tty1".enable = false;
    systemd.services."autovt@tty1".enable = false;
  };
}
