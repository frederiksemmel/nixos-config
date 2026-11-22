{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.beachparty;
  subname = lib.removeSuffix ".${cfg.zone}" cfg.domain;
in
{
  options.services.beachparty = {
    enable = lib.mkEnableOption "the website of Semmels Beachparty 2023, served by nginx";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.callPackage ./package.nix { };
      defaultText = lib.literalExpression "beachparty";
    };

    domain = lib.mkOption {
      type = lib.types.str;
      example = "beachparty.semmeltv.ch";
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
      example = "semmeltv3.semmeltv.ch";
      description = "Name the CNAME of `domain` points at, one that reaches this host's `port`.";
    };

    port = lib.mkOption {
      type = lib.types.port;
      default = 8445;
      description = "Port the site listens on for TLS connections that start with a PROXY protocol header.";
    };

    relayedFrom = lib.mkOption {
      type = lib.types.str;
      example = "192.168.1.178";
      description = "Address of the host that relays connections to `port`, the only one whose PROXY protocol header is trusted.";
    };

    desecTokenFile = lib.mkOption {
      type = lib.types.str;
      example = "/run/agenix/desec_token";
      description = "File with `DESEC_TOKEN=<token>`, a deSEC token that may change the zone's records.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = lib.hasSuffix ".${cfg.zone}" cfg.domain;
        message = "services.beachparty.domain must lie in services.beachparty.zone";
      }
    ];

    security.acme = {
      acceptTerms = true;
      certs.${cfg.domain} = {
        dnsProvider = "desec";
        environmentFile = cfg.desecTokenFile;
        dnsResolver = "9.9.9.9:53";
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
            addr = "0.0.0.0";
            inherit (cfg) port;
            ssl = true;
            proxyProtocol = true;
          }
        ];
        root = cfg.package;
        extraConfig = ''
          set_real_ip_from ${cfg.relayedFrom};
          real_ip_header proxy_protocol;
        '';
        locations."= /api/events".extraConfig = ''
          default_type application/json;
          return 200 '{"info":[],"events":[]}';
        '';
        locations."/_app/immutable/".extraConfig = ''
          expires max;
        '';
      };
    };

    networking.firewall.allowedTCPPorts = [ cfg.port ];

    systemd.services.beachparty-dns = {
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
