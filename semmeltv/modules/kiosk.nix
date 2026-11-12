{
  crane,
  nixpkgs,
  vpnConfinement,
  agenix,
}:
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
  imports = [
    (import ./web.nix { inherit crane nixpkgs; })
    (import ./downloads.nix { inherit nixpkgs vpnConfinement agenix; })
    ./headphones.nix
  ];

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

      edid = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        example = lib.literalExpression "./samsung-edid.bin";
        description = "The EDID of the TV on HDMI0, read from /sys/class/drm/card0-HDMI-A-1/edid while it is connected. When set, HDMI0 counts as connected even with the TV in standby or unplugged, so video, audio and HDMI-CEC stay the same whether the TV is on or not. The vc4 HDMI driver is rebuilt to take HDMI-CEC's address and the audio ELD from this EDID even when the TV does not answer on the DDC lines. Applies after a reboot. Null leaves hotplug to the TV.";
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

    boot.kernelParams = lib.mkIf (cfg.tv.edid != null) [
      "vc4.force_hotplug=1"
      "drm.edid_firmware=HDMI-A-1:edid/semmeltv-tv.bin"
    ];
    hardware.firmware = lib.mkIf (cfg.tv.edid != null) [
      (pkgs.runCommand "semmeltv-tv-edid" { } ''
        install -D -m 444 ${cfg.tv.edid} $out/lib/firmware/edid/semmeltv-tv.bin
      '')
    ];
    boot.extraModulePackages = lib.mkIf (cfg.tv.edid != null) [
      (config.boot.kernelPackages.callPackage ./vc4-edid-override.nix { })
    ];

    services.udev.extraRules = ''
      ACTION=="add", SUBSYSTEM=="drm", KERNEL=="card[0-9]*-HDMI-A-[0-9]*", RUN+="${pkgs.coreutils}/bin/chgrp video /sys%p/status", RUN+="${pkgs.coreutils}/bin/chmod g+w /sys%p/status"
    '';
  };
}
