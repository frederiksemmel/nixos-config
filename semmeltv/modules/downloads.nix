{
  nixpkgs,
  vpnConfinement,
  agenix,
}:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.semmeltv.downloads;
  semmeltv = config.services.semmeltv;
  newer = nixpkgs.legacyPackages.${pkgs.stdenv.hostPlatform.system};
  confined = cfg.wireguardConfigFile != null;
  namespace = "mullvad";
  namespaceAddress = "192.168.15.1";
  serviceHost = if confined then namespaceAddress else "127.0.0.1";
  jackettPort = 9117;
  rqbitPort = 3030;
  jackettData = config.services.jackett.dataDir;
  downloadDir = "${semmeltv.stateDir}/downloads";
  stateDir = "/var/lib/semmeltv-downloads";
  runtimeDir = "/run/semmeltv-downloads";
  remoteHost = "api.pcloud.com";

  rqbit = newer.rqbit.overrideAttrs (old: {
    cargoBuildFeatures = (old.cargoBuildFeatures or [ ]) ++ [ "disable-upload" ];
    cargoCheckFeatures = (old.cargoCheckFeatures or [ ]) ++ [ "disable-upload" ];
  });

  indexer =
    id: site: extra:
    pkgs.writeText "${id}.json" (
      builtins.toJSON (
        [
          {
            id = "sitelink";
            type = "inputstring";
            name = "Site Link";
            value = site;
          }
        ]
        ++ extra
        ++ [
          {
            id = "tags";
            type = "inputtags";
            name = "Tags";
            value = "";
          }
        ]
      )
    );
  indexers = {
    therarbg = indexer "therarbg" "https://therarbg.to/" [
      {
        id = "sortrequestedfromsite";
        type = "inputselect";
        name = "Sort requested from site";
        value = "-se";
      }
    ];
    knaben = indexer "knaben" "https://knaben.org/" [ ];
    thepiratebay = indexer "thepiratebay" "https://thepiratebay.org/" [
      {
        id = "apiurl";
        type = "inputstring";
        name = "API URL";
        value = "apibay.org";
      }
    ];
    torrentscsv = indexer "torrentscsv" "https://torrents-csv.com/" [ ];
    yts = indexer "yts" "https://yts.gg/" [
      {
        id = "apiurl";
        type = "inputstring";
        name = "API URL";
        value = "movies-api.accel.li";
      }
    ];
  };

  jackettSetup = pkgs.writeShellApplication {
    name = "jackett-setup";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.jq
    ];
    text = ''
      data=${lib.escapeShellArg jackettData}
      mkdir -p "$data/Indexers"
      config="$data/ServerConfig.json"
      if [ ! -s "$config" ]; then
        key=$(tr -dc 'a-z0-9' < /dev/urandom | head -c 32)
        printf '{"APIKey": "%s"}\n' "$key" > "$config"
      fi
      jq '.Port = ${toString jackettPort} | .AllowExternal = ${lib.boolToString confined} | .UpdateDisabled = true | .FlareSolverrUrl = null' \
        "$config" > "$config.new"
      mv "$config.new" "$config"
      ${lib.concatStrings (
        lib.mapAttrsToList (id: file: ''
          [ -e "$data/Indexers/${id}.json" ] || install -m 600 ${file} "$data/Indexers/${id}.json"
        '') (lib.filterAttrs (id: _: lib.elem id cfg.indexers) indexers)
      )}
      wanted=" ${lib.concatStringsSep " " cfg.indexers} "
      for existing in "$data"/Indexers/*.json; do
        [ -e "$existing" ] || continue
        id=$(basename "$existing" .json)
        case "$wanted" in
          *" $id "*) ;;
          *) rm -f "$existing" "$existing.bak" ;;
        esac
      done
    '';
  };

  downloadsSetup = pkgs.writeShellApplication {
    name = "semmeltv-downloads-setup";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.jq
      pkgs.rclone
    ];
    text = ''
      umask 077
      jq -r .APIKey ${lib.escapeShellArg jackettData}/ServerConfig.json > ${runtimeDir}/jackett-key
      printf '[pcloud]\ntype = pcloud\nhostname = ${remoteHost}\n' > ${runtimeDir}/rclone.conf
      rclone config update pcloud config_refresh_token=false \
        token="$(cat ${config.age.secrets.pcloud_token.path})" \
        --config ${runtimeDir}/rclone.conf --non-interactive > /dev/null
      chown semmeltv ${runtimeDir}/jackett-key ${runtimeDir}/rclone.conf
    '';
  };
in
{
  imports = [
    vpnConfinement
    agenix
  ];

  options.services.semmeltv.downloads = {
    enable = lib.mkEnableOption "downloading films and series on request, through Jackett and rqbit";

    wireguardConfigFile = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = "Mullvad wg-quick file. Jackett and rqbit then run in a network namespace whose only way out is this tunnel.";
    };

    testWithoutVpn = lib.mkEnableOption "Jackett and rqbit on the host network, without the tunnel and its kill switch. Temporary, until the Mullvad WireGuard file exists";

    port = lib.mkOption {
      type = lib.types.port;
      default = 8092;
      description = "Port of the downloads service on 127.0.0.1.";
    };

    quotaGiB = lib.mkOption {
      type = lib.types.ints.positive;
      default = 30;
      description = "Most space that running downloads may take.";
    };

    reserveGiB = lib.mkOption {
      type = lib.types.ints.positive;
      default = 10;
      description = "Free space a download must leave on the disk, so it never forces the app to evict cached titles.";
    };

    indexers = lib.mkOption {
      type = lib.types.listOf (lib.types.enum (lib.attrNames indexers));
      default = lib.attrNames indexers;
      description = "Jackett indexers to search.";
    };
  };

  config = lib.mkIf cfg.enable (
    lib.mkMerge [
      {
        assertions = [
          {
            assertion = confined != cfg.testWithoutVpn;
            message = "services.semmeltv.downloads needs either wireguardConfigFile or testWithoutVpn, not both";
          }
          {
            assertion = semmeltv.api.enable;
            message = "services.semmeltv.downloads needs services.semmeltv.api.enable";
          }
        ];

        age.secrets = {
          tmdb_token = {
            file = ../../secrets/tmdb_token.age;
            owner = "semmeltv";
          };
          mdblist_api_key = {
            file = ../../secrets/mdblist_api_key.age;
            owner = "semmeltv";
          };
        };

        services.jackett = {
          enable = true;
          package = newer.jackett;
          port = jackettPort;
        };

        systemd.services.jackett.serviceConfig.ExecStartPre = lib.getExe jackettSetup;

        services.rqbit = {
          enable = true;
          package = rqbit;
          httpHost = serviceHost;
          httpPort = rqbitPort;
          inherit downloadDir;
        };

        systemd.services.rqbit = {
          environment = {
            RQBIT_DISABLE_UPLOAD = "true";
            RQBIT_LSD_DISABLE = "true";
            RQBIT_UPNP_PORT_FORWARD_DISABLE = "true";
          };
          serviceConfig.UMask = lib.mkForce "0007";
        };

        users.users.semmeltv.extraGroups = [ config.services.rqbit.group ];

        systemd.tmpfiles.settings.semmeltv-downloads.${downloadDir}.d = {
          user = config.services.rqbit.user;
          group = config.services.rqbit.group;
          mode = "2770";
        };

        security.polkit.extraConfig = ''
          polkit.addRule(function(action, subject) {
            if (action.id == "org.freedesktop.systemd1.manage-units"
                && action.lookup("unit") == "semmeltv-sync.service"
                && action.lookup("verb") == "start"
                && subject.user == "semmeltv") {
              return polkit.Result.YES;
            }
          });
        '';

        systemd.services.semmeltv-downloads = {
          description = "SemmelTV downloads: search, confirm, download, then add to the library";
          wantedBy = [ "multi-user.target" ];
          wants = [
            "network-online.target"
            "jackett.service"
            "rqbit.service"
          ];
          after = [
            "network-online.target"
            "jackett.service"
            "rqbit.service"
            "systemd-tmpfiles-setup.service"
          ];
          path = [
            pkgs.rclone
            pkgs.coreutils
            pkgs.util-linux
            config.systemd.package
          ];
          environment = {
            SEMMELTV_DOWNLOADS_LISTEN = "127.0.0.1:${toString cfg.port}";
            SEMMELTV_DOWNLOADS_STATE = stateDir;
            SEMMELTV_DOWNLOADS_DIR = downloadDir;
            SEMMELTV_DOWNLOADS_QUOTA_GIB = toString cfg.quotaGiB;
            SEMMELTV_DOWNLOADS_RESERVE_GIB = toString cfg.reserveGiB;
            SEMMELTV_TORZNAB = "http://${serviceHost}:${toString jackettPort}/api/v2.0/indexers/all/results/torznab/api";
            SEMMELTV_TORZNAB_KEY_FILE = "${runtimeDir}/jackett-key";
            SEMMELTV_RQBIT = "http://${serviceHost}:${toString rqbitPort}";
            SEMMELTV_META = "${semmeltv.stateDir}/meta";
            SEMMELTV_LIBRARY_REMOTE = "pcloud:SemmelTV";
            RCLONE_CONFIG = "${runtimeDir}/rclone.conf";
            TMDB_TOKEN_FILE = config.age.secrets.tmdb_token.path;
            MDBLIST_API_KEY_FILE = config.age.secrets.mdblist_api_key.path;
          };
          serviceConfig = {
            User = "semmeltv";
            StateDirectory = "semmeltv-downloads";
            RuntimeDirectory = "semmeltv-downloads";
            RuntimeDirectoryMode = "0700";
            ExecStartPre = "+${lib.getExe downloadsSetup}";
            ExecStart = "${lib.getExe semmeltv.package} downloads";
            Restart = "on-failure";
            RestartSec = 10;
          };
        };

        services.cage.environment.SEMMELTV_DOWNLOADS = "http://127.0.0.1:${toString cfg.port}";
        systemd.services.semmeltv-web = lib.mkIf semmeltv.web.enable {
          environment.SEMMELTV_DOWNLOADS_URL = "http://127.0.0.1:${toString cfg.port}";
        };
      }

      (lib.mkIf confined {
        vpnNamespaces.${namespace} = {
          enable = true;
          wireguardConfigFile = cfg.wireguardConfigFile;
          inherit namespaceAddress;
          portMappings = [
            {
              from = jackettPort;
              to = jackettPort;
            }
            {
              from = rqbitPort;
              to = rqbitPort;
            }
          ];
        };

        systemd.services.${namespace}.serviceConfig = {
          Restart = "on-failure";
          RestartSec = 30;
        };

        systemd.services.jackett.vpnConfinement = {
          enable = true;
          vpnNamespace = namespace;
        };

        systemd.services.rqbit.vpnConfinement = {
          enable = true;
          vpnNamespace = namespace;
        };
      })
    ]
  );
}
