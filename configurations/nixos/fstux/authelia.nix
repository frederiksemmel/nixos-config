{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.autheliaDevice;
in
{
  options.services.autheliaDevice = {
    enable = lib.mkEnableOption "Authelia per-device setup";

    deviceIP = lib.mkOption {
      type = lib.types.str;
      description = "IP address of this device (VLAN access)";
    };

    tailscaleHostname = lib.mkOption {
      type = lib.types.str;
      description = "Tailscale MagicDNS hostname";
    };

    usersFile = lib.mkOption {
      type = lib.types.path;
      description = "Path to users.yaml";
    };
  };

  config = lib.mkIf cfg.enable {
    services.authelia.instances.main = {
      enable = true;
      secrets = {
        jwtSecretFile = "/run/secrets/authelia/jwt";
        storageEncryptionKeyFile = "/run/secrets/authelia/storage";
        sessionSecretFile = "/run/secrets/authelia/session";
      };
      settings = {
        theme = "auto";
        default_2fa_method = "";

        server.address = "tcp://127.0.0.1:9091";

        log = {
          level = "info";
          format = "text";
        };

        authentication_backend.file = {
          path = "/etc/authelia/users.yaml";
          watch = false;
          password.algorithm = "argon2id";
        };

        access_control = {
          default_policy = "deny";
          rules = [
            {
              domain = [
                cfg.deviceIP
                cfg.tailscaleHostname
              ];
              policy = "one_factor";
              subject = [
                "group:admin"
                "group:users"
              ];
            }
          ];
        };

        session = {
          name = "authelia_session";
          cookies = [
            {
              domain = cfg.deviceIP;
              authelia_url = "https://${cfg.deviceIP}/authelia";
              default_redirection_url = "https://${cfg.deviceIP}";
            }
            {
              domain = cfg.tailscaleHostname;
              authelia_url = "https://${cfg.tailscaleHostname}/authelia";
              default_redirection_url = "https://${cfg.tailscaleHostname}";
            }
          ];
          expiration = "12h";
          inactivity = "2h";
        };

        storage.local.path = "/var/lib/authelia-main/db.sqlite3";

        notifier.filesystem.filename = "/var/lib/authelia-main/notifications.txt";
      };
    };

    environment.etc."authelia/users.yaml" = {
      source = cfg.usersFile;
      mode = "0600";
      user = "authelia-main";
    };
  };
}
