{ pkgs, ... }:
let
  router = "192.168.1.1";
  logDir = "/var/log/blocky";

  adblock-stats = pkgs.writeShellApplication {
    name = "adblock-stats";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.gawk
    ];
    text = ''
      days=''${1:-1}
      if ! [[ $days =~ ^[1-9][0-9]*$ ]]; then
        echo "usage: adblock-stats [DAYS]   (default 1: today only)" >&2
        exit 2
      fi
      files=()
      for ((i = days - 1; i >= 0; i--)); do
        f=${logDir}/$(date -d "-$i days" +%F)_ALL.log
        [[ -r $f ]] && files+=("$f")
      done
      if ((''${#files[@]} == 0)); then
        echo "no query log for the last $days day(s) in ${logDir}" >&2
        exit 1
      fi
      awk -F '\t' -v days="$days" '
        function top(arr, n, title, extra,   k, i, j, keys, tmp) {
          printf "\n%s\n", title
          i = 0
          for (k in arr) keys[++i] = k
          for (j = 1; j <= i; j++)
            for (k = j + 1; k <= i; k++)
              if (arr[keys[k]] > arr[keys[j]]) { tmp = keys[j]; keys[j] = keys[k]; keys[k] = tmp }
          for (j = 1; j <= i && j <= n; j++) {
            if (extra == "blocked")
              printf "  %7d  %5d blocked  %s\n", arr[keys[j]], clientBlocked[keys[j]], keys[j]
            else
              printf "  %7d  %s\n", arr[keys[j]], keys[j]
          }
        }
        {
          total++
          type = $9
          name = $6
          sub(/\.$/, "", name)
          client = ($3 != "") ? $3 : $2
          clients[client]++
          if (type == "BLOCKED") { blocked++; blockedDomains[name]++; clientBlocked[client]++ }
          else if (type == "CACHED") cached++
        }
        END {
          printf "Queries over %d day(s): %d\n", days, total
          printf "  blocked  %7d  (%.1f%%)\n", blocked, total ? 100 * blocked / total : 0
          printf "  cached   %7d  (%.1f%%)\n", cached, total ? 100 * cached / total : 0
          top(blockedDomains, 15, "Most blocked domains")
          top(clients, 15, "Devices", "blocked")
        }
      ' "''${files[@]}"
    '';
  };
in
{
  services.blocky = {
    enable = true;
    settings = {
      ports = {
        dns = 53;
        http = "127.0.0.1:4000";
      };
      upstreams = {
        init.strategy = "fast";
        strategy = "parallel_best";
        groups.default = [
          "https://dns.quad9.net/dns-query"
          "https://cloudflare-dns.com/dns-query"
        ];
      };
      bootstrapDns = [
        "tcp+udp:9.9.9.9"
        "tcp+udp:1.1.1.1"
      ];
      conditional.mapping = {
        "." = router;
        home = router;
        "1.168.192.in-addr.arpa" = router;
      };
      clientLookup.upstream = router;
      blocking = {
        denylists.ads = [
          "https://cdn.jsdelivr.net/gh/hagezi/dns-blocklists@latest/wildcard/pro-onlydomains.txt"
        ];
        clientGroupsBlock.default = [ "ads" ];
        blockType = "zeroIp";
        loading = {
          strategy = "fast";
          refreshPeriod = "24h";
        };
      };
      caching = {
        minTime = "5m";
        prefetching = true;
      };
      queryLog = {
        type = "csv";
        target = logDir;
        logRetentionDays = 7;
      };
      prometheus.enable = true;
    };
  };

  networking.firewall.interfaces.end0 = {
    allowedTCPPorts = [ 53 ];
    allowedUDPPorts = [ 53 ];
  };

  environment.systemPackages = [
    adblock-stats
    pkgs.blocky
  ];
}
