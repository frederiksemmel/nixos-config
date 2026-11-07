{
  config,
  pkgs,
  lib,
  ...
}:
let
  cfg = config.profiles.cloud;
  # Two-way sync of an rclone remote with a local directory, run by a timer.
  # Failures and conflicts always notify. A run requested from the power menu
  # finds the id of its "Syncing" notification in the request file and replaces
  # it with the result.
  mkBisync =
    {
      name,
      title,
      remote,
      remoteLabel,
      dir,
      host,
      flags ? [ ],
      onCalendar,
    }:
    let
      request = "\"$XDG_RUNTIME_DIR\"/${name}-sync.notify-id";
      sync = pkgs.writeShellApplication {
        name = "${name}-sync";
        runtimeInputs = with pkgs; [
          config.programs.rclone.package
          coreutils
          findutils
          getent
          gnugrep
          gnused
          libnotify
        ];
        text = ''
          notify() {
            local id=""
            if [ -f ${request} ]; then
              id=$(cat ${request})
              rm -f ${request}
            fi
            notify-send -a rclone -i folder-remote ''${id:+-r "$id"} "$@"
          }

          if ! getent hosts ${host} >/dev/null; then
            [ -f ${request} ] && notify "${title} sync skipped" "Offline"
            exit 0
          fi

          log=$(mktemp)
          started=$(mktemp)
          trap 'rm -f "$log" "$started"' EXIT

          if ! rclone bisync ${remote} ${dir} \
            ${lib.concatMapStrings (f: "${f} \\\n") flags}--fast-list --create-empty-src-dirs \
            --compare size,modtime,checksum --slow-hash-sync-only \
            --resilient --recover --max-lock 15m \
            --transfers 8 --checkers 16 \
            --color NEVER -v 2>&1 | tee "$log"; then
            notify -u critical "${title} sync failed" \
              "$(grep -E 'ERROR|CRITICAL' "$log" | tail -n 3 | sed -E 's/^.*(ERROR|CRITICAL) *: *//')"
            exit 1
          fi

          conflicts=$(find ${dir} -cnewer "$started" -name '*.conflict[0-9]*' -printf '%P\n')
          if [ -n "$conflicts" ]; then
            notify -u critical "${title} sync: conflicts" "$conflicts"
          elif [ -f ${request} ]; then
            summary=$(sed -nE \
              -e 's/.*Path1: +[0-9]+ changes: +/From ${remoteLabel}: /p' \
              -e 's/.*Path2: +[0-9]+ changes: +/From local: /p' "$log" | tr -s ' ')
            notify "${title} synced" "''${summary:-No changes}"
          fi
        '';
      };
    in
    {
      syncNow = pkgs.writeShellApplication {
        name = "${name}-sync-now";
        runtimeInputs = with pkgs; [
          libnotify
          systemd
        ];
        text = ''
          notify-send -p -a rclone -i folder-remote "${title} sync" "Syncing…" > ${request}
          systemctl --user start --no-block ${name}-sync.service
        '';
      };
      service = {
        Unit = {
          Description = "Bisync ${remote} with ${dir}";
          After = [
            "agenix.service"
            "rclone-config.service"
          ];
          Wants = [ "rclone-config.service" ];
        };
        Service = {
          Type = "oneshot";
          ExecStart = lib.getExe sync;
          Nice = 10;
          IOSchedulingClass = "idle";
        };
      };
      timer = {
        Unit.Description = "Bisync ${remote} with ${dir} (${onCalendar})";
        Timer = {
          OnCalendar = onCalendar;
          Persistent = true;
        };
        Install.WantedBy = [ "timers.target" ];
      };
    };
  spadesvisionSync = mkBisync {
    name = "spadesvision";
    title = "Spadesvision";
    remote = "spadesvision:/";
    remoteLabel = "Drive";
    dir = "${config.home.homeDirectory}/spadesvision";
    host = "www.googleapis.com";
    flags = [
      "--filter-from ${config.home.homeDirectory}/.rclone_filter_spadesvision.txt"
      "--drive-skip-gdocs --drive-chunk-size 64Mi"
    ];
    onCalendar = "hourly";
  };
  scansSync = mkBisync {
    name = "scans";
    title = "Scans";
    remote = "pcloud:Scans";
    remoteLabel = "pCloud";
    dir = "${config.home.homeDirectory}/Scans";
    host = "api.pcloud.com";
    onCalendar = "*:0/15";
  };
in
{
  options.profiles.cloud = {
    enable = lib.mkEnableOption "rclone remotes for Drive and pCloud, the pCloud mount and the bisyncs";
    syncNow = lib.mkOption {
      type = lib.types.attrsOf lib.types.package;
      readOnly = true;
      default = {
        spadesvision = spadesvisionSync.syncNow;
        scans = scansSync.syncNow;
      };
      description = "Scripts that start a bisync run and report its result in a notification.";
    };
  };

  config = lib.mkIf cfg.enable {
    age.secrets = {
      spadesvision_gdrive_token.file = ../../secrets/spadesvision_gdrive_token.age;
      spadesvision_gdrive_client_secret.file = ../../secrets/spadesvision_gdrive_client_secret.age;
      pcloud_token.file = ../../secrets/pcloud_token.age;
      tmdb_token = {
        file = ../../secrets/tmdb_token.age;
        path = "${config.xdg.dataHome}/semmeltv/tmdb-token";
      };
    };

    home.sessionVariables.TMDB_TOKEN_FILE = config.age.secrets.tmdb_token.path;

    programs.rclone.remotes.spadesvision = {
      config = {
        type = "drive";
        client_id = "113233231770-v4b6rfq5ajc3imv0vs0g3fnbl2q8rd84.apps.googleusercontent.com";
        scope = "drive";
        team_drive = "0AH2EQdMAdZriUk9PVA";
        root_folder_id = "";
      };
      secrets = {
        client_secret = config.age.secrets.spadesvision_gdrive_client_secret.path;
        token = config.age.secrets.spadesvision_gdrive_token.path;
      };
    };

    # The whole account as an on-demand drive: files download only when opened,
    # into a size-capped cache. ~/Scans alone is kept locally, by scans-sync.
    programs.rclone.remotes.pcloud = {
      config = {
        type = "pcloud";
        hostname = "api.pcloud.com";
      };
      secrets.token = config.age.secrets.pcloud_token.path;
      mounts."" = {
        enable = true;
        mountPoint = "${config.home.homeDirectory}/pcloud";
        options = {
          vfs-cache-max-size = "10G";
          vfs-cache-max-age = "720h";
        };
      };
    };

    systemd.user.services."rclone-mount:@pcloud" =
      lib.mkIf config.programs.rclone.remotes.pcloud.mounts."".enable
        {
          Service.ExecStartPre = lib.mkForce [
            "-/run/wrappers/bin/fusermount3 -uz ${config.home.homeDirectory}/pcloud"
            "${lib.getExe' pkgs.coreutils "mkdir"} -p ${config.home.homeDirectory}/pcloud"
          ];
        };

    home.file.".rclone_filter_spadesvision.txt".text = ''
      - Marketing/Material für */**
      - Development/Jetson Server Dev/*Releases*/**
    '';

    systemd.user.services.spadesvision-sync = spadesvisionSync.service;
    systemd.user.timers.spadesvision-sync = spadesvisionSync.timer;
    systemd.user.services.scans-sync = scansSync.service;
    systemd.user.timers.scans-sync = scansSync.timer;
  };
}
