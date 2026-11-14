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
  serve = if cfg.web.funnel then "funnel" else "serve";
  lan = cfg.web.lan;
  lanPort = 8443;
  subname = lib.removeSuffix ".${lan.zone}" lan.domain;

  publishAddress = pkgs.writeShellApplication {
    name = "semmeltv-publish-address";
    runtimeInputs = [
      pkgs.curl
      pkgs.iproute2
      pkgs.gnused
    ];
    text = ''
      address=$(ip -4 route get 1.1.1.1 | sed -n 's/.* src \([0-9.]*\).*/\1/p')
      if [[ -z $address ]]; then
        echo "no IPv4 address yet"
        exit 0
      fi
      published="$STATE_DIRECTORY/address"
      if [[ $(cat "$published" 2>/dev/null) == "$address" ]]; then
        exit 0
      fi
      curl --fail-with-body --silent --show-error --request PATCH \
        --header "Authorization: Token $DESEC_TOKEN" \
        --header "Content-Type: application/json" \
        --data '[{"subname": "${subname}", "type": "A", "ttl": 3600, "records": ["'"$address"'"]}]' \
        "https://desec.io/api/v1/domains/${lan.zone}/rrsets/"
      echo
      printf '%s\n' "$address" >"$published"
      echo "${lan.domain} now points to $address"
    '';
  };
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

      funnel = lib.mkEnableOption "the web app and the API on the public internet, with `tailscale funnel` in place of `tailscale serve`";

      lan = {
        domain = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "semmeltv5.semmeltv.ch";
          description = ''
            Name the web app and the API answer on over HTTPS in the local network, or null for none.
            Its A record at deSEC follows the TV's local IPv4 address, so the name only works at home;
            its Let's Encrypt certificate comes from a DNS challenge at deSEC.
          '';
        };

        zone = lib.mkOption {
          type = lib.types.str;
          default = "semmeltv.ch";
          description = "The domain at deSEC that `domain` lies in.";
        };

        desecTokenFile = lib.mkOption {
          type = lib.types.str;
          example = "/run/agenix/desec_token";
          description = "File with `DESEC_TOKEN=<token>`, a deSEC token that may change the zone's records.";
        };
      };
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
          unitConfig.StartLimitIntervalSec = 0;
          serviceConfig = {
            Type = "oneshot";
            RemainAfterExit = true;
            Restart = "on-failure";
            RestartSec = 5;
            ExecStart = [
              "${tailscale} serve reset"
              "${tailscale} ${serve} --bg --https=443 ${web}"
              "${tailscale} ${serve} --bg --https=443 --set-path=/api ${api}/api"
            ];
            ExecStop = "${tailscale} serve reset";
          };
        };
      })

      (lib.mkIf (cfg.web.enable && lan.domain != null) {
        assertions = [
          {
            assertion = lib.hasSuffix ".${lan.zone}" lan.domain;
            message = "services.semmeltv.web.lan.domain must lie in services.semmeltv.web.lan.zone";
          }
        ];

        security.acme = {
          acceptTerms = true;
          certs.${lan.domain} = {
            dnsProvider = "desec";
            environmentFile = lan.desecTokenFile;
            dnsResolver = "9.9.9.9:53";
            group = config.services.nginx.group;
            reloadServices = [ "nginx.service" ];
          };
        };

        services.nginx = {
          enable = true;
          recommendedProxySettings = true;
          recommendedTlsSettings = true;
          recommendedGzipSettings = true;
          virtualHosts.${lan.domain} = {
            useACMEHost = lan.domain;
            onlySSL = true;
            listen = [
              {
                addr = "0.0.0.0";
                port = lanPort;
                ssl = true;
              }
            ];
            locations."/".proxyPass = web;
            locations."/api" = {
              proxyPass = api;
              proxyWebsockets = true;
              extraConfig = ''
                proxy_read_timeout 1h;
                proxy_send_timeout 1h;
                proxy_buffering off;
                proxy_set_header Tailscale-User-Login "";
                proxy_set_header Tailscale-Funnel-Request "";
              '';
            };
          };
        };

        networking.firewall = {
          allowedTCPPorts = [ lanPort ];
          extraCommands = ''
            iptables -t nat -N semmeltv-https 2>/dev/null || true
            iptables -t nat -F semmeltv-https
            iptables -t nat -A semmeltv-https ! -i tailscale0 -m addrtype --dst-type LOCAL -p tcp --dport 443 -j REDIRECT --to-ports ${toString lanPort}
            iptables -t nat -C PREROUTING -j semmeltv-https 2>/dev/null || iptables -t nat -A PREROUTING -j semmeltv-https
          '';
          extraStopCommands = ''
            iptables -t nat -D PREROUTING -j semmeltv-https 2>/dev/null || true
            iptables -t nat -F semmeltv-https 2>/dev/null || true
            iptables -t nat -X semmeltv-https 2>/dev/null || true
          '';
        };

        systemd.services.semmeltv-publish-address = {
          description = "Point ${lan.domain} at the TV's local IPv4 address";
          wants = [ "network-online.target" ];
          after = [ "network-online.target" ];
          serviceConfig = {
            Type = "oneshot";
            ExecStart = lib.getExe publishAddress;
            EnvironmentFile = lan.desecTokenFile;
            DynamicUser = true;
            StateDirectory = "semmeltv-publish-address";
          };
        };

        systemd.timers.semmeltv-publish-address = {
          wantedBy = [ "timers.target" ];
          timerConfig = {
            OnBootSec = "30s";
            OnUnitActiveSec = "5min";
          };
        };
      })
    ]
  );
}
