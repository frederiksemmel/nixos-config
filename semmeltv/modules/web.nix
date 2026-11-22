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
  front = cfg.web.front;
  frontUrl = "http://127.0.0.1:${toString front.port}";
  maintenance = lib.getExe' front.package "semmeltv-maintenance";
  serve = if cfg.web.funnel then "funnel" else "serve";
  lan = cfg.web.lan;
  signIn = cfg.web.signIn;
  tailnetOrigin = lib.optional (
    cfg.web.tailscale && signIn.tailnet != null
  ) "https://${config.networking.hostName}.${signIn.tailnet}";
  lanOrigin = lib.optional (lan.domain != null) "https://${lan.domain}";
  untrusted = [
    "Tailscale-User-Login"
    "Tailscale-User-Name"
    "Tailscale-User-Profile-Pic"
    "Tailscale-Funnel-Request"
    "Tailscale-Headers-Info"
  ];
  stripUntrusted = lib.concatMapStrings (name: "proxy_set_header ${name} \"\";\n") untrusted;
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
      ${
        if lan.public then
          ''
            address=$(curl --silent --fail --max-time 20 https://checkipv4.dedyn.io/ || true)
          ''
        else
          ''
            address=$(ip -4 route get 1.1.1.1 | sed -n 's/.* src \([0-9.]*\).*/\1/p')
          ''
      }
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
        --data '[{"subname": "${subname}", "type": "A", "ttl": ${
          if lan.public then "900" else "3600"
        }, "records": ["'"$address"'"]}]' \
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

      front = {
        port = lib.mkOption {
          type = lib.types.port;
          default = 8093;
          description = ''
            Port of the front on 127.0.0.1, the proxy that the tailnet and the LAN name reach the web app
            and the API through. While the core or the web app doesn't answer, or during planned work, it
            answers with the bakery's closed screen and a guess of when SemmelTV is back.
          '';
        };

        controlPort = lib.mkOption {
          type = lib.types.port;
          default = 8094;
          description = "Port on 127.0.0.1 that `semmeltv-maintenance` and the core's stop hook tell the front through.";
        };

        package = lib.mkOption {
          type = lib.types.package;
          default = pkgs.callPackage ../front { inherit crane; };
          defaultText = lib.literalMD "the front from `semmeltv/front`";
          description = "The SemmelTV front, with the `semmeltv-maintenance` command.";
        };
      };

      tailscale = lib.mkEnableOption "the web app and the API over HTTPS on the tailnet, with `tailscale serve`";

      funnel = lib.mkEnableOption "the web app and the API on the public internet, with `tailscale funnel` in place of `tailscale serve`";

      signIn = {
        tailnet = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "cuscus-exponential.ts.net";
          description = "The tailnet's MagicDNS suffix, so the tailnet address of the web app is a sign-in origin.";
        };

        origins = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = tailnetOrigin ++ lanOrigin;
          defaultText = lib.literalMD "the tailnet address and the `lan.domain`, over HTTPS";
          description = ''
            Origins people sign in on. Each gets its own passkeys, since a passkey belongs to one domain,
            and only these may change anything through the API.
          '';
        };

        serviceTokenFile = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "/run/agenix/semmeltv_service_token";
          description = ''
            File with the token the web app's server follows the TV's event stream with.
            Readable by the `semmeltv` user; the web app gets it as a systemd credential.
          '';
        };
      };

      lan = {
        domain = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "semmeltv5.semmeltv.ch";
          description = ''
            Name the web app and the API answer on over HTTPS outside the tailnet, or null for none.
            Its A record at deSEC follows the TV's local IPv4 address, or its public one with `public`;
            its Let's Encrypt certificate comes from a DNS challenge at deSEC.
          '';
        };

        public = lib.mkEnableOption ''
          pointing `domain` at the home's public IPv4 address instead of the TV's local one. The router
          must forward TCP port 443 to the TV, and the name then only works at home if the router
          loops connections to its own public address back in
        '';

        forward = lib.mkOption {
          type = lib.types.attrsOf lib.types.str;
          default = { };
          example = {
            "semmeltv3.semmeltv.ch" = "192.168.1.179:8443";
          };
          description = ''
            Names of other TVs behind the same router port forward. Their HTTPS connections arrive
            here and pass through unopened, by TLS server name, to the given `host:port`, which must be
            that TV's `relayPort`: every relayed connection starts with a PROXY protocol header naming
            the visitor's address.
          '';
        };

        relayedFrom = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "192.168.1.178";
          description = ''
            Address of the TV whose `forward` relays this TV's name, or null. Relayed connections arrive
            on `relayPort` with a PROXY protocol header, which is trusted from this address only, so the
            app sees the visitor's own address and not the relaying TV's.
          '';
        };

        relayPort = lib.mkOption {
          type = lib.types.port;
          default = lanPort + 2;
          defaultText = lib.literalExpression "8445";
          readOnly = true;
          description = "Port that relayed connections, with their PROXY protocol header, arrive on.";
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
        services.semmeltv.coreEnvironment.SEMMELTV_API = "127.0.0.1:${toString cfg.api.port}";
      })

      (lib.mkIf cfg.web.enable {
        assertions = [
          {
            assertion = cfg.api.enable;
            message = "services.semmeltv.web needs services.semmeltv.api.enable";
          }
          {
            assertion = signIn.origins != [ ] && signIn.serviceTokenFile != null;
            message = "services.semmeltv.web needs sign-in: set web.signIn.serviceTokenFile and at least one origin";
          }
        ];

        services.semmeltv.coreEnvironment = {
          SEMMELTV_ORIGINS = lib.concatStringsSep "," signIn.origins;
          SEMMELTV_SERVICE_TOKEN_FILE = signIn.serviceTokenFile;
        };

        systemd.services.semmeltv-web = {
          description = "SemmelTV web app";
          wantedBy = [ "multi-user.target" ];
          after = [ "network.target" ];
          environment = {
            HOST = "127.0.0.1";
            PORT = toString cfg.web.port;
            SEMMELTV_API_URL = api;
            SEMMELTV_SERVICE_TOKEN_FILE = "%d/service-token";
          };
          serviceConfig = {
            ExecStart = lib.getExe cfg.web.package;
            LoadCredential = "service-token:${signIn.serviceTokenFile}";
            DynamicUser = true;
            Restart = "always";
            RestartSec = 2;
            PrivateTmp = true;
            ProtectSystem = "strict";
            ProtectHome = true;
            NoNewPrivileges = true;
          };
        };

        systemd.services.semmeltv-front = {
          description = "SemmelTV front: the proxy before the core and the web app, with the bakery's closed screen";
          wantedBy = [ "multi-user.target" ];
          after = [ "network.target" ];
          stopIfChanged = false;
          environment = {
            SEMMELTV_FRONT_LISTEN = "127.0.0.1:${toString front.port}";
            SEMMELTV_FRONT_CONTROL = "127.0.0.1:${toString front.controlPort}";
            SEMMELTV_CORE = "127.0.0.1:${toString cfg.api.port}";
            SEMMELTV_WEB = "127.0.0.1:${toString cfg.web.port}";
          };
          serviceConfig = {
            ExecStart = lib.getExe front.package;
            DynamicUser = true;
            StateDirectory = "semmeltv-front";
            Restart = "always";
            RestartSec = 1;
            PrivateTmp = true;
            ProtectSystem = "strict";
            ProtectHome = true;
            NoNewPrivileges = true;
          };
        };

        environment.systemPackages = [
          (pkgs.writeShellApplication {
            name = "semmeltv-maintenance";
            runtimeEnv.SEMMELTV_FRONT_CONTROL = "127.0.0.1:${toString front.controlPort}";
            text = ''exec ${maintenance} "$@"'';
          })
        ];

        systemd.services.${if cfg.split then "semmeltv" else "cage-tty1"}.serviceConfig.ExecStop =
          "-${pkgs.writeShellScript "semmeltv-announce-restart" ''
            SEMMELTV_FRONT_CONTROL=127.0.0.1:${toString front.controlPort} exec ${maintenance} restarting
          ''}";
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
              "${tailscale} ${serve} --bg --https=443 ${frontUrl}"
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

        boot.kernel.sysctl."net.ipv4.tcp_notsent_lowat" = 16384;

        services.nginx = {
          enable = true;
          recommendedProxySettings = true;
          recommendedTlsSettings = true;
          recommendedGzipSettings = true;
          commonHttpConfig = ''
            log_format timed '$remote_addr [$time_local] "$request" $status $body_bytes_sent '
                             'took $request_time s, upstream headers after $upstream_header_time s '
                             '"$http_user_agent"';
          '';
          streamConfig = lib.mkIf (lan.forward != { }) ''
            map $ssl_preread_server_name $semmeltv_tv {
            ${
              lib.concatStrings (lib.mapAttrsToList (name: target: "  ${name} ${target};\n") lan.forward)
            }  default 127.0.0.1:${toString (lanPort + 1)};
            }
            server {
              listen 0.0.0.0:${toString lanPort};
              ssl_preread on;
              proxy_pass $semmeltv_tv;
              proxy_protocol on;
            }
          '';
          virtualHosts.${lan.domain} = {
            useACMEHost = lan.domain;
            onlySSL = true;
            listen = [
              (
                if lan.forward == { } then
                  {
                    addr = "0.0.0.0";
                    port = lanPort;
                    ssl = true;
                  }
                else
                  {
                    addr = "127.0.0.1";
                    port = lanPort + 1;
                    ssl = true;
                    proxyProtocol = true;
                  }
              )
            ]
            ++ lib.optional (lan.relayedFrom != null) {
              addr = "0.0.0.0";
              port = lan.relayPort;
              ssl = true;
              proxyProtocol = true;
            };
            extraConfig = ''
              access_log /var/log/nginx/access.log timed;
            ''
            + lib.optionalString (lan.forward != { }) ''
              set_real_ip_from 127.0.0.1;
            ''
            + lib.optionalString (lan.relayedFrom != null) ''
              set_real_ip_from ${lan.relayedFrom};
            ''
            + lib.optionalString (lan.forward != { } || lan.relayedFrom != null) ''
              real_ip_header proxy_protocol;
            '';
            locations."/" = {
              proxyPass = frontUrl;
              proxyWebsockets = true;
              extraConfig = stripUntrusted + ''
                proxy_read_timeout 1h;
                proxy_send_timeout 1h;
                proxy_buffering off;
              '';
            };
          };
        };

        networking.firewall = {
          allowedTCPPorts = [ lanPort ] ++ lib.optional (lan.relayedFrom != null) lan.relayPort;
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
          description = "Point ${lan.domain} at the ${
            if lan.public then "home's public" else "TV's local"
          } IPv4 address";
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
