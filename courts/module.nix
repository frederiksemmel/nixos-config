{ crane, nixpkgs }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.courts;
  newer = nixpkgs.legacyPackages.${pkgs.stdenv.hostPlatform.system};
  subname = lib.removeSuffix ".${cfg.zone}" cfg.domain;
in
{
  options.services.courts = {
    enable = lib.mkEnableOption "the court finder: free tennis courts from GotCourts, with watches that notify";

    package = lib.mkOption {
      type = lib.types.package;
      default = newer.callPackage ./package.nix { inherit crane; };
      defaultText = lib.literalMD "the app from `courts`, built with the flake's nixpkgs for Topcoat's Rust";
    };

    domain = lib.mkOption {
      type = lib.types.str;
      example = "courts.semmeltv.ch";
      description = ''
        Name the site answers on over HTTPS. A CNAME at deSEC points it at `target`, and its
        Let's Encrypt certificate comes from a DNS challenge at deSEC.
      '';
    };

    zone = lib.mkOption {
      type = lib.types.str;
      default = "semmeltv.ch";
      description = "The domain at deSEC that `domain` lies in.";
    };

    target = lib.mkOption {
      type = lib.types.str;
      example = "semmeltv5.semmeltv.ch";
      description = "Name the CNAME of `domain` points at, whose port 443 relays `domain` to `port`.";
    };

    address = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1";
      description = "Address nginx takes the relayed TLS connections on.";
    };

    port = lib.mkOption {
      type = lib.types.port;
      default = 8446;
      description = "Port nginx takes TLS connections on that start with a PROXY protocol header.";
    };

    relayedFrom = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1";
      description = "Address of the host that relays connections to `port`, the only one whose PROXY protocol header is trusted.";
    };

    appPort = lib.mkOption {
      type = lib.types.port;
      default = 8096;
      description = "Port the app itself listens on, on localhost, behind nginx.";
    };

    keyFile = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "/run/agenix/courts_key";
      description = ''
        File with the key to sign in with, at least 16 characters, or null for one the app makes
        up on its first start and keeps in `/var/lib/courts/key`.
      '';
    };

    desecTokenFile = lib.mkOption {
      type = lib.types.str;
      example = "/run/agenix/desec_token";
      description = "File with `DESEC_TOKEN=<token>`, a deSEC token that may change the zone's records.";
    };

    clubs = lib.mkOption {
      type = lib.types.listOf lib.types.ints.positive;
      description = "GotCourts club ids to ask about.";
    };

    home = lib.mkOption {
      type = lib.types.str;
      default = "47.3779,8.5403";
      description = "Latitude and longitude that distances count from.";
    };

    days = lib.mkOption {
      type = lib.types.ints.between 1 60;
      default = 14;
      description = "How many days ahead to ask about.";
    };

    perMinute = lib.mkOption {
      type = lib.types.ints.positive;
      default = 20;
      description = ''
        How many club-days after today to ask about each minute, besides today and the watched
        days. With 14 days and 27 clubs, 20 brings every one round in about 18 minutes.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = lib.hasSuffix ".${cfg.zone}" cfg.domain;
        message = "services.courts.domain must lie in services.courts.zone";
      }
    ];

    systemd.services.courts = {
      description = "Court finder";
      wantedBy = [ "multi-user.target" ];
      wants = [ "network-online.target" ];
      after = [ "network-online.target" ];
      environment = {
        HOST = "127.0.0.1";
        PORT = toString cfg.appPort;
        COURTS_DB = "/var/lib/courts/courts.db";
        COURTS_ORIGIN = "https://${cfg.domain}";
        COURTS_CLUBS = lib.concatMapStringsSep "," toString cfg.clubs;
        COURTS_HOME = cfg.home;
        COURTS_DAYS = toString cfg.days;
        COURTS_PER_MINUTE = toString cfg.perMinute;
      }
      // lib.optionalAttrs (cfg.keyFile != null) { COURTS_KEY_FILE = "%d/key"; };
      serviceConfig = {
        ExecStart = lib.getExe cfg.package;
        LoadCredential = lib.mkIf (cfg.keyFile != null) "key:${cfg.keyFile}";
        DynamicUser = true;
        StateDirectory = "courts";
        StateDirectoryMode = "0700";
        WorkingDirectory = "/var/lib/courts";
        Restart = "always";
        RestartSec = 5;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        NoNewPrivileges = true;
      };
    };

    security.acme = {
      acceptTerms = true;
      certs.${cfg.domain} = {
        dnsProvider = "desec";
        environmentFile = cfg.desecTokenFile;
        extraLegoFlags = [
          "--dns.propagation-wait"
          "5m"
        ];
        group = config.services.nginx.group;
        reloadServices = [ "nginx.service" ];
      };
    };

    services.nginx = {
      enable = true;
      virtualHosts.${cfg.domain} = {
        useACMEHost = cfg.domain;
        onlySSL = true;
        listen = [
          {
            addr = cfg.address;
            inherit (cfg) port;
            ssl = true;
            proxyProtocol = true;
          }
        ];
        extraConfig = ''
          set_real_ip_from ${cfg.relayedFrom};
          real_ip_header proxy_protocol;
        '';
        locations."/" = {
          proxyPass = "http://127.0.0.1:${toString cfg.appPort}";
          recommendedProxySettings = true;
        };
      };
    };

    networking.firewall.allowedTCPPorts = lib.mkIf (cfg.address != "127.0.0.1") [ cfg.port ];

    systemd.services.courts-dns = {
      description = "Point ${cfg.domain} at ${cfg.target}";
      wantedBy = [ "multi-user.target" ];
      wants = [ "network-online.target" ];
      after = [ "network-online.target" ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        EnvironmentFile = cfg.desecTokenFile;
        DynamicUser = true;
        Restart = "on-failure";
        RestartSec = 60;
      };
      script = ''
        ${lib.getExe pkgs.curl} --fail-with-body --silent --show-error --request PATCH \
          --header "Authorization: Token $DESEC_TOKEN" \
          --header "Content-Type: application/json" \
          --data '[{"subname": "${subname}", "type": "CNAME", "ttl": 3600, "records": ["${cfg.target}."]}]' \
          "https://desec.io/api/v1/domains/${cfg.zone}/rrsets/"
      '';
    };
  };
}
