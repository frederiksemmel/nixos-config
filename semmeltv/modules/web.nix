{ crane, nixpkgs }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.semmeltv;
  newer = nixpkgs.legacyPackages.${pkgs.stdenv.hostPlatform.system};
  tailscale = lib.getExe config.services.tailscale.package;
  api = "http://127.0.0.1:${toString cfg.api.port}";
  web = "http://127.0.0.1:${toString cfg.web.port}";
in
{
  options.services.semmeltv = {
    api = {
      enable = lib.mkEnableOption "the app's HTTP API on 127.0.0.1, for the web app and any other client";

      port = lib.mkOption {
        type = lib.types.port;
        default = 8090;
        description = "Port of the API on 127.0.0.1.";
      };

      phoneMicrophone = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "plughw:Loopback,0,0";
        description = "ALSA device that audio streamed to /api/microphone is played into, or null to refuse it.";
      };
    };

    web = {
      enable = lib.mkEnableOption "the SemmelTV web app, a frontend on the API";

      port = lib.mkOption {
        type = lib.types.port;
        default = 8091;
        description = "Port of the web app on 127.0.0.1.";
      };

      package = lib.mkOption {
        type = lib.types.package;
        default = newer.callPackage ../web { inherit crane; };
        defaultText = lib.literalMD "the web app from `semmeltv/web`, built with the flake's nixpkgs for Topcoat's Rust";
        description = "The SemmelTV web app.";
      };

      tailscale = lib.mkEnableOption "the web app and the API over HTTPS on the tailnet, with `tailscale serve`";
    };
  };

  config = lib.mkIf cfg.enable (
    lib.mkMerge [
      (lib.mkIf cfg.api.enable {
        services.cage.environment = {
          SEMMELTV_API = "127.0.0.1:${toString cfg.api.port}";
        }
        // lib.optionalAttrs (cfg.api.phoneMicrophone != null) {
          SEMMELTV_PHONE_MICROPHONE = cfg.api.phoneMicrophone;
        };
      })

      (lib.mkIf cfg.web.enable {
        assertions = [
          {
            assertion = cfg.api.enable;
            message = "services.semmeltv.web needs services.semmeltv.api.enable";
          }
        ];

        systemd.services.semmeltv-web = {
          description = "SemmelTV web app";
          wantedBy = [ "multi-user.target" ];
          after = [ "network.target" ];
          environment = {
            HOST = "127.0.0.1";
            PORT = toString cfg.web.port;
            SEMMELTV_API_URL = api;
          };
          serviceConfig = {
            ExecStart = lib.getExe cfg.web.package;
            DynamicUser = true;
            Restart = "always";
            RestartSec = 2;
            PrivateTmp = true;
            ProtectSystem = "strict";
            ProtectHome = true;
            NoNewPrivileges = true;
          };
        };
      })

      (lib.mkIf (cfg.web.enable && cfg.web.tailscale) {
        services.tailscale.enable = true;

        systemd.services.semmeltv-web-https = {
          description = "Serve the SemmelTV web app and API over HTTPS on the tailnet";
          wantedBy = [ "multi-user.target" ];
          wants = [ "tailscaled.service" ];
          after = [ "tailscaled.service" ];
          serviceConfig = {
            Type = "oneshot";
            RemainAfterExit = true;
            ExecStart = [
              "${tailscale} serve reset"
              "${tailscale} serve --bg --https=443 ${web}"
              "${tailscale} serve --bg --https=443 --set-path=/api ${api}/api"
            ];
            ExecStop = "${tailscale} serve reset";
          };
        };
      })
    ]
  );
}
