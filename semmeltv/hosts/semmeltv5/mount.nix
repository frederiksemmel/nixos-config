{
  config,
  flake,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.semmeltv;
  library = "${cfg.stateDir}/library";
  meta = "${cfg.stateDir}/meta";
  cacheDir = "${cfg.stateDir}/rclone-cache";
  remoteHost = "api.pcloud.com";

  rcloneConfig = ''
    config="$RUNTIME_DIRECTORY/rclone.conf"
    umask 077
    printf '[pcloud]\ntype = pcloud\nhostname = ${remoteHost}\n' > "$config"
    rclone config update pcloud config_refresh_token=false \
      token="$(cat ${config.age.secrets.pcloud_token.path})" \
      --config "$config" --non-interactive > /dev/null
  '';

  mount = pkgs.writeShellApplication {
    name = "semmeltv-mount";
    runtimeInputs = [ pkgs.rclone ];
    text = ''
      ${rcloneConfig}
      mkdir -p ${library}
      exec rclone mount pcloud:SemmelTV ${library} \
        --config "$config" \
        --read-only \
        --allow-non-empty \
        --filter '+ /Series/**' \
        --filter '+ /Movies/**' \
        --filter '- **' \
        --cache-dir ${cacheDir} \
        --vfs-cache-mode full \
        --vfs-cache-max-age 8760h \
        --vfs-cache-min-free-space 5G \
        --vfs-read-ahead 256M \
        --dir-cache-time 1h
    '';
  };

  sync = pkgs.writeShellApplication {
    name = "semmeltv-sync";
    runtimeInputs = [
      pkgs.rclone
      pkgs.coreutils
      pkgs.findutils
    ];
    text = ''
      ${rcloneConfig}
      umask 022
      remote=$(rclone hashsum sha1 pcloud:SemmelTV --include /library.toml --config "$config" | cut -d' ' -f1)
      if [ -z "$remote" ]; then
        echo "pcloud:SemmelTV/library.toml not found"
        exit 1
      fi
      local=$(sha1sum ${meta}/library.toml 2>/dev/null | cut -d' ' -f1 || true)
      if [ "$remote" = "$local" ] && [ -d ${meta}/Artwork ]; then
        echo "library.toml unchanged ($remote), artwork not synced"
        exit 0
      fi
      echo "library.toml changed (''${local:-none} -> $remote), syncing"
      rclone copyto pcloud:SemmelTV/library.toml ${meta}/.library.toml.new --config "$config"
      rclone copy pcloud:SemmelTV/Artwork ${meta}/Artwork --config "$config" --stats 0
      mv -f ${meta}/.library.toml.new ${meta}/library.toml
      rclone sync pcloud:SemmelTV/Artwork ${meta}/Artwork --config "$config" --stats 0
      echo "synced library.toml $(sha1sum ${meta}/library.toml | cut -d' ' -f1) and $(find ${meta}/Artwork -type f | wc -l) artwork files"
    '';
  };
in
{
  imports = [ flake.inputs.agenix.nixosModules.default ];

  options.services.semmeltv.offlineCacheGiB = lib.mkOption {
    type = lib.types.ints.positive;
    description = "Disk space for downloaded titles, in GiB. Past it the app removes whole titles, least recently watched first, until they fit in 90% of it.";
  };

  config = {
    age.secrets.pcloud_token = {
      file = ../../../secrets/pcloud_token.age;
      owner = "semmeltv";
    };

    services.cage.environment = {
      SEMMELTV_META = meta;
      SEMMELTV_VFS_CACHE = "${cacheDir}/vfs/pcloud/SemmelTV";
      SEMMELTV_VFS_CACHE_META = "${cacheDir}/vfsMeta/pcloud/SemmelTV";
      SEMMELTV_VFS_CACHE_BYTES = toString (cfg.offlineCacheGiB * 1024 * 1024 * 1024);
      SEMMELTV_REMOTE = "${remoteHost}:443";
    };

    systemd.tmpfiles.settings.semmeltv = lib.genAttrs [ cacheDir meta ] (_: {
      d = {
        user = "semmeltv";
        group = "users";
        mode = "0755";
      };
    });

    systemd.services.cage-tty1 = {
      after = [ "semmeltv-mount.service" ];
      wants = [ "semmeltv-mount.service" ];
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

    systemd.services.semmeltv-sync = {
      description = "Copy SemmelTV's library.toml and artwork from pCloud to local disk";
      after = [
        "network-online.target"
        "systemd-tmpfiles-setup.service"
      ];
      wants = [ "network-online.target" ];
      serviceConfig = {
        Type = "oneshot";
        User = "semmeltv";
        RuntimeDirectory = "semmeltv-sync";
        RuntimeDirectoryMode = "0700";
        ExecStart = lib.getExe sync;
      };
    };

    systemd.timers.semmeltv-sync = {
      wantedBy = [ "timers.target" ];
      timerConfig = {
        OnBootSec = "30s";
        OnUnitActiveSec = "15min";
        RandomizedDelaySec = "1min";
      };
    };
  };
}
