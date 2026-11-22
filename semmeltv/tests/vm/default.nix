{
  pkgs,
  semmeltvModule,
  semmeltv,
}:

let
  library = pkgs.callPackage ./library.nix { inherit semmeltv; };
  serviceToken = pkgs.writeText "semmeltv-service-token" "vm-test-service-token";
  hlsJs = pkgs.fetchurl {
    url = "https://cdn.jsdelivr.net/npm/hls.js@1.5.17/dist/hls.min.js";
    hash = "sha256-SEBU6M0D0/bReB+39AK9wxjYpMUn+TOpXGJOJ8yalHA=";
  };
  moveMouse = pkgs.writers.writePython3Bin "semmeltv-move-mouse" {
    libraries = [ pkgs.python3Packages.evdev ];
  } (builtins.readFile ./move-mouse.py);
  timing = {
    upNext = 6;
    quiet = 20;
    watching = 8;
  };
  groups = [
    "tv-playback"
    "tv-remote"
    "up-next"
    "resume"
    "segments"
    "streams"
    "phone-playback"
    "laptop"
    "handover"
    "live"
    "pages"
    "phone-ui"
    "feedback"
    "downloads"
    "sign-in"
    "bakeoff"
    "tv-restart"
    "maintenance"
  ];
  expectedToFail = [ ];
  proxied = {
    proxyWebsockets = true;
    extraConfig = ''
      proxy_set_header Host $host:$server_port;
      proxy_buffering off;
      proxy_read_timeout 1h;
    '';
  };
  test =
    group:
    pkgs.testers.runNixOSTest {
      name = "semmeltv-vm-${group}";
      enableOCR = true;
      extraPythonPackages = ps: [
        ps.playwright
        ps.pillow
      ];

      nodes.tv = {
        imports = [ semmeltvModule ];

        virtualisation = {
          memorySize = 4096;
          cores = 4;
          qemu.options = [
            "-vga none"
            "-device virtio-gpu-pci"
          ];
        };

        boot.kernelModules = [ "snd-dummy" ];

        services.semmeltv = {
          headphones.enable = true;
          enable = true;
          adminUser = "root";
          package = semmeltv;
          api.enable = true;
          web = {
            enable = true;
            signIn = {
              origins = [ "http://localhost:8080" ];
              serviceTokenFile = "${serviceToken}";
            };
          };
        };

        services.semmeltv.tv = {
          renderer = "http://127.0.0.1:9197/dmr";
          decodes = [ "hevc" ];
        };

        systemd.services.fake-tv = {
          description = "A TV set's UPnP renderer, for the hand-off of titles the Pi can't decode";
          wantedBy = [ "multi-user.target" ];
          before = [ "cage-tty1.service" ];
          path = [ pkgs.ffmpeg-headless ];
          serviceConfig = {
            RuntimeDirectory = "fake-tv";
            ExecStart = "${pkgs.python3}/bin/python3 ${./fake-tv.py} /run/fake-tv";
          };
        };

        systemd.services.fake-downloads = {
          description = "The downloads service's answers for the show page's season buttons";
          wantedBy = [ "multi-user.target" ];
          serviceConfig.ExecStart = "${pkgs.python3}/bin/python3 ${./fake-downloads.py} 100001";
        };

        systemd.services.fake-introdb = {
          description = "theintrodb.org's segment API, answering what the test leaves in /run/fake-introdb";
          wantedBy = [ "multi-user.target" ];
          before = [ "cage-tty1.service" ];
          serviceConfig = {
            RuntimeDirectory = "fake-introdb";
            ExecStart = "${pkgs.python3}/bin/python3 -m http.server 9198 --bind 127.0.0.1 --directory /run/fake-introdb";
          };
        };

        services.cage.environment.WLR_NO_HARDWARE_CURSORS = "1";

        services.semmeltv.coreEnvironment = {
          SEMMELTV_INTRODB = "http://127.0.0.1:9198/v4/media";
          SEMMELTV_UP_NEXT_SECONDS = toString timing.upNext;
          SEMMELTV_QUIET_SECONDS = toString timing.quiet;
          SEMMELTV_WATCHING_SECONDS = toString timing.watching;
        };

        systemd.tmpfiles.rules = [ "L+ /var/lib/semmeltv/library - - - - ${library}" ];

        environment.systemPackages = [
          pkgs.curl
          pkgs.socat
          moveMouse
        ];

        services.nginx = {
          enable = true;
          virtualHosts.phone = {
            default = true;
            listen = [
              {
                addr = "0.0.0.0";
                port = 8080;
              }
            ];
            locations."/" = proxied // {
              proxyPass = "http://127.0.0.1:8093";
            };
          };
        };
        networking.firewall.allowedTCPPorts = [ 8080 ];
      };

      testScript = ''
        import os
        import tempfile
        os.environ["PLAYWRIGHT_BROWSERS_PATH"] = "${pkgs.playwright-driver.browsers}"
        os.environ["PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS"] = "true"
        os.environ["FONTCONFIG_FILE"] = "${
          pkgs.makeFontsConf {
            fontDirectories = [
              pkgs.inter
              pkgs.dejavu_fonts
            ];
          }
        }"
        os.environ["__EGL_VENDOR_LIBRARY_DIRS"] = "${pkgs.mesa}/share/glvnd/egl_vendor.d"
        os.environ["LIBGL_ALWAYS_SOFTWARE"] = "1"
        os.environ["HOME"] = tempfile.mkdtemp()
        SEMMELTV = "${pkgs.lib.getExe semmeltv}"
        HLS_JS = "${hlsJs}"
        UP_NEXT_SECONDS = ${toString timing.upNext}
        QUIET_SECONDS = ${toString timing.quiet}
        WATCHING_SECONDS = ${toString timing.watching}
        EXPECTED_TO_FAIL = ${if builtins.elem group expectedToFail then "True" else "False"}
      ''
      + pkgs.lib.concatMapStrings builtins.readFile [
        ./common.py
        ./groups/${group}.py
        ./finish.py
      ];
    };
  checks = pkgs.lib.genAttrs groups test;
in
pkgs.lib.mapAttrs' (group: pkgs.lib.nameValuePair "semmeltv-vm-${group}") checks
// {
  semmeltv-vm = pkgs.linkFarm "semmeltv-vm" checks;
}
