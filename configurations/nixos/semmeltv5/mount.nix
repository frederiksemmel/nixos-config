{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.semmeltv;
  stateDir = "/var/lib/semmeltv";
  library = "${stateDir}/library";
  cacheDir = "${stateDir}/rclone-cache";
  remoteHost = "api.pcloud.com";

  mount = pkgs.writeShellApplication {
    name = "semmeltv-mount";
    runtimeInputs = [ pkgs.rclone ];
    text = ''
      config="$RUNTIME_DIRECTORY/rclone.conf"
      umask 077
      printf '[pcloud]\ntype = pcloud\nhostname = ${remoteHost}\n' > "$config"
      rclone config update pcloud config_refresh_token=false \
        token="$(cat ${config.age.secrets.pcloud_token.path})" \
        --config "$config" --non-interactive > /dev/null

      mkdir -p ${library}
      exec rclone mount pcloud: ${library} \
        --config "$config" \
        --read-only \
        --allow-non-empty \
        --filter '+ /library.toml' \
        --filter '+ /Series/**' \
        --filter '+ /Movies/**' \
        --filter '- **' \
        --cache-dir ${cacheDir} \
        --vfs-cache-mode full \
        --vfs-cache-max-size ${toString cfg.offlineCacheGiB}G \
        --vfs-cache-max-age 8760h \
        --vfs-cache-min-free-space 5G \
        --vfs-read-ahead 256M \
        --dir-cache-time 1h
    '';
  };
in
{
  options.semmeltv.offlineCacheGiB = lib.mkOption {
    type = lib.types.ints.positive;
    description = "Disk space for downloaded titles, in GiB. The least recently watched are evicted first.";
  };

  config = {
    age.secrets.pcloud_token = {
      file = ../../../secrets/pcloud_token.age;
      owner = "semmeltv";
    };

    services.cage.environment = {
      SEMMELTV_VFS_CACHE = "${cacheDir}/vfs/pcloud";
      SEMMELTV_VFS_CACHE_BYTES = toString (cfg.offlineCacheGiB * 1024 * 1024 * 1024);
      SEMMELTV_REMOTE = "${remoteHost}:443";
    };

    systemd.tmpfiles.settings.semmeltv.${cacheDir}.d = {
      user = "semmeltv";
      group = "users";
      mode = "0755";
    };

    systemd.services.semmeltv-mount = {
      description = "Read-only pCloud mount for SemmelTV";
      wantedBy = [ "multi-user.target" ];
      after = [
        "network-online.target"
        "systemd-tmpfiles-setup.service"
      ];
      wants = [ "network-online.target" ];
      path = [ "/run/wrappers" ];
      serviceConfig = {
        Type = "notify";
        User = "semmeltv";
        RuntimeDirectory = "semmeltv-mount";
        RuntimeDirectoryMode = "0700";
        ExecStartPre = "-/run/wrappers/bin/fusermount3 -uz ${library}";
        ExecStart = lib.getExe mount;
        ExecStopPost = "-/run/wrappers/bin/fusermount3 -uz ${library}";
        Restart = "on-failure";
        RestartSec = 30;
      };
    };
  };
}
