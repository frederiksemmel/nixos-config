{ crane }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.epaper;
in
{
  options.services.epaper = {
    enable = lib.mkEnableOption "the Waveshare 2.7\" e-Paper HAT (B) and its Telegram bot";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.callPackage ./package.nix { inherit crane; };
      defaultText = lib.literalExpression "epaper";
    };

    tokenFile = lib.mkOption {
      type = lib.types.path;
      description = "File holding the Telegram bot token, read by systemd as root.";
    };

    allowedUsers = lib.mkOption {
      type = lib.types.listOf lib.types.ints.unsigned;
      default = [ ];
      description = "Telegram user ids allowed to draw. Everyone may draw when the list is empty.";
    };

    rotation = lib.mkOption {
      type = lib.types.enum [
        0
        90
        180
        270
      ];
      default = 180;
      description = "Rotation of the picture on the panel, in degrees.";
    };
  };

  config = lib.mkIf cfg.enable {
    hardware.raspberry-pi.config.all.base-dt-params.spi = {
      enable = true;
      value = "on";
    };

    users.users.epaper = {
      isSystemUser = true;
      group = "epaper";
      extraGroups = [
        "gpio"
        "spi"
      ];
    };
    users.groups.epaper = { };

    environment.systemPackages = [ cfg.package ];

    services.udev.extraRules = ''
      SUBSYSTEM=="spidev", KERNEL=="spidev0.0", TAG+="systemd"
    '';

    systemd.services.epaper = {
      description = "Telegram bot that draws on the e-Paper HAT";
      wantedBy = [ "dev-spidev0.0.device" ];
      bindsTo = [ "dev-spidev0.0.device" ];
      wants = [ "network-online.target" ];
      after = [
        "dev-spidev0.0.device"
        "network-online.target"
      ];
      unitConfig.ConditionPathExists = "/dev/spidev0.0";
      environment = {
        EPAPER_ROTATION = toString cfg.rotation;
      }
      // lib.optionalAttrs (cfg.allowedUsers != [ ]) {
        EPAPER_ALLOW = lib.concatMapStringsSep "," toString cfg.allowedUsers;
      };
      serviceConfig = {
        ExecStart = "${lib.getExe cfg.package} bot --token-file %d/token";
        LoadCredential = "token:${cfg.tokenFile}";
        User = "epaper";
        Group = "epaper";
        StateDirectory = "epaper";
        StateDirectoryMode = "0775";
        Restart = "on-failure";
        RestartSec = 10;

        DevicePolicy = "closed";
        DeviceAllow = [
          "char-gpiochip rw"
          "char-spi rw"
        ];
        CapabilityBoundingSet = "";
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectClock = true;
        ProtectControlGroups = true;
        ProtectHome = true;
        ProtectHostname = true;
        ProtectKernelLogs = true;
        ProtectKernelModules = true;
        ProtectKernelTunables = true;
        ProtectSystem = "strict";
        RestrictAddressFamilies = [
          "AF_INET"
          "AF_INET6"
          "AF_UNIX"
        ];
        RestrictNamespaces = true;
        RestrictRealtime = true;
        SystemCallArchitectures = "native";
      };
    };
  };
}
