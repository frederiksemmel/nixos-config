{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.ramura-preview;
  subname = lib.removeSuffix ".${cfg.zone}" cfg.domain;
in
{
  options.services.ramura-preview = {
    enable = lib.mkEnableOption "previews of the feedback pull requests on Ramura's site, one folder per issue";

    domain = lib.mkOption {
      type = lib.types.str;
      example = "ramura.semmeltv.ch";
      description = "Name the previews are served on over HTTPS, at /issue-N/. A CNAME at deSEC points it at `target`.";
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

    port = lib.mkOption {
      type = lib.types.port;
      default = 8447;
      description = "Port nginx takes the relayed TLS connections on, each starting with a PROXY protocol header.";
    };

    desecTokenFile = lib.mkOption {
      type = lib.types.path;
      description = "Environment file with DESEC_TOKEN, for the CNAME and the certificate's DNS challenge.";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.tmpfiles.rules = [ "d /var/lib/ramura-preview 0755 root root -" ];

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
            addr = "127.0.0.1";
            inherit (cfg) port;
            ssl = true;
            proxyProtocol = true;
          }
        ];
        root = "/var/lib/ramura-preview";
        extraConfig = ''
          set_real_ip_from 127.0.0.1;
          real_ip_header proxy_protocol;
          add_header Cache-Control "no-cache";
          add_header X-Robots-Tag "noindex";
        '';
      };
    };

    systemd.services.ramura-preview-dns = {
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
