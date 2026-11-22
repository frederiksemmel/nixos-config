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
  tvSocket = "/run/semmeltv/tv.sock";
  tvReads = [
    "SEMMELTV_LIBRARY"
    "SEMMELTV_META"
  ];
  blankCursor = pkgs.callPackage ../app/blank-cursor.nix { };
  stats = pkgs.writeShellApplication {
    name = "semmeltv-stats";
    text = ''
      exec sudo -u semmeltv env SEMMELTV_DB=${cfg.stateDir}/semmeltv.db ${lib.getExe cfg.package} stats "$@"
    '';
  };
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

    decoder = lib.mkOption {
      type = lib.types.enum [
        "drm"
        "v4l2m2m"
      ];
      default = "drm";
      description = "How mpv decodes video. `drm` uses the stateless V4L2 decoders (the Pi 5's HEVC) and draws the frames with the GPU. `v4l2m2m` uses the stateful V4L2 decoder (the Pi 3's H.264) and hands its frames to cage untouched, because the Pi 3's GPU can't draw 1080p video at full frame rate.";
    };

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
      description = "Home of the `semmeltv` user, holding `library`, `cache`, the database `semmeltv.db` and the feedback reports in `feedback`.";
    };

    split = lib.mkEnableOption ''
      two processes: the core (`semmeltv core`: the API, the library, the
      TV's remote and what plays) as `semmeltv.service`, and the tv
      (`semmeltv tv`: the menu and mpv) under cage, talking over a Unix
      socket. Each has a watchdog, and a tv that freezes or crashes no
      longer takes the API with it. Off, the app runs as one process under
      cage'';

    coreEnvironment = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      default = { };
      example = {
        SEMMELTV_HEADPHONES = "bluetooth";
      };
      description = "Environment of the SemmelTV core: the API, the database, the library, downloads, headphones and TV control. With `split` it is `semmeltv.service`'s; without, the app under cage gets it on top of `services.cage.environment`.";
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

      decodes = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        example = [ "hevc" ];
        description = "Video codecs, as ffprobe names them, that the TV set plays on its own player instead of this Pi. When set, H.264 beyond the Pis' decoders (10-bit, or above 1080p) goes to the TV too. The Pi serves such a title over HTTP and steers the renderer's AVTransport; audio the TV can't play (DTS, TrueHD) is converted to AC-3 on the way. Needs `tv.renderer`.";
      };

      port = lib.mkOption {
        type = lib.types.port;
        default = 8097;
        description = "The TCP port this Pi serves titles to the TV set on, opened in the firewall when `tv.decodes` is set.";
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
        "systemd-journal"
      ];
    };

    systemd.tmpfiles.settings.semmeltv =
      lib.genAttrs
        [
          cfg.stateDir
          "${cfg.stateDir}/cache"
          "${cfg.stateDir}/feedback"
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
            && (action.lookup("unit") == "cage-tty1.service" || action.lookup("unit") == "semmeltv.service")
            && subject.user == "${cfg.adminUser}") {
          return polkit.Result.YES;
        }
      });
    '';

    services.semmeltv.coreEnvironment = {
      SEMMELTV_LIBRARY = "${cfg.stateDir}/library";
      SEMMELTV_CACHE = "${cfg.stateDir}/cache";
      SEMMELTV_STREAM_DIR = "/dev/shm/semmeltv-stream";
      SEMMELTV_DB = "${cfg.stateDir}/semmeltv.db";
      SEMMELTV_FEEDBACK = "${cfg.stateDir}/feedback";
      SEMMELTV_GRIM = lib.getExe pkgs.grim;
    }
    // lib.optionalAttrs (cfg.tv.kind != null) { SEMMELTV_TV_KIND = cfg.tv.kind; }
    // lib.optionalAttrs (cfg.tv.renderer != null) { SEMMELTV_TV_RENDERER = cfg.tv.renderer; }
    // lib.optionalAttrs (cfg.tv.decodes != [ ]) {
      SEMMELTV_TV_DECODES = lib.concatStringsSep "," cfg.tv.decodes;
      SEMMELTV_TV_PORT = toString cfg.tv.port;
    };

    services.cage = {
      enable = true;
      user = "semmeltv";
      package = pkgs.cage.override {
        wlroots_0_20 = pkgs.wlroots_0_20.override { enableXWayland = false; };
      };
      program = if cfg.split then "${lib.getExe cfg.package} tv" else lib.getExe cfg.package;
      environment =
        (
          if cfg.split then
            lib.filterAttrs (name: _: lib.elem name tvReads) cfg.coreEnvironment
            // {
              SEMMELTV_TV_SOCKET = tvSocket;
            }
          else
            cfg.coreEnvironment
        )
        // {
          FONTCONFIG_FILE = "${pkgs.makeFontsConf {
            fontDirectories = [
              pkgs.dejavu_fonts
              pkgs.inter
            ];
          }}";
          XCURSOR_THEME = "blank";
          WLR_LIBINPUT_NO_DEVICES = "1";
          SEMMELTV_DECODER = cfg.decoder;
        };
    };

    assertions = [
      {
        assertion = cfg.tv.decodes == [ ] || cfg.tv.renderer != null;
        message = "services.semmeltv.tv.decodes needs services.semmeltv.tv.renderer, the TV that plays them.";
      }
    ];

    networking.firewall.allowedTCPPorts = lib.mkIf (cfg.tv.decodes != [ ]) [ cfg.tv.port ];

    environment.systemPackages = [
      pkgs.sqlite
      blankCursor
      stats
    ];

    environment.etc."semmeltv/revision" = lib.mkIf (config.system.configurationRevision != null) {
      text = config.system.configurationRevision;
    };

    systemd.services.cage-tty1 = {
      after = [ "systemd-tmpfiles-setup.service" ];
      wants = lib.optional cfg.split "semmeltv.service";
      restartIfChanged = lib.mkForce true;
      serviceConfig = {
        Restart = "always";
        RestartSec = 2;
      };
    };

    systemd.services.semmeltv = lib.mkIf cfg.split {
      description = "SemmelTV core: the API, the library, the TV's remote and what plays";
      wantedBy = [ "multi-user.target" ];
      after = [
        "systemd-tmpfiles-setup.service"
        "network.target"
        "cage-tty1.service"
      ];
      startLimitIntervalSec = 0;
      environment = cfg.coreEnvironment // {
        SEMMELTV_TV_SOCKET = tvSocket;
      };
      serviceConfig = {
        ExecStart = "${lib.getExe cfg.package} core";
        User = "semmeltv";
        Group = "users";
        WorkingDirectory = cfg.stateDir;
        Type = "notify";
        NotifyAccess = "main";
        WatchdogSec = 30;
        Restart = "always";
        RestartSec = 1;
        RuntimeDirectory = "semmeltv";
        RuntimeDirectoryMode = "0700";
      };
    };

    services.logind.settings.Login = {
      KillUserProcesses = true;
      KillOnlyUsers = [ "semmeltv" ];
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
      SUBSYSTEM=="input", KERNEL=="event*", SUBSYSTEMS=="rc", ENV{LIBINPUT_IGNORE_DEVICE}="1"
    '';
  };
}
