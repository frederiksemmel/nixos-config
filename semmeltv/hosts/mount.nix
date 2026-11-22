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

  offline = cfg.offlineCacheGiB != null;

  cacheFlags =
    if offline then
      "--cache-dir ${cacheDir} --vfs-cache-mode full --vfs-cache-max-age 8760h --vfs-cache-min-free-space 5G --vfs-read-ahead 256M"
    else
      "--vfs-cache-mode off";

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
        ${cacheFlags} \
        --dir-cache-time 1h \
        --rc \
        --rc-addr "unix://$RUNTIME_DIRECTORY/rc.sock" \
        --rc-no-auth
    '';
  };

  syncFrom = pkgs.callPackage ./sync.nix { };

  sync = pkgs.writeShellApplication {
    name = "semmeltv-sync";
    runtimeInputs = [ pkgs.rclone ];
    text = ''
      ${rcloneConfig}
      export RCLONE_CONFIG="$config"
      exec ${lib.getExe syncFrom} pcloud:SemmelTV ${meta}
    '';
  };
in
{
  imports = [ flake.inputs.agenix.nixosModules.default ];

  options.services.semmeltv.offlineCacheGiB = lib.mkOption {
    type = lib.types.nullOr lib.types.ints.positive;
    description = "Disk space for downloaded titles, in GiB. Past it the app removes whole titles, least recently watched first, until they fit in 90% of it. With null the TV only streams: rclone keeps nothing on disk, the app reads nothing ahead, and cards show no download state.";
  };

  config = {
    age.secrets.pcloud_token = {
      file = ../../secrets/pcloud_token.age;
      owner = "semmeltv";
    };

    services.cage.environment.SEMMELTV_META = meta;

    services.semmeltv.coreEnvironment = {
      SEMMELTV_META = meta;
      SEMMELTV_REMOTE = "${remoteHost}:443";
    }
    // lib.optionalAttrs offline {
      SEMMELTV_VFS_CACHE = "${cacheDir}/vfs/pcloud/SemmelTV";
      SEMMELTV_VFS_CACHE_META = "${cacheDir}/vfsMeta/pcloud/SemmelTV";
      SEMMELTV_VFS_CACHE_BYTES = toString (cfg.offlineCacheGiB * 1024 * 1024 * 1024);
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

    systemd.services.semmeltv = lib.mkIf cfg.split {
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
        SuccessExitStatus = 143;
        RestartSec = 30;
      };
    };

    systemd.services.semmeltv-sync = {
      description = "Copy SemmelTV's library.db and artwork from pCloud to local disk";
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
